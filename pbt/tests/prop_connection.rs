//! Property-Based Testing for HTTP/3 Connection (RFC 9114)
//!
//! get_stream_data のループ終了性とデータ完全性、0-RTT 再開時の WebTransport
//! SETTINGS の減少検出が選択したドラフト版に依存すること、および WebTransport の
//! 先行バッファリングがストリームの到着順や受信チャンクの分割に依存しないことを
//! 検証する。

use pbt::strategies::sample_len;
use shiguredo_http3::frame::encode_frame;
use shiguredo_http3::webtransport::{DraftVersion, Settings as WtSettings};
use shiguredo_http3::{
    ClientConnection, Error, ErrorCode, Frame, Header, Settings, SettingsPayload, VarInt,
};

/// リクエスト送信後の take_stream_data ループ用ヘルパー
///
/// take_stream_data ループを実行し、収集したデータと反復回数を返す。
/// ループはデータ交付 → FIN 交付 (`(空, fin=true)`) → None の順で終了する。
/// FIN の 1 回交付は決定的テスト (tests/test_connection.rs) で担保し、
/// この PBT はループの終了性とデータ完全性だけを検証する。
fn drain_stream_data(
    conn: &mut ClientConnection,
    stream_id: u64,
    max_iterations: usize,
) -> (Vec<u8>, usize) {
    let mut collected = Vec::new();
    let mut iterations = 0;

    while let Some((chunk, _fin)) = conn.take_stream_data(stream_id) {
        collected.extend_from_slice(&chunk);
        iterations += 1;
        if iterations >= max_iterations {
            break;
        }
    }

    (collected, iterations)
}

// =============================================================================
// get_stream_data Termination Properties
// =============================================================================

/// Property: 任意サイズのボディで get_stream_data ループが有限回で終了する
#[test]
fn prop_get_stream_data_terminates() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_CONNECTION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let body_len = sample_len(ctx, 0..=8191);
        let body = vec![0xABu8; body_len];

        let mut client = ClientConnection::with_default_settings();
        client.set_control_stream_id(2).expect("test must succeed");

        let request_headers = vec![
            Header::new(b":method", b"POST").expect("test must succeed"),
            Header::new(b":path", b"/").expect("test must succeed"),
            Header::new(b":scheme", b"https").expect("test must succeed"),
            Header::new(b":authority", b"example.com").expect("test must succeed"),
        ];

        // ボディ付きリクエストを送信 (fin=false でヘッダーのみ先に送信)
        let stream_id = client
            .send_request(&request_headers, false)
            .expect("test must succeed");
        client
            .send_body(stream_id, &body, true)
            .expect("test must succeed");

        let (_collected, iterations) = drain_stream_data(&mut client, stream_id, 10);

        assert!(
            iterations < 10,
            "get_stream_data ループが 10 回以内に終了しなかった (iterations={})",
            iterations
        );
        Ok(())
    })?;
    Ok(())
}

/// Property: ループで収集したデータ長が元のフレームデータ長と一致する (データ欠損なし)
#[test]
fn prop_get_stream_data_all_data_collected() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_CONNECTION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    // sample_len は 0/1/上限を 1/5 で選ぶ。3 境界なら空の p=1/15、
    // 256 ケースでの見逃しは (14/15)^256 ≈ 2.4e-8。
    let empty_body = std::cell::Cell::new(0usize);
    let nonempty_body = std::cell::Cell::new(0usize);
    runner.run(256, |ctx| {
        let body_len = sample_len(ctx, 0..=8191);
        let body = vec![0xCDu8; body_len];

        let mut client = ClientConnection::with_default_settings();
        client.set_control_stream_id(2).expect("test must succeed");

        let request_headers = vec![
            Header::new(b":method", b"POST").expect("test must succeed"),
            Header::new(b":path", b"/").expect("test must succeed"),
            Header::new(b":scheme", b"https").expect("test must succeed"),
            Header::new(b":authority", b"example.com").expect("test must succeed"),
        ];

        let stream_id = client
            .send_request(&request_headers, false)
            .expect("test must succeed");
        client
            .send_body(stream_id, &body, true)
            .expect("test must succeed");

        let (collected, _iterations) = drain_stream_data(&mut client, stream_id, 10);

        // 収集データにはフレームヘッダーも含まれるため、ボディ長以上であることを検証
        assert!(
            collected.len() >= body_len,
            "収集データ長 {} がボディ長 {} より小さい",
            collected.len(),
            body_len
        );
        if body_len == 0 {
            empty_body.set(empty_body.get() + 1);
        } else {
            nonempty_body.set(nonempty_body.get() + 1);
        }
        Ok(())
    })?;
    assert!(empty_body.get() > 0, "空ボディを未到達\n{runner}");
    assert!(nonempty_body.get() > 0, "非空ボディを未到達\n{runner}");
    Ok(())
}

