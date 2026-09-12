//! WebTransport フロー制御の PBT 用フィクスチャ
//!
//! 実接続 (`ServerConnection`) 上に WebTransport セッションを確立し、
//! フロー制御カプセルを供給するためのヘルパーを提供する。
//! 公開 API だけで組むことで、本番経路そのものを検証する。
//! (draft-ietf-webtrans-http3-16 Section 5.6)

use shiguredo_http3::qpack::Encoder;
use shiguredo_http3::webtransport::{Capsule, Settings as WtSettings};
use shiguredo_http3::{Header, ServerConnection, Settings, VarInt};

/// Safari 26.4 形クライアントの初期上限
///
/// Safari は draft-07 の SETTINGS ID と `WT_INITIAL_MAX_*` を併送する
/// ハイブリッド実装である (docs/SAFARI_WT.md)。
pub const PEER_INITIAL_MAX_STREAMS: u64 = 100;
pub const PEER_INITIAL_MAX_DATA: u64 = 8 * 1024 * 1024;

/// テスト用の VarInt を作成する
pub fn vi(value: u64) -> VarInt {
    VarInt::new(value).expect("テスト用の値は VarInt 範囲内")
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

/// Safari 形クライアントから見たサーバー SETTINGS (送信側上限の広告元)
fn client_settings() -> Settings {
    let wt = WtSettings::new()
        .webtransport_max_sessions_draft07(vi(100))
        .wt_initial_max_streams_uni(vi(PEER_INITIAL_MAX_STREAMS))
        .wt_initial_max_streams_bidi(vi(PEER_INITIAL_MAX_STREAMS))
        .wt_initial_max_data(vi(PEER_INITIAL_MAX_DATA));
    Settings::new().enable_webtransport_client(wt)
}

/// サーバー側 SETTINGS (フロー制御を有効化する)
fn server_settings() -> Settings {
    let wt = WtSettings::new()
        .wt_enabled(vi(1))
        .webtransport_max_sessions_draft07(vi(100))
        .wt_initial_max_streams_uni(vi(100))
        .wt_initial_max_streams_bidi(vi(100))
        .wt_initial_max_data(vi(8 * 1024 * 1024));
    Settings::new().enable_webtransport_server(wt)
}

/// WebTransport セッションを確立済みのサーバーと CONNECT ストリーム ID を返す
///
/// 制御ストリーム (クライアント stream 2) を流し込み、CONNECT (stream 0) を
/// feed して 200 OK を返すところまで進める。
pub fn established_server() -> (ServerConnection, u64) {
    let mut client = shiguredo_http3::ClientConnection::new(client_settings());
    client
        .set_control_stream_id(2)
        .expect("制御ストリーム ID の設定は成功する");
    let (client_ctrl, _) = client
        .take_stream_data(2)
        .expect("制御ストリームの初期データが取れる");

    let mut server = ServerConnection::new(server_settings());
    server
        .set_control_stream_id(3)
        .expect("制御ストリーム ID の設定は成功する");
    server
        .set_webtransport_transport_verified(true, true)
        .expect("transport parameter の注入は成功する");
    server
        .feed_stream(2, &client_ctrl, false)
        .expect("クライアント SETTINGS の受理は成功する");
    let _ = server.drain_events().expect("イベントの取り出しは成功する");

    let stream_id = 0u64;
    let headers = vec![
        Header::new(b":method", b"CONNECT").expect("テスト用ヘッダーは有効"),
        Header::new(b":protocol", b"webtransport").expect("テスト用ヘッダーは有効"),
        Header::new(b":scheme", b"https").expect("テスト用ヘッダーは有効"),
        Header::new(b":authority", b"example.com").expect("テスト用ヘッダーは有効"),
        Header::new(b":path", b"/wt").expect("テスト用ヘッダーは有効"),
    ];
    let frame = build_headers_frame(&headers);
    server
        .feed_stream(stream_id, &frame, false)
        .expect("WT CONNECT の受理は成功する");
    let _ = server.drain_events().expect("イベントの取り出しは成功する");

    let response = vec![Header::new(b":status", b"200").expect("テスト用ヘッダーは有効")];
    server
        .send_response(stream_id, &response, false)
        .expect("200 応答の送信は成功する");
    let _ = server.drain_events().expect("イベントの取り出しは成功する");

    (server, stream_id)
}

/// サーバーへフロー制御カプセルを 1 つ供給する
///
/// CONNECT ストリーム上のカプセルは HTTP/3 DATA フレームとして届く
/// (RFC 9297 Section 3.1)。
pub fn feed_capsule(server: &mut ServerConnection, stream_id: u64, capsule: &Capsule) {
    let mut data = Vec::new();
    capsule
        .encode_as_data_frame(&mut data)
        .expect("テスト用カプセルのエンコードは成功する");
    server
        .feed_stream(stream_id, &data, false)
        .expect("カプセルの受理は成功する");
    let _ = server.drain_events().expect("イベントの取り出しは成功する");
}
