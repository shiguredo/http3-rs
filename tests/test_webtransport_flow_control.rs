//! 統合テスト: WebTransport フロー制御の実接続経路
//!
//! 実際の `ClientConnection` / `ServerConnection` を使い、SETTINGS 交換から
//! WebTransport セッション確立、フロー制御カプセルの送受信までの流れを検証する。
//!
//! 以前は `webtransport::Session` (本番経路から切り離された並行実装) を対象に
//! していたが、送信側フロー制御を `Connection` 層へ統合したため、
//! 本番で動く経路そのものを検証する形へ移行した。
//! (draft-ietf-webtrans-http3-16 Section 5.6)

use shiguredo_http3::qpack::Encoder;
use shiguredo_http3::webtransport::{Capsule, Settings as WtSettings};
use shiguredo_http3::{ClientConnection, Header, ServerConnection, Settings, VarInt};

/// テスト用の VarInt を作成する
fn vi(value: u64) -> VarInt {
    VarInt::new(value).expect("テスト用の値は VarInt 範囲内")
}

/// Safari 形クライアントの初期上限 (docs/SAFARI_WT.md)
const PEER_INITIAL_MAX_STREAMS: u64 = 100;
const PEER_INITIAL_MAX_DATA: u64 = 8 * 1024 * 1024;

/// サーバー側 SETTINGS (フロー制御を有効化する)
fn server_wt_settings() -> Settings {
    let wt = WtSettings::new()
        .wt_enabled(vi(1))
        .webtransport_max_sessions_draft07(vi(100))
        .wt_initial_max_streams_uni(vi(100))
        .wt_initial_max_streams_bidi(vi(100))
        .wt_initial_max_data(vi(8 * 1024 * 1024));
    Settings::new().enable_webtransport_server(wt)
}

/// QPACK エンコードされた HEADERS フレームを構築する
fn build_headers_frame(headers: &[Header]) -> Vec<u8> {
    let mut encoder = Encoder::new();
    let mut qpack_buf = vec![0u8; 4096];
    let qpack_len = encoder
        .encode(&mut qpack_buf, headers, 0)
        .expect("テスト用ヘッダーの QPACK エンコードは成功する");
    qpack_buf.truncate(qpack_len);

    let mut frame = Vec::new();
    shiguredo_http3::varint::encode_into_vec(&mut frame, VarInt::from_static(0x01));
    shiguredo_http3::varint::encode_into_vec(
        &mut frame,
        VarInt::new(qpack_len as u64).expect("QPACK 長は VarInt 範囲内"),
    );
    frame.extend_from_slice(&qpack_buf);
    frame
}

/// WebTransport CONNECT のヘッダー
fn wt_connect_headers() -> Vec<Header> {
    vec![
        Header::new(b":method", b"CONNECT").expect("テスト用ヘッダーは有効"),
        Header::new(b":protocol", b"webtransport").expect("テスト用ヘッダーは有効"),
        Header::new(b":scheme", b"https").expect("テスト用ヘッダーは有効"),
        Header::new(b":authority", b"example.com").expect("テスト用ヘッダーは有効"),
        Header::new(b":path", b"/wt").expect("テスト用ヘッダーは有効"),
    ]
}

/// サーバーにクライアント SETTINGS と WT CONNECT を流し込み、200 OK を返す
///
/// `client_settings` でフロー制御の有無を切り替える。
fn establish(server: &mut ServerConnection, client_settings: Settings) -> u64 {
    let mut client = ClientConnection::new(client_settings);
    client
        .set_control_stream_id(2)
        .expect("制御ストリーム ID の設定は成功する");
    let (client_ctrl, _) = client
        .take_stream_data(2)
        .expect("制御ストリームの初期データが取れる");
    server
        .feed_stream(2, &client_ctrl, false)
        .expect("クライアント SETTINGS の受理は成功する");
    let _ = server.drain_events().expect("イベントの取り出しは成功する");

    let stream_id = 0u64;
    let frame = build_headers_frame(&wt_connect_headers());
    server
        .feed_stream(stream_id, &frame, false)
        .expect("WT CONNECT の受理は成功する");
    let _ = server.drain_events().expect("イベントの取り出しは成功する");

    let response = vec![Header::new(b":status", b"200").expect("テスト用ヘッダーは有効")];
    server
        .send_response(stream_id, &response, false)
        .expect("200 応答の送信は成功する");
    let _ = server.drain_events().expect("イベントの取り出しは成功する");
    stream_id
}