/// Property: FIN 交付後に get_stream_data が None を返す
///
/// drain_stream_data はデータ交付 → FIN 交付 → None の順でループを抜けるため、
/// ここでの検証対象は「FIN 交付後の None」である。
/// (FIN はデータ全消費後の追加呼び出しで交付されるため、全データ消費直後の
///  get_stream_data は (空, fin=true) を返す)
#[test]
fn prop_get_stream_data_returns_none_after_consume() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_CONNECTION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let body_len = sample_len(ctx, 0..=8191);
        let body = vec![0xEFu8; body_len];

        let mut client = ClientConnection::with_default_settings();
        client.set_control_stream_id(2).expect("test must succeed");

        let request_headers = vec![
            Header::new(b":method", b"POST").expect("test must succeed"),
            Header::new(b":path", b"/").expect("test must succeed"),
            Header::new(b":scheme", b"https").expect("test must succeed"),
            Header::new(b":authority", b"example.com").expect("test must succeed"),
        ];

        let stream_id = client
            .send_request(&request_headers, false)
            .expect("test must succeed");
        client
            .send_body(stream_id, &body, true)
            .expect("test must succeed");

        // データと FIN をすべて消費
        let (_collected, _iterations) = drain_stream_data(&mut client, stream_id, 10);

        // FIN 交付後は None が返ることを検証
        let result = client.get_stream_data(stream_id);
        assert!(
            result.is_none(),
            "FIN 交付後に get_stream_data が None ではなく {:?} を返した",
            result.map(|(d, f)| (d.len(), f))
        );
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// WebTransport 0-RTT 再開時の WT SETTINGS 減少検出
// (draft-ietf-webtrans-http3-16 Section 3.2,
//  draft-ietf-webtrans-http3-17 Section 3.2)
// =============================================================================

/// 指定した版と初期上限を持つ WebTransport SETTINGS を構築する
///
/// テストで扱う版は、旧コードポイントの draft-14 / draft-15 / 16 と
/// 新コードポイントの draft-17 である。
fn wt_settings(draft: DraftVersion, uni: u64, bidi: u64, data: u64) -> WtSettings {
    let v = |x: u64| VarInt::new(x).expect("生成値が VarInt の範囲内であること");
    let settings = WtSettings::new()
        .wt_initial_max_streams_uni(v(uni))
        .wt_initial_max_streams_bidi(v(bidi))
        .wt_initial_max_data(v(data));
    match draft {
        DraftVersion::Draft17 => settings.wt_enabled_draft17(VarInt::from_static(1)),
        DraftVersion::Draft14 => settings.wt_max_sessions_draft14(VarInt::from_static(1)),
        _ => settings.wt_enabled(VarInt::from_static(1)),
    }
}

/// SETTINGS フレームを制御ストリームのバイト列にする
fn settings_control_stream(settings: &Settings) -> Vec<u8> {
    let payload = SettingsPayload::from_settings(settings);
    let mut buf = vec![0u8; 4096];
    let len = encode_frame(&mut buf, &Frame::Settings(payload))
        .expect("テスト用の SETTINGS フレームエンコードに成功すること");
    buf.truncate(len);

    // 単方向ストリームの先頭に制御ストリームタイプ (0x00) を置く
    let mut stream = vec![0x00];
    stream.extend_from_slice(&buf);
    stream
}

/// Property: 0-RTT 再開時の WT SETTINGS の減少検出は選択版が旧コードポイントの
/// ときだけ働き、draft-17 では前回値と現在値の大小関係によらず検出しない
///
/// 前回値と現在値の大小関係、および選択される版の組み合わせをランダムに生成し、
/// 次の対応を検証する:
/// - draft-14 / 15 / 16: 現在値のいずれかが前回値より小さければ H3_SETTINGS_ERROR
/// - draft-17: 大小関係によらず受理し、現在の SETTINGS の値だけを使う
#[test]
fn prop_previous_wt_settings_reduction_only_for_old_draft() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_CONNECTION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        // 両端が同じコードポイントを広告するため、この版が接続の選択結果になる
        let draft = noprop::sample_choice(
            ctx,
            &[
                DraftVersion::Draft14,
                DraftVersion::Draft15,
                DraftVersion::Draft17,
            ],
        );
        // 前回接続の版は現在の版と独立に選ぶ。前回値のコードポイントが
        // 現在の選択版の判定に影響しないことを確かめる。
        let previous_draft =
            noprop::sample_choice(ctx, &[DraftVersion::Draft15, DraftVersion::Draft17]);
        let prev_uni = noprop::sample_u64_in(ctx, 0..=1000);
        let prev_bidi = noprop::sample_u64_in(ctx, 0..=1000);
        let prev_data = noprop::sample_u64_in(ctx, 0..=100_000);

        // 現在値は、前回値との大小関係 (省略 / 同値 / 減少 / 増加) を必ず含む
        // 候補から選ぶ。省略はデフォルト値 0 として扱われる。残りの確率で
        // 前回値と無関係な値も選び、大小関係の組み合わせを広げる。
        let mut new_value = |prev: u64, max: u64| {
            let random = noprop::sample_u64_in(ctx, 0..=max);
            noprop::sample_choice(
                ctx,
                &[
                    0,
                    prev,
                    prev.saturating_sub(1),
                    prev.saturating_add(1),
                    random,
                ],
            )
        };
        let new_uni = new_value(prev_uni, 1000);
        let new_bidi = new_value(prev_bidi, 1000);
        let new_data = new_value(prev_data, 100_000);

        // ローカルは選択版に加えて draft-17 のコードポイントも広告する。
        // 版選択が退行して `None` になった場合に、旧規則の適用条件が
        // 変わってテストが落ちるようにするため。
        let local_wt = wt_settings(draft, 0, 0, 0);
        let local_wt = if draft == DraftVersion::Draft17 {
            local_wt
        } else {
            local_wt.wt_enabled_draft17(VarInt::from_static(1))
        };
        let mut client =
            ClientConnection::new(Settings::new().enable_webtransport_client(local_wt));
        client
            .set_control_stream_id(2)
            .expect("テスト用の制御ストリーム設定に成功すること");
        client.set_previous_wt_settings(wt_settings(
            previous_draft,
            prev_uni,
            prev_bidi,
            prev_data,
        ));

        let peer_wt = wt_settings(draft, new_uni, new_bidi, new_data);
        let peer_settings = Settings::new().enable_webtransport_server(peer_wt);
        let result = client.feed_stream(3, &settings_control_stream(&peer_settings), false);

        let decreased = new_uni < prev_uni || new_bidi < prev_bidi || new_data < prev_data;
        // draft-17 だけが前回値を破棄する。旧コードポイントの版は前回の版に
        // かかわらず減少検出の対象になる
        let expect_error = draft != DraftVersion::Draft17 && decreased;
        if expect_error {
            assert!(
                matches!(
                    result,
                    Err(Error::ConnectionError(ErrorCode::SettingsError))
                ),
                "旧コードポイントの版で初期上限が減少した場合は H3_SETTINGS_ERROR になること: {result:?}"
            );
        } else {
            assert!(
                result.is_ok(),
                "選択版 {draft:?} で初期上限の減少を検出しないこと: {result:?}"
            );
            assert_eq!(
                client.negotiated_wt_draft_version(),
                Some(draft),
                "現在の SETTINGS のコードポイントから選択版が決まること"
            );
        }
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// WebTransport draft-17 の先行バッファリングの順序・分割非依存性
// (draft-ietf-webtrans-http3-17 Section 4.6)
//
// ストリームの到着順や受信チャンクの分割を変えても、上限内であれば拒否判定が
// 変わらず、セッション確立後に全データが配送され、同じセッションで新しい
// ストリームを利用できることを検証する。上限超過そのものの境界値は
// tests/test_connection.rs 側で扱う。
//
// Section 4.6 が MUST として定めるのは保留するストリーム数 / データグラム数の
// 上限である。1 ストリームあたり 64 KiB の受信バイト上限は実装独自の DoS 対策
// なので、ここでは到達しない範囲のペイロードだけを送る。
// =============================================================================

mod webtransport_buffering {
    use noprop::TestCaseContext;
    use pbt::strategies::sample_len;
    use shiguredo_http3::webtransport::stream::StreamHeader;
    use shiguredo_http3::webtransport::{Datagram, DraftVersion, Settings as WtSettings};
    use shiguredo_http3::{
        ClientConnection, Event, Header, ServerConnection, Settings, WebTransportEvent,
    };

    use super::wt_settings;

    /// 本モジュールのテストで使うセッション ID
    ///
    /// クライアントは最初のリクエストとして CONNECT をクライアント開始 bidi の
    /// stream 0 に送るため、セッション ID (CONNECT ストリーム ID) は 0 になる。
    const SESSION_ID: u64 = 0;