/// Safari 形クライアントでセッションを確立済みのサーバーを返す
///
/// Safari 26.4 は draft-07 の SETTINGS ID と `WT_INITIAL_MAX_*` を併送する
/// ハイブリッド実装で、カプセルベースのフロー制御が有効になる
/// (docs/SAFARI_WT.md)。
fn established_server() -> (ServerConnection, u64) {
    let client_wt = WtSettings::new()
        .webtransport_max_sessions_draft07(vi(100))
        .wt_initial_max_streams_uni(vi(PEER_INITIAL_MAX_STREAMS))
        .wt_initial_max_streams_bidi(vi(PEER_INITIAL_MAX_STREAMS))
        .wt_initial_max_data(vi(PEER_INITIAL_MAX_DATA));
    let client_settings = Settings::new().enable_webtransport_client(client_wt);

    let mut server = ServerConnection::new(server_wt_settings());
    server
        .set_control_stream_id(3)
        .expect("制御ストリーム ID の設定は成功する");
    server
        .set_webtransport_transport_verified(true, true)
        .expect("transport parameter の注入は成功する");
    let stream_id = establish(&mut server, client_settings);
    (server, stream_id)
}

/// サーバーへフロー制御カプセルを 1 つ供給する
fn feed_capsule(server: &mut ServerConnection, stream_id: u64, capsule: &Capsule) {
    let mut data = Vec::new();
    capsule
        .encode_as_data_frame(&mut data)
        .expect("テスト用カプセルのエンコードは成功する");
    server
        .feed_stream(stream_id, &data, false)
        .expect("カプセルの受理は成功する");
    let _ = server.drain_events().expect("イベントの取り出しは成功する");
}

/// Safari 形クライアントでセッション確立直後に初期クレジットが取り出せること
///
/// これが送出されないと、ピアは初期クレジット 0 のままストリームを開けない
/// (Safari 26.4 が繋がらない直接原因)。
#[test]
fn test_initial_flow_control_capsules_for_safari_shape_client() {
    let (mut server, stream_id) = established_server();
    assert!(
        server.wt_session_flow_control_enabled(stream_id),
        "Safari 形クライアントでフロー制御が有効にならない"
    );

    let capsules = server.take_wt_flow_control_capsules(stream_id);
    assert_eq!(capsules.len(), 3, "初期カプセルが 3 件でない: {capsules:?}");
    assert!(matches!(
        capsules[0],
        Capsule::MaxStreams {
            bidirectional: true,
            maximum: 100
        }
    ));
    assert!(matches!(
        capsules[1],
        Capsule::MaxStreams {
            bidirectional: false,
            maximum: 100
        }
    ));
    assert!(matches!(
        capsules[2],
        Capsule::MaxData { maximum } if maximum == 8 * 1024 * 1024
    ));

    // 取り出しは冪等
    assert!(server.take_wt_flow_control_capsules(stream_id).is_empty());
}

/// 受信した WT_MAX_DATA が送信上限に反映されること
#[test]
fn test_inbound_max_data_limits_outbound_send() {
    let (mut server, stream_id) = established_server();
    let _ = server.take_wt_flow_control_capsules(stream_id);

    // ピアの初期上限 (8 MiB) より大きい値を広告する必要がある
    // (増加しない値はエラー。draft-ietf-webtrans-http3-16 Section 5.6.4)
    let limit = 10_000_000u64;
    feed_capsule(&mut server, stream_id, &Capsule::MaxData { maximum: limit });

    assert_eq!(server.wt_remote_max_data(stream_id), Some(limit));
    assert!(server.can_send_wt_data(stream_id, limit));
    assert!(!server.can_send_wt_data(stream_id, limit + 1));

    // limit - 400 バイト送ると残り 400 バイト
    assert!(server.wt_data_sent(stream_id, limit - 400));
    assert!(server.can_send_wt_data(stream_id, 400));
    assert!(!server.can_send_wt_data(stream_id, 401));

    // 上限を超える送信は WT_DATA_BLOCKED を生成する
    assert!(!server.wt_data_sent(stream_id, 401));
    let capsules = server.take_wt_flow_control_capsules(stream_id);
    assert!(
        capsules.iter().any(|c| matches!(
            c,
            Capsule::DataBlocked { maximum } if *maximum == limit
        )),
        "WT_DATA_BLOCKED が生成されていない: {capsules:?}"
    );
}