    /// 確立後の新規クライアント開始 bidi ストリーム ID
    ///
    /// クライアント開始 bidi は 0 mod 4。上限内の先行ストリームは最大 4 本で
    /// stream 4 から 16 までを使うため、衝突しない十分大きな値にする。
    const POST_ESTABLISH_CLIENT_BIDI_STREAM_ID: u64 = 404;

    /// 確立後の新規サーバー開始 bidi ストリーム ID
    ///
    /// サーバー開始 bidi は 1 mod 4。上限内の先行ストリームは stream 1 から 13
    /// までを使い、サーバーの制御ストリーム (stream 3) とも衝突しない値にする。
    const POST_ESTABLISH_SERVER_BIDI_STREAM_ID: u64 = 405;

    /// draft-17 のフロー制御上限を広く広告する WebTransport 設定を構築する
    ///
    /// バッファ上限 (100 ストリーム) まで配送してもフロー制御違反にならないよう、
    /// ストリーム数とデータ量を十分大きくする。
    fn draft17_settings() -> WtSettings {
        wt_settings(DraftVersion::Draft17, 1000, 1000, 8 * 1024 * 1024)
    }

    /// draft-17 のサーバーとクライアントを作り、サーバー SETTINGS だけを届ける
    ///
    /// 戻り値のクライアント制御ストリームデータは、サーバーへの SETTINGS 投入を
    /// 遅らせた状態から各プロパティを開始するために呼び出し側へ返す。
    fn connection_pair() -> (ServerConnection, ClientConnection, Vec<u8>) {
        let mut server =
            ServerConnection::new(Settings::new().enable_webtransport_server(draft17_settings()));
        server
            .set_control_stream_id(3)
            .expect("テスト用の制御ストリーム設定に成功すること");
        server
            .set_webtransport_transport_verified(true, true)
            .expect("テスト用の transport parameter 検証に成功すること");
        let (server_control, _) = server
            .take_stream_data(3)
            .expect("テスト用のサーバー SETTINGS 取得に成功すること");

        let mut client =
            ClientConnection::new(Settings::new().enable_webtransport_client(draft17_settings()));
        client
            .set_control_stream_id(2)
            .expect("テスト用の制御ストリーム設定に成功すること");
        client
            .set_webtransport_transport_verified(true, true)
            .expect("テスト用の transport parameter 検証に成功すること");
        let (client_control, _) = client
            .take_stream_data(2)
            .expect("テスト用のクライアント SETTINGS 取得に成功すること");

        client
            .feed_stream(3, &server_control, false)
            .expect("クライアントのサーバー SETTINGS 受信に成功すること");
        client
            .drain_events()
            .expect("テスト用のクライアントイベント取得に成功すること");

        (server, client, client_control)
    }

    /// draft-17 の CONNECT ヘッダーを構築する
    ///
    /// 本文版のトークンである `webtransport-h3` を使う
    /// (draft-ietf-webtrans-http3-17 Section 9.1)。
    fn connect_headers() -> Vec<Header> {
        vec![
            Header::new(b":method", b"CONNECT").expect("テスト用のヘッダー作成に成功すること"),
            Header::new(b":protocol", b"webtransport-h3")
                .expect("テスト用のヘッダー作成に成功すること"),
            Header::new(b":scheme", b"https").expect("テスト用のヘッダー作成に成功すること"),
            Header::new(b":authority", b"example.com")
                .expect("テスト用のヘッダー作成に成功すること"),
            Header::new(b":path", b"/wt").expect("テスト用のヘッダー作成に成功すること"),
        ]
    }

    /// 双方向 WT ストリームの実バイト列 (signal value + session ID + ペイロード)
    ///
    /// (draft-ietf-webtrans-http3-17 Section 4.3)
    fn bidi_stream_bytes(session_id: u64, payload: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        StreamHeader::new(session_id)
            .expect("テスト用の session ID が client-initiated bidi ストリーム ID であること")
            .encode_bidirectional(&mut buf);
        buf.extend_from_slice(payload);
        buf
    }

    /// HTTP Datagram の実バイト列 (Quarter Stream ID + ペイロード)
    ///
    /// (RFC 9297 Section 2.1, draft-ietf-webtrans-http3-17 Section 4.5)
    fn datagram_bytes(session_id: u64, payload: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        Datagram::new(session_id, payload.to_vec())
            .expect("テスト用の datagram 作成に成功すること")
            .encode(&mut buf);
        buf
    }

    /// クライアントから WT CONNECT を送り、サーバーに受理させる
    ///
    /// CONNECT 投入直後のサーバーイベントも返し、呼び出し側が確立前のイベント列を
    /// 検証できるようにする。
    fn send_connect(
        server: &mut ServerConnection,
        client: &mut ClientConnection,
    ) -> (u64, Vec<Event>) {
        let stream_id = client
            .send_request(&connect_headers(), false)
            .expect("draft-17 の CONNECT 送信に成功すること");
        let (request, _) = client
            .take_stream_data(stream_id)
            .expect("テスト用の CONNECT ヘッダー取得に成功すること");
        server
            .feed_stream(stream_id, &request, false)
            .expect("draft-17 の CONNECT 受理に成功すること");
        let events = server
            .drain_events()
            .expect("テスト用のサーバーイベント取得に成功すること");
        (stream_id, events)
    }

    /// サーバーが 2xx レスポンスを生成し、クライアントにはまだ投入しない
    fn start_response(server: &mut ServerConnection, session_id: u64) -> Vec<u8> {
        let response =
            vec![Header::new(b":status", b"200").expect("テスト用のヘッダー作成に成功すること")];
        server
            .send_response(session_id, &response, false)
            .expect("テスト用の 200 応答送信に成功すること");
        server
            .take_stream_data(session_id)
            .expect("テスト用の 200 応答取得に成功すること")
            .0
    }

    /// 2xx レスポンスをクライアントに投入し、クライアントのイベントを返す
    fn feed_response(
        client: &mut ClientConnection,
        session_id: u64,
        response: &[u8],
    ) -> Vec<Event> {
        client
            .feed_stream(session_id, response, false)
            .expect("クライアントの 200 応答受信に成功すること");
        client
            .drain_events()
            .expect("テスト用のクライアントイベント取得に成功すること")
    }

    /// 0..len の添字をランダムな順序に並べ替える (Fisher-Yates)
    fn shuffled_indices(ctx: &mut TestCaseContext, len: usize) -> Vec<usize> {
        let mut order: Vec<usize> = (0..len).collect();
        for i in (1..len).rev() {
            let j = noprop::sample_usize_in(ctx, 0..=i);
            order.swap(i, j);
        }
        order
    }

    /// 実バイト列をランダムな長さのチャンクに分割して投入する
    ///
    /// `feed` にはサーバー / クライアントの `feed_stream` を渡す。チャンク境界は
    /// 先頭 varint の途中にも置かれうる。FIN は分割とは独立に空チャンクとして
    /// 最後に投入する。
    fn feed_in_random_chunks(
        ctx: &mut TestCaseContext,
        stream_id: u64,
        bytes: &[u8],
        mut feed: impl FnMut(u64, &[u8], bool),
    ) {
        let mut offset = 0;
        while offset < bytes.len() {
            let remaining = bytes.len() - offset;
            let chunk_len = noprop::sample_usize_in(ctx, 1..=remaining.min(8));
            feed(stream_id, &bytes[offset..offset + chunk_len], false);
            offset += chunk_len;
        }
        feed(stream_id, &[], true);
    }

    /// 位置依存のペイロードを生成する
    ///
    /// 同一バイトの繰り返しでは、チャンク分割や到着順の入れ替わりでバイト順が
    /// 壊れても検出できないため、添字ごとに異なるバイト列にする。
    fn payload(i: usize, len: usize) -> Vec<u8> {
        (0..len)
            .map(|j| {
                let mixed = (i as u32).wrapping_mul(0x9e37_79b9).wrapping_add(j as u32);
                (mixed ^ (mixed >> 13)) as u8
            })
            .collect()
    }

    /// 指定ストリームの双方向データを到着順に連結して返す
    fn collected_bidi_data(events: &[Event], stream_id: u64) -> Vec<u8> {
        events
            .iter()
            .filter_map(|event| match event {
                Event::WebTransport(WebTransportEvent::BidiStreamData {
                    stream_id: event_stream_id,
                    data,
                }) if *event_stream_id == stream_id => Some(data.as_slice()),
                _ => None,
            })
            .flatten()
            .copied()
            .collect()
    }

    /// Open / Data / End のいずれの配送イベントも含まれないことを検証する
    fn assert_no_stream_delivery(events: &[Event], context: &str) {
        assert!(
            !events.iter().any(|event| matches!(
                event,
                Event::WebTransport(
                    WebTransportEvent::BidiStreamOpen { .. }
                        | WebTransportEvent::BidiStreamData { .. }
                        | WebTransportEvent::BidiStreamEnd { .. }
                        | WebTransportEvent::UniStreamOpen { .. }
                        | WebTransportEvent::UniStreamData { .. }
                        | WebTransportEvent::UniStreamEnd { .. }
                )
            )),
            "{context}: セッション確立前に Open / Data / End を発火しないこと: {events:?}"
        );
    }