/// 受信した WT_MAX_STREAMS が開設上限に反映されること
#[test]
fn test_inbound_max_streams_limits_outbound_open() {
    let (mut server, stream_id) = established_server();
    let _ = server.take_wt_flow_control_capsules(stream_id);

    // ピアの初期上限は 100 なので、101 に増やす
    let limit = 101u64;
    feed_capsule(
        &mut server,
        stream_id,
        &Capsule::MaxStreams {
            bidirectional: true,
            maximum: limit,
        },
    );

    for opened in 0..limit {
        assert!(
            server.wt_stream_opened(stream_id, true),
            "上限内の開設が拒否された: opened={opened}"
        );
    }
    assert!(!server.wt_stream_opened(stream_id, true));
    let capsules = server.take_wt_flow_control_capsules(stream_id);
    assert!(
        capsules.iter().any(|c| matches!(
            c,
            Capsule::StreamsBlocked {
                bidirectional: true,
                maximum,
            } if *maximum == limit
        )),
        "WT_STREAMS_BLOCKED が生成されていない: {capsules:?}"
    );
}

/// ピアの WT 単方向ストリーム受信がデータ FC に計上されること
///
/// 受信計上が行われないと、アプリの消費通知 (`wt_data_consumed`) が
/// ウィンドウ更新のしきい値判定に反映されない
/// (draft-ietf-webtrans-http3-16 Section 5.4)。
#[test]
fn test_peer_uni_stream_data_is_accounted() {
    let (mut server, stream_id) = established_server();
    let _ = server.take_wt_flow_control_capsules(stream_id);

    // ピア開始 uni ストリーム (0x54) + session_id=0 + body
    // (draft-ietf-webtrans-http3-16 Section 4.2)
    let mut data = vec![0x40, 0x54, 0x00];
    data.extend_from_slice(&[0xAAu8; 32]);
    server
        .feed_stream(6, &data, false)
        .expect("WT 単方向ストリームの受理は成功する");
    let _ = server.drain_events().expect("イベントの取り出しは成功する");

    // 32 バイト受信したので、消費するとウィンドウが 32 バイト増える。
    // 初期ウィンドウ (8 MiB) の残りがしきい値を下回るまで受信していないため、
    // この時点では更新カプセルは生成されない (過剰な更新を送らない)。
    server.wt_data_consumed(stream_id, 32);
    assert!(
        server.take_wt_flow_control_capsules(stream_id).is_empty(),
        "しきい値に達していないのに WT_MAX_DATA が生成されている"
    );

    // FIN で閉じても受信計上やエラーにならない
    server
        .feed_stream(6, &[], true)
        .expect("WT 単方向ストリームの FIN 受理は成功する");
    let events = server.drain_events().expect("イベントの取り出しは成功する");
    assert!(
        events.iter().any(|e| matches!(
            e,
            shiguredo_http3::Event::WebTransport(
                shiguredo_http3::WebTransportEvent::UniStreamEnd { stream_id: id }
            ) if *id == 6
        )),
        "UniStreamEnd が発火していない: {events:?}"
    );
}

/// フロー制御が無効な draft-07 クライアントではカプセルを生成しないこと
#[test]
fn test_no_capsules_when_flow_control_disabled() {
    // WT_INITIAL_MAX_* を送らない素の draft-07 クライアント
    let client_wt = WtSettings::new().webtransport_max_sessions_draft07(vi(1));
    let client_settings = Settings::new().enable_webtransport_client(client_wt);

    let mut server = ServerConnection::new(server_wt_settings());
    server
        .set_control_stream_id(3)
        .expect("制御ストリーム ID の設定は成功する");
    server
        .set_webtransport_transport_verified(true, false)
        .expect("transport parameter の注入は成功する");
    let stream_id = establish(&mut server, client_settings);

    assert!(
        !server.wt_session_flow_control_enabled(stream_id),
        "フロー制御が無効なクライアントで有効になっている"
    );
    assert!(
        server.take_wt_flow_control_capsules(stream_id).is_empty(),
        "フロー制御が無効なのにカプセルが生成されている"
    );
    // 上限判定を行わないため常に送信可能
    assert!(server.can_send_wt_data(stream_id, u64::MAX / 2));
    assert!(server.can_open_wt_bidi_stream(stream_id));
}