    /// 拒否イベントが発火していないことを検証する
    fn assert_no_buffered_stream_rejected(events: &[Event], context: &str) {
        assert!(
            !events.iter().any(|event| matches!(
                event,
                Event::WebTransport(WebTransportEvent::BufferedStreamRejected { .. })
            )),
            "{context}: 上限内の入力では拒否イベントが発火しないこと: {events:?}"
        );
    }

    /// セッション終了イベントが発火していないことを検証する
    fn assert_no_session_closed(events: &[Event], context: &str) {
        assert!(
            !events.iter().any(|event| matches!(
                event,
                Event::WebTransport(WebTransportEvent::SessionClosed { .. })
            )),
            "{context}: セッション終了イベントが発火しないこと: {events:?}"
        );
    }

    /// 指定セッションの SessionEstablished がイベント列に含まれることを検証する
    fn assert_session_established(events: &[Event], session_id: u64) {
        assert!(
            events.iter().any(|event| matches!(
                event,
                Event::WebTransport(WebTransportEvent::SessionEstablished {
                    session_id: event_session_id,
                    ..
                }) if *event_session_id == session_id
            )),
            "session {session_id} の SessionEstablished が発火すること: {events:?}"
        );
    }

    /// 指定ストリームの BidiStreamOpen が指定セッションに紐付くことを検証する
    fn assert_bidi_stream_open(events: &[Event], stream_id: u64, session_id: u64) {
        assert!(
            events.iter().any(|event| matches!(
                event,
                Event::WebTransport(WebTransportEvent::BidiStreamOpen {
                    stream_id: event_stream_id,
                    session_id: event_session_id,
                }) if *event_stream_id == stream_id && *event_session_id == session_id
            )),
            "stream {stream_id} の BidiStreamOpen が session {session_id} に紐付くこと: {events:?}"
        );
    }

    /// Property: SETTINGS 受信前の先行 bidi ストリームは、到着順と分割によらず
    /// 拒否されず、セッション確立後に全データが配送される
    #[test]
    fn prop_pre_negotiation_bidi_buffering_is_order_and_split_invariant() -> noprop::TestResult {
        let seed = noprop::seed_from_env_or_time("PROP_CONNECTION_SEED")?;
        let mut runner = noprop::Runner::new(seed);
        runner.run(256, |ctx| {
            let (mut server, mut client, client_control) = connection_pair();

            // ストリーム数と各ストリームのペイロード長を生成する。
            // 位置依存のバイト列にすることで、分割や順序の入れ替わりで
            // バイト順が壊れた場合も検出できる。
            let stream_count = noprop::sample_usize_in(ctx, 1..=4);
            let mut payloads = Vec::new();
            for i in 0..stream_count {
                let payload_len = sample_len(ctx, 0..=512);
                payloads.push(payload(i, payload_len));
            }

            // 到着順をランダム化し、各ストリームの受信バイト列をランダムに分割して
            // SETTINGS 受信前に投入する。FIN もここで投入する。
            let order = shuffled_indices(ctx, stream_count);
            for &i in &order {
                let stream_id = (i as u64 + 1) * 4;
                let bytes = bidi_stream_bytes(SESSION_ID, &payloads[i]);
                feed_in_random_chunks(ctx, stream_id, &bytes, |stream_id, data, fin| {
                    server
                        .feed_stream(stream_id, data, fin)
                        .expect("分割した bidi ストリームの投入に成功すること");
                });
            }

            // SETTINGS 受信前はイベントが発火しない
            let events = server
                .drain_events()
                .expect("テスト用のサーバーイベント取得に成功すること");
            assert!(
                events.is_empty(),
                "SETTINGS 受信前はイベントが発火しないこと: {events:?}"
            );

            // 遅延させたクライアント SETTINGS を投入する
            server
                .feed_stream(2, &client_control, false)
                .expect("サーバーのクライアント SETTINGS 受信に成功すること");
            let events = server
                .drain_events()
                .expect("テスト用のサーバーイベント取得に成功すること");
            assert!(
                events
                    .iter()
                    .any(|event| matches!(event, Event::SettingsReceived { .. })),
                "クライアント SETTINGS 受信イベントが発火すること: {events:?}"
            );
            assert_no_buffered_stream_rejected(&events, "SETTINGS 受信時");
            assert_no_session_closed(&events, "SETTINGS 受信時");

            // CONNECT を送り、2xx を返してセッションを確立する
            let (session_id, connect_events) = send_connect(&mut server, &mut client);
            assert_eq!(session_id, SESSION_ID, "CONNECT は stream 0 で送られること");
            assert_no_stream_delivery(&connect_events, "CONNECT 受理時");

            let response = start_response(&mut server, session_id);
            let client_events = feed_response(&mut client, session_id, &response);
            assert_session_established(&client_events, session_id);

            let events = server
                .drain_events()
                .expect("テスト用のサーバーイベント取得に成功すること");
            assert_session_established(&events, session_id);
            assert_no_buffered_stream_rejected(&events, "セッション確立時");
            assert_no_session_closed(&events, "セッション確立時");

            // 到着順によらず、全ストリームの Open / Data / End が配送される
            let open_count = events
                .iter()
                .filter(|event| {
                    matches!(
                        event,
                        Event::WebTransport(WebTransportEvent::BidiStreamOpen {
                            session_id: event_session_id,
                            ..
                        }) if *event_session_id == session_id
                    )
                })
                .count();
            assert_eq!(
                open_count, stream_count,
                "保持した全ストリームが 1 回ずつ Open されること: {events:?}"
            );
            for (i, payload) in payloads.iter().enumerate() {
                let stream_id = (i as u64 + 1) * 4;
                let end_count = events
                    .iter()
                    .filter(|event| {
                        matches!(
                            event,
                            Event::WebTransport(WebTransportEvent::BidiStreamEnd {
                                stream_id: event_stream_id,
                            }) if *event_stream_id == stream_id
                        )
                    })
                    .count();
                assert_eq!(
                    end_count, 1,
                    "stream {stream_id} の End が 1 回だけ発火すること: {events:?}"
                );
                assert_eq!(
                    collected_bidi_data(&events, stream_id),
                    *payload,
                    "stream {stream_id} のデータが到着順や分割によらず一致すること"
                );
            }

            // 確立後は同じセッションで新しいストリームを利用できる
            let new_stream_id = POST_ESTABLISH_CLIENT_BIDI_STREAM_ID;
            server
                .feed_stream(
                    new_stream_id,
                    &bidi_stream_bytes(session_id, b"after"),
                    true,
                )
                .expect("確立後の新規 bidi ストリームが受理されること");
            let events = server
                .drain_events()
                .expect("テスト用のサーバーイベント取得に成功すること");
            assert_bidi_stream_open(&events, new_stream_id, session_id);
            assert_eq!(
                collected_bidi_data(&events, new_stream_id),
                b"after",
                "拒否が発生していないセッションで新しいストリームを受信できること"
            );
            assert_no_session_closed(&events, "確立後のストリーム受信時");
            Ok(())
        })?;
        Ok(())
    }

    /// Property: 2xx 受信前のサーバー開始 bidi ストリームとデータグラムは、
    /// 到着順と分割によらず保留され、セッション確立後に全データが配送される
    #[test]
    fn prop_pending_session_buffering_is_order_and_split_invariant() -> noprop::TestResult {
        let seed = noprop::seed_from_env_or_time("PROP_CONNECTION_SEED")?;
        let mut runner = noprop::Runner::new(seed);
        runner.run(256, |ctx| {
            let (mut server, mut client, client_control) = connection_pair();
            server
                .feed_stream(2, &client_control, false)
                .expect("サーバーのクライアント SETTINGS 受信に成功すること");
            let events = server
                .drain_events()
                .expect("テスト用のサーバーイベント取得に成功すること");
            assert_no_session_closed(&events, "クライアント SETTINGS 受信時");

            // クライアントは CONNECT を送って Pending セッションを作る。
            // サーバーは send_response で確立するが、2xx バイトはまだクライアントに
            // 投入しない。
            let (session_id, connect_events) = send_connect(&mut server, &mut client);
            assert_eq!(session_id, SESSION_ID, "CONNECT は stream 0 で送られること");
            assert_no_stream_delivery(&connect_events, "CONNECT 受理時");
            let response = start_response(&mut server, session_id);

            // サーバー開始 bidi ストリームの到着順と分割をランダム化する。
            // ペイロードは位置依存のバイト列にして、バイト順の破壊も検出できるようにする。
            let stream_count = noprop::sample_usize_in(ctx, 1..=4);
            let mut payloads = Vec::new();
            for i in 0..stream_count {
                let payload_len = sample_len(ctx, 0..=512);
                payloads.push(payload(i, payload_len));
            }
            let order = shuffled_indices(ctx, stream_count);
            for &i in &order {
                // サーバー開始 bidi は 1 mod 4。サーバー制御ストリーム (3) と衝突しない
                let stream_id = 1 + (i as u64) * 4;
                let bytes = bidi_stream_bytes(session_id, &payloads[i]);
                feed_in_random_chunks(ctx, stream_id, &bytes, |stream_id, data, fin| {
                    client
                        .feed_stream(stream_id, data, fin)
                        .expect("分割した bidi ストリームの投入に成功すること");
                });
            }

            // データグラムも 2xx 前に送る。到着順をランダム化し、配送順が
            // 仕様で規定されていないことを前提に検証する。
            let datagram_count = noprop::sample_usize_in(ctx, 1..=4);
            let mut datagram_payloads = Vec::new();
            for i in 0..datagram_count {
                let payload_len = sample_len(ctx, 0..=64);
                datagram_payloads.push(payload(i, payload_len));
            }
            let datagram_order = shuffled_indices(ctx, datagram_count);
            for &i in &datagram_order {
                client
                    .feed_datagram(&datagram_bytes(session_id, &datagram_payloads[i]))
                    .expect("2xx 前の datagram が保留されること");
            }

            let events = client
                .drain_events()
                .expect("テスト用のクライアントイベント取得に成功すること");
            assert!(
                events.is_empty(),
                "2xx 受信前はイベントが発火しないこと: {events:?}"
            );

            // 2xx を投入してセッションを確立する
            let events = feed_response(&mut client, session_id, &response);
            assert_session_established(&events, session_id);
            assert_no_buffered_stream_rejected(&events, "セッション確立時");
            assert_no_session_closed(&events, "セッション確立時");

            let open_count = events
                .iter()
                .filter(|event| {
                    matches!(
                        event,
                        Event::WebTransport(WebTransportEvent::BidiStreamOpen {
                            session_id: event_session_id,
                            ..
                        }) if *event_session_id == session_id
                    )
                })
                .count();
            assert_eq!(
                open_count, stream_count,
                "保持した全ストリームが 1 回ずつ Open されること: {events:?}"
            );
            for (i, payload) in payloads.iter().enumerate() {
                let stream_id = 1 + (i as u64) * 4;
                let end_count = events
                    .iter()
                    .filter(|event| {
                        matches!(
                            event,
                            Event::WebTransport(WebTransportEvent::BidiStreamEnd {
                                stream_id: event_stream_id,
                            }) if *event_stream_id == stream_id
                        )
                    })
                    .count();
                assert_eq!(
                    end_count, 1,
                    "stream {stream_id} の End が 1 回だけ発火すること: {events:?}"
                );
                assert_eq!(
                    collected_bidi_data(&events, stream_id),
                    *payload,
                    "stream {stream_id} のデータが到着順や分割によらず一致すること"
                );
            }

            // データグラムの配送順は仕様で規定されていないため、順序ではなく
            // 全 payload が過不足なく配送されることだけを検証する。
            let mut delivered_datagrams: Vec<Vec<u8>> = events
                .iter()
                .filter_map(|event| match event {
                    Event::WebTransport(WebTransportEvent::Datagram { payload, .. }) => {
                        Some(payload.clone())
                    }
                    _ => None,
                })
                .collect();
            let mut expected_datagrams = datagram_payloads.clone();
            delivered_datagrams.sort();
            expected_datagrams.sort();
            assert_eq!(
                delivered_datagrams, expected_datagrams,
                "到着順によらず、送信した全 datagram が過不足なく配送されること: {events:?}"
            );

            // 確立後は同じセッションで新しいストリームとデータグラムを利用できる
            let new_stream_id = POST_ESTABLISH_SERVER_BIDI_STREAM_ID;
            client
                .feed_stream(
                    new_stream_id,
                    &bidi_stream_bytes(session_id, b"after"),
                    true,
                )
                .expect("確立後のサーバー開始 bidi ストリームが受理されること");
            client
                .feed_datagram(&datagram_bytes(session_id, b"after-datagram"))
                .expect("確立後の datagram が受理されること");
            let events = client
                .drain_events()
                .expect("テスト用のクライアントイベント取得に成功すること");
            assert_bidi_stream_open(&events, new_stream_id, session_id);
            assert_eq!(
                collected_bidi_data(&events, new_stream_id),
                b"after",
                "確立後の新しいストリームを受信できること"
            );
            assert!(
                events.iter().any(|event| matches!(
                    event,
                    Event::WebTransport(WebTransportEvent::Datagram { payload, .. })
                        if payload == b"after-datagram"
                )),
                "確立後の新しい datagram が配送されること: {events:?}"
            );
            assert_no_session_closed(&events, "確立後の受信時");
            Ok(())
        })?;
        Ok(())
    }
}
