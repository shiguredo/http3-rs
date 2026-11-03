//! WebTransport CONNECT の draft バージョン別バリデーションテスト
//!
//! サーバーが各 draft (02/07/14/15/16/17) クライアントからの CONNECT を正しく受理/拒否するか検証。
//!
//! 特に SETTINGS_ENABLE_CONNECT_PROTOCOL (0x08) の要否がドラフトバージョンによって異なる:
//! - draft-02: 不要 (SETTINGS_ENABLE_WEBTRANSPORT_DRAFT02 が暗示)
//! - draft-07: 不要 (クライアントは SETTINGS_WEBTRANSPORT_MAX_SESSIONS のみ MUST,
//!   draft-ietf-webtrans-http3-07 Section 3.2)
//! - draft-14: 必須 (RFC 9220)
//! - draft-15 / 16 / 17: 必須 (RFC 9220)
//!
//! draft-15 と draft-16 は SETTINGS_WT_ENABLED のコードポイントを共有し、
//! draft-17 は別コードポイント (0x20b8933b) を使う
//! (draft-ietf-webtrans-http3-17 Section 9.2)。
//! SETTINGS 単体のコードポイント対応は tests/test_webtransport_settings.rs で扱う。

use shiguredo_http3::qpack::Encoder;
use shiguredo_http3::webtransport::DraftVersion;
use shiguredo_http3::{
    Error, ErrorCode, Event, Header, ServerConnection, Settings, VarInt, WebTransportEvent,
    WtSetupError, webtransport,
};

fn vi(value: u64) -> VarInt {
    VarInt::new(value).expect("test must succeed")
}

// =========================================================================
// ヘルパー関数
// =========================================================================

/// QPACK エンコードされた HEADERS フレームを手動構築する
fn build_headers_frame(headers: &[Header]) -> Vec<u8> {
    let mut encoder = Encoder::new();
    let mut qpack_buf = vec![0u8; 4096];
    let qpack_len = encoder
        .encode(&mut qpack_buf, headers, 0)
        .expect("test must succeed");
    qpack_buf.truncate(qpack_len);

    let mut frame = Vec::new();
    // HEADERS フレームタイプ: 0x01
    shiguredo_http3::varint::encode_into_vec(
        &mut frame,
        shiguredo_http3::VarInt::from_static(0x01),
    );
    // ペイロード長
    shiguredo_http3::varint::encode_into_vec(
        &mut frame,
        shiguredo_http3::VarInt::new(qpack_len as u64).expect("qpack length fits in VarInt"),
    );
    frame.extend_from_slice(&qpack_buf);
    frame
}

/// draft-02 クライアントの SETTINGS を持つ Connection を作成し、
/// 制御ストリームデータを返す
fn build_draft02_client_ctrl() -> Vec<u8> {
    let wt = webtransport::Settings::new().enable_webtransport_draft02(true);
    // draft-02 では enable_connect_protocol は不要
    let settings = Settings {
        enable_connect_protocol: None,
        ..Settings::new()
            .h3_datagram(true)
            .enable_webtransport_server(wt)
    };
    let mut client = shiguredo_http3::ClientConnection::new(settings);
    client.set_control_stream_id(2).expect("test must succeed");
    let (ctrl, _) = client.take_stream_data(2).expect("test must succeed");
    ctrl
}

/// draft-07 クライアントの SETTINGS を持つ Connection を作成し、
/// 制御ストリームデータを返す
/// Safari と同じパターン: ENABLE_CONNECT_PROTOCOL なし
fn build_draft07_client_ctrl() -> Vec<u8> {
    let wt = webtransport::Settings::new().webtransport_max_sessions_draft07(vi(1));
    let settings = Settings {
        enable_connect_protocol: None,
        ..Settings::new()
            .h3_datagram(true)
            .enable_webtransport_server(wt)
    };
    let mut client = shiguredo_http3::ClientConnection::new(settings);
    client.set_control_stream_id(2).expect("test must succeed");
    let (ctrl, _) = client.take_stream_data(2).expect("test must succeed");
    ctrl
}

/// draft-14 クライアントの SETTINGS を持つ Connection を作成し、
/// 制御ストリームデータを返す
/// ENABLE_CONNECT_PROTOCOL あり
fn build_draft14_client_ctrl_with_ecp() -> Vec<u8> {
    let wt = webtransport::Settings::new()
        .wt_max_sessions_draft14(vi(1))
        .wt_initial_max_streams_uni(vi(100))
        .wt_initial_max_streams_bidi(vi(100))
        .wt_initial_max_data(vi(8 * 1024 * 1024));
    let settings = Settings::new()
        .h3_datagram(true)
        .enable_connect_protocol(true)
        .enable_webtransport_server(wt);
    let mut client = shiguredo_http3::ClientConnection::new(settings);
    client.set_control_stream_id(2).expect("test must succeed");
    let (ctrl, _) = client.take_stream_data(2).expect("test must succeed");
    ctrl
}

/// draft-14 クライアントの SETTINGS (ENABLE_CONNECT_PROTOCOL なし)
fn build_draft14_client_ctrl_without_ecp() -> Vec<u8> {
    let wt = webtransport::Settings::new()
        .wt_max_sessions_draft14(vi(1))
        .wt_initial_max_streams_uni(vi(100))
        .wt_initial_max_streams_bidi(vi(100))
        .wt_initial_max_data(vi(8 * 1024 * 1024));
    let settings = Settings {
        enable_connect_protocol: None,
        ..Settings::new()
            .h3_datagram(true)
            .enable_webtransport_server(wt)
    };
    let mut client = shiguredo_http3::ClientConnection::new(settings);
    client.set_control_stream_id(2).expect("test must succeed");
    let (ctrl, _) = client.take_stream_data(2).expect("test must succeed");
    ctrl
}

/// draft-15 クライアントの SETTINGS (ENABLE_CONNECT_PROTOCOL あり)
fn build_draft15_client_ctrl_with_ecp() -> Vec<u8> {
    let wt = webtransport::Settings::new().wt_enabled(vi(1));
    let settings = Settings::new()
        .h3_datagram(true)
        .enable_webtransport_server(wt);
    let mut client = shiguredo_http3::ClientConnection::new(settings);
    client.set_control_stream_id(2).expect("test must succeed");
    let (ctrl, _) = client.take_stream_data(2).expect("test must succeed");
    ctrl
}

/// draft-15 クライアントの SETTINGS (ENABLE_CONNECT_PROTOCOL なし)
fn build_draft15_client_ctrl_without_ecp() -> Vec<u8> {
    let wt = webtransport::Settings::new().wt_enabled(vi(1));
    let settings = Settings {
        enable_connect_protocol: None,
        ..Settings::new()
            .h3_datagram(true)
            .enable_webtransport_server(wt)
    };
    let mut client = shiguredo_http3::ClientConnection::new(settings);
    client.set_control_stream_id(2).expect("test must succeed");
    let (ctrl, _) = client.take_stream_data(2).expect("test must succeed");
    ctrl
}

/// draft-02 / 07 / 14 / 15 / 16 に対応するサーバーを構築する
///
/// draft-17 のコードポイントは広告しないため、draft-17 の検証は
/// `mod draft17_connect` の専用 SETTINGS で行う。
fn setup_server(reset_stream_at: bool) -> ServerConnection {
    let wt = webtransport::Settings::new()
        .wt_enabled(vi(1))
        .enable_webtransport_draft02(true)
        .webtransport_max_sessions_draft07(vi(100))
        .wt_max_sessions_draft14(vi(100));
    let settings = Settings::new().enable_webtransport_server(wt);
    let mut server = ServerConnection::new(settings);
    server.set_control_stream_id(3).expect("test must succeed");
    server
        .set_webtransport_transport_verified(true, reset_stream_at)
        .expect("test must succeed");
    // サーバーの制御ストリームデータを消費
    let _ = server.take_stream_data(3).expect("test must succeed");
    server
}

/// サーバーにクライアントの制御ストリームデータを feed し、SETTINGS イベントを消費する
fn feed_client_settings(server: &mut ServerConnection, client_ctrl: &[u8]) {
    server
        .feed_stream(2, client_ctrl, false)
        .expect("test must succeed");
    // SETTINGS イベントを消費
    loop {
        match server.poll_event().expect("test must succeed") {
            Some(Event::SettingsReceived { .. }) => break,
            Some(_) => {}
            None => break,
        }
    }
}

/// WebTransport CONNECT ヘッダーを構築する
fn wt_connect_headers(draft: DraftVersion) -> Vec<Header> {
    vec![
        Header::new(b":method", b"CONNECT").expect("test must succeed"),
        Header::new(
            b":protocol",
            match draft {
                DraftVersion::Draft15 | DraftVersion::Draft17 => b"webtransport-h3" as &[u8],
                _ => b"webtransport",
            },
        )
        .expect("test must succeed"),
        Header::new(b":scheme", b"https").expect("test must succeed"),
        Header::new(b":authority", b"example.com").expect("test must succeed"),
        Header::new(b":path", b"/wt").expect("test must succeed"),
    ]
}

// =========================================================================
// draft-02: ENABLE_CONNECT_PROTOCOL 不要
// =========================================================================

mod draft02 {
    use super::*;

    #[test]
    fn connect_accepted_without_enable_connect_protocol() {
        // draft-02 クライアントは SETTINGS_ENABLE_WEBTRANSPORT_DRAFT02 が
        // Extended CONNECT のサポートを暗示するため、
        // SETTINGS_ENABLE_CONNECT_PROTOCOL は不要
        let mut server = setup_server(false);
        let client_ctrl = build_draft02_client_ctrl();
        feed_client_settings(&mut server, &client_ctrl);

        // peer SETTINGS 検証
        let peer = server.peer_settings().expect("test must succeed");
        assert!(peer.is_webtransport_enabled());
        assert_eq!(peer.enable_connect_protocol, None);
        assert_eq!(
            peer.webtransport_draft_pattern(),
            Some(DraftVersion::Draft02)
        );

        let headers = wt_connect_headers(DraftVersion::Draft02);
        let frame = build_headers_frame(&headers);
        let result = server.feed_stream(0, &frame, false);
        assert!(
            result.is_ok(),
            "draft-02 クライアントの CONNECT が拒否された: {result:?}"
        );
    }
}

// =========================================================================
// draft-07: ENABLE_CONNECT_PROTOCOL 不要 (Safari パターン)
// =========================================================================

mod draft07 {
    use super::*;

    #[test]
    fn connect_accepted_without_enable_connect_protocol() {
        // draft-07 クライアントは SETTINGS_WEBTRANSPORT_MAX_SESSIONS のみ MUST
        // SETTINGS_ENABLE_CONNECT_PROTOCOL はサーバーのみ MUST
        // (draft-ietf-webtrans-http3-07 Section 3.2)
        let mut server = setup_server(false);
        let client_ctrl = build_draft07_client_ctrl();
        feed_client_settings(&mut server, &client_ctrl);

        // peer SETTINGS 検証
        let peer = server.peer_settings().expect("test must succeed");
        assert!(peer.is_webtransport_enabled());
        assert_eq!(peer.enable_connect_protocol, None);
        assert_eq!(
            peer.webtransport_draft_pattern(),
            Some(DraftVersion::Draft07)
        );

        let headers = wt_connect_headers(DraftVersion::Draft07);
        let frame = build_headers_frame(&headers);
        let result = server.feed_stream(0, &frame, false);
        assert!(
            result.is_ok(),
            "draft-07 クライアントの CONNECT が拒否された: {result:?}"
        );
    }

    #[test]
    fn safari_pattern_with_draft14_settings_also_accepted() {
        // Safari は draft-07 + draft-14 の SETTINGS ID を両方送る
        // (WEBTRANSPORT_MAX_SESSIONS + WT_MAX_SESSIONS + WT_INITIAL_MAX_*)
        // SETTINGS ネゴシエーションは draft-07 を優先する (Safari が draft-14 固有の
        // 応答 SETTINGS を拒否するため)。draft-14 固有のカプセルベースフロー制御は
        // セッション確立後に別途扱う。
        let wt = webtransport::Settings::new()
            .webtransport_max_sessions_draft07(vi(1))
            .wt_max_sessions_draft14(vi(1))
            .wt_initial_max_data(vi(8 * 1024 * 1024))
            .wt_initial_max_streams_uni(vi(100))
            .wt_initial_max_streams_bidi(vi(100));
        let settings = Settings {
            enable_connect_protocol: None,
            ..Settings::new()
                .h3_datagram(true)
                .enable_webtransport_server(wt)
        };
        let mut client = shiguredo_http3::ClientConnection::new(settings);
        client.set_control_stream_id(2).expect("test must succeed");
        let (client_ctrl, _) = client.take_stream_data(2).expect("test must succeed");

        let mut server = setup_server(true);
        feed_client_settings(&mut server, &client_ctrl);

        let peer = server.peer_settings().expect("test must succeed");
        assert_eq!(
            peer.webtransport_draft_pattern(),
            Some(DraftVersion::Draft07),
            "Safari パターンは SETTINGS ネゴシエーションとして draft-07 優先で検出されるべき"
        );
        assert_eq!(peer.enable_connect_protocol, None);

        let headers = wt_connect_headers(DraftVersion::Draft07);
        let frame = build_headers_frame(&headers);
        let result = server.feed_stream(0, &frame, false);
        assert!(
            result.is_ok(),
            "Safari パターンの CONNECT が拒否された: {result:?}"
        );
    }
}

// =========================================================================
// draft-14: ENABLE_CONNECT_PROTOCOL 必須
// =========================================================================

mod draft14 {
    use super::*;

    #[test]
    fn connect_accepted_with_enable_connect_protocol() {
        // draft-14 クライアントは ENABLE_CONNECT_PROTOCOL を送信すべき
        // draft-14 は reset_stream_at transport parameter も必須 (Section 3.1)
        let mut server = setup_server(true);
        let client_ctrl = build_draft14_client_ctrl_with_ecp();
        feed_client_settings(&mut server, &client_ctrl);

        let peer = server.peer_settings().expect("test must succeed");
        assert!(peer.is_webtransport_enabled());
        assert_eq!(peer.enable_connect_protocol, Some(true));

        let headers = wt_connect_headers(DraftVersion::Draft14);
        let frame = build_headers_frame(&headers);
        let result = server.feed_stream(0, &frame, false);
        assert!(
            result.is_ok(),
            "draft-14 クライアントの CONNECT が拒否された: {result:?}"
        );
    }

    #[test]
    fn connect_accepted_without_enable_connect_protocol() {
        // ENABLE_CONNECT_PROTOCOL はサーバーが送る設定 (RFC 9220, RFC 8441 Section 3)
        // クライアントは送信義務がない (draft-ietf-webtrans-http3-14/15 Section 3.1)
        // draft-14 は reset_stream_at transport parameter が必須
        let mut server = setup_server(true);
        let client_ctrl = build_draft14_client_ctrl_without_ecp();
        feed_client_settings(&mut server, &client_ctrl);

        let peer = server.peer_settings().expect("test must succeed");
        assert!(peer.is_webtransport_enabled());
        assert_eq!(peer.enable_connect_protocol, None);

        let headers = wt_connect_headers(DraftVersion::Draft14);
        let frame = build_headers_frame(&headers);
        let result = server.feed_stream(0, &frame, false);
        assert!(
            result.is_ok(),
            "draft-14 クライアントの CONNECT が拒否された (ENABLE_CONNECT_PROTOCOL はクライアントに送信義務なし): {result:?}"
        );
    }
}

// =========================================================================
// draft-15: ENABLE_CONNECT_PROTOCOL 必須
// =========================================================================

mod draft15 {
    use super::*;

    #[test]
    fn connect_accepted_with_enable_connect_protocol() {
        // draft-15 クライアントは ENABLE_CONNECT_PROTOCOL + WT_ENABLED が必要
        let mut server = setup_server(true);
        let client_ctrl = build_draft15_client_ctrl_with_ecp();
        feed_client_settings(&mut server, &client_ctrl);

        let peer = server.peer_settings().expect("test must succeed");
        assert!(peer.is_webtransport_enabled());
        assert_eq!(peer.enable_connect_protocol, Some(true));

        let headers = wt_connect_headers(DraftVersion::Draft15);
        let frame = build_headers_frame(&headers);
        let result = server.feed_stream(0, &frame, false);
        assert!(
            result.is_ok(),
            "draft-15 クライアントの CONNECT が拒否された: {result:?}"
        );
    }

    #[test]
    fn connect_accepted_without_enable_connect_protocol() {
        // ENABLE_CONNECT_PROTOCOL はサーバーが送る設定 (RFC 9220, RFC 8441 Section 3)
        // クライアントは送信義務がない (draft-ietf-webtrans-http3-15 Section 3.1)
        let mut server = setup_server(true);
        let client_ctrl = build_draft15_client_ctrl_without_ecp();
        feed_client_settings(&mut server, &client_ctrl);

        let peer = server.peer_settings().expect("test must succeed");
        assert!(peer.is_webtransport_enabled());
        assert_eq!(peer.enable_connect_protocol, None);

        let headers = wt_connect_headers(DraftVersion::Draft15);
        let frame = build_headers_frame(&headers);
        let result = server.feed_stream(0, &frame, false);
        assert!(
            result.is_ok(),
            "draft-15 クライアントの CONNECT が拒否された (ENABLE_CONNECT_PROTOCOL はクライアントに送信義務なし): {result:?}"
        );
    }

    #[test]
    fn connect_rejected_without_reset_stream_at() {
        // draft-15 は RESET_STREAM_AT 拡張が必須
        // reset_stream_at_supported=false で拒否されるべき
        let mut server = setup_server(false); // reset_stream_at = false
        let client_ctrl = build_draft15_client_ctrl_with_ecp();
        feed_client_settings(&mut server, &client_ctrl);

        let headers = wt_connect_headers(DraftVersion::Draft15);
        let frame = build_headers_frame(&headers);
        let err = server.feed_stream(0, &frame, false).unwrap_err();
        assert!(
            matches!(err, Error::StreamError(ErrorCode::MessageError)),
            "draft-15 で RESET_STREAM_AT 未対応は拒否されるべき: {err:?}"
        );
    }
}

// =========================================================================
// 共通バリデーション (全 draft 共通)
// =========================================================================

mod common {
    use super::*;

    #[test]
    fn connect_accepted_without_common_draft_for_old_draft_server() {
        // draft-14 以前だけに対応するサーバーは、SETTINGS_WT_ENABLED による
        // コードポイント交渉をしない。クライアントが本文版のコードポイントだけを
        // 広告していても、従来どおり版依存の検証をスキップして CONNECT を処理する
        // (draft-ietf-webtrans-http3-14 Section 5.1 以前の挙動を維持する)
        let wt = webtransport::Settings::new().wt_max_sessions_draft14(vi(1));
        let settings = Settings::new().enable_webtransport_server(wt);
        let mut server = ServerConnection::new(settings);
        server
            .set_control_stream_id(3)
            .expect("テスト用の制御ストリーム設定に成功すること");
        server
            .set_webtransport_transport_verified(true, true)
            .expect("テスト用の transport parameter 検証に成功すること");
        let _ = server
            .take_stream_data(3)
            .expect("テスト用のサーバー SETTINGS 取得に成功すること");

        let client_wt = webtransport::Settings::new()
            .wt_enabled_draft17(vi(1))
            .wt_initial_max_data(vi(1024));
        let mut client = shiguredo_http3::ClientConnection::new(
            Settings::new().enable_webtransport_client(client_wt),
        );
        client
            .set_control_stream_id(2)
            .expect("テスト用の制御ストリーム設定に成功すること");
        let (client_ctrl, _) = client
            .take_stream_data(2)
            .expect("テスト用のクライアント SETTINGS 取得に成功すること");
        feed_client_settings(&mut server, &client_ctrl);

        let headers = wt_connect_headers(DraftVersion::Draft02);
        let frame = build_headers_frame(&headers);
        server
            .feed_stream(0, &frame, false)
            .expect("旧 draft のサーバーは版依存の検証をスキップして CONNECT を処理すること");
    }

    #[test]
    fn connect_rejected_when_h3_datagram_disabled() {
        // H3_DATAGRAM が無効なクライアントからの CONNECT は全 draft で拒否
        let wt = webtransport::Settings::new().webtransport_max_sessions_draft07(vi(1));
        let settings = Settings {
            enable_connect_protocol: None,
            h3_datagram: Some(false),
            ..Settings::new().enable_webtransport_server(wt)
        };
        let mut client = shiguredo_http3::ClientConnection::new(settings);
        client.set_control_stream_id(2).expect("test must succeed");
        let (client_ctrl, _) = client.take_stream_data(2).expect("test must succeed");

        let mut server = setup_server(false);
        feed_client_settings(&mut server, &client_ctrl);

        let headers = wt_connect_headers(DraftVersion::Draft07);
        let frame = build_headers_frame(&headers);
        let err = server.feed_stream(0, &frame, false).unwrap_err();
        assert!(
            matches!(err, Error::StreamError(ErrorCode::MessageError)),
            "H3_DATAGRAM 無効時は拒否されるべき: {err:?}"
        );
    }

    #[test]
    fn connect_rejected_when_wt_disabled() {
        // WebTransport が無効なクライアントからの CONNECT は拒否
        let settings = Settings::new()
            .h3_datagram(true)
            .enable_connect_protocol(true);
        let mut client = shiguredo_http3::ClientConnection::new(settings);
        client.set_control_stream_id(2).expect("test must succeed");
        let (client_ctrl, _) = client.take_stream_data(2).expect("test must succeed");

        let mut server = setup_server(false);
        feed_client_settings(&mut server, &client_ctrl);

        let peer = server.peer_settings().expect("test must succeed");
        assert!(!peer.is_webtransport_enabled());

        let headers = wt_connect_headers(DraftVersion::Draft07);
        let frame = build_headers_frame(&headers);
        let err = server.feed_stream(0, &frame, false).unwrap_err();
        assert!(
            matches!(err, Error::StreamError(ErrorCode::MessageError)),
            "WT 無効時は拒否されるべき: {err:?}"
        );
    }

    #[test]
    fn connect_rejected_when_scheme_not_https() {
        // :scheme が https でない場合は拒否
        let mut server = setup_server(false);
        let client_ctrl = build_draft07_client_ctrl();
        feed_client_settings(&mut server, &client_ctrl);

        let headers = vec![
            Header::new(b":method", b"CONNECT").expect("test must succeed"),
            Header::new(b":protocol", b"webtransport").expect("test must succeed"),
            Header::new(b":scheme", b"http").expect("test must succeed"),
            Header::new(b":authority", b"example.com").expect("test must succeed"),
            Header::new(b":path", b"/wt").expect("test must succeed"),
        ];
        let frame = build_headers_frame(&headers);
        let err = server.feed_stream(0, &frame, false).unwrap_err();
        assert!(
            matches!(err, Error::StreamError(ErrorCode::MessageError)),
            ":scheme=http は拒否されるべき: {err:?}"
        );
    }

    #[test]
    fn connect_rejected_without_transport_verified() {
        // wt_transport_verified が false の場合は拒否
        let wt = webtransport::Settings::new().webtransport_max_sessions_draft07(vi(1));
        let settings = Settings::new().enable_webtransport_server(wt);
        let mut server = ServerConnection::new(settings);
        server.set_control_stream_id(3).expect("test must succeed");
        // set_webtransport_transport_verified を呼ばない
        let _ = server.take_stream_data(3).expect("test must succeed");

        let client_ctrl = build_draft07_client_ctrl();
        feed_client_settings(&mut server, &client_ctrl);

        let headers = wt_connect_headers(DraftVersion::Draft07);
        let frame = build_headers_frame(&headers);
        let err = server.feed_stream(0, &frame, false).unwrap_err();
        assert!(
            matches!(err, Error::StreamError(ErrorCode::MessageError)),
            "transport_verified なしは拒否されるべき: {err:?}"
        );
    }

    #[test]
    fn non_wt_extended_connect_not_affected() {
        // WebSocket 等の Extended CONNECT は WebTransport チェックの対象外
        let mut server = setup_server(false);
        // WT 無効なクライアント SETTINGS を feed
        let settings = Settings::new()
            .h3_datagram(false)
            .enable_connect_protocol(true);
        let mut client = shiguredo_http3::ClientConnection::new(settings);
        client.set_control_stream_id(2).expect("test must succeed");
        let (client_ctrl, _) = client.take_stream_data(2).expect("test must succeed");
        feed_client_settings(&mut server, &client_ctrl);

        let headers = vec![
            Header::new(b":method", b"CONNECT").expect("test must succeed"),
            Header::new(b":protocol", b"websocket").expect("test must succeed"),
            Header::new(b":scheme", b"https").expect("test must succeed"),
            Header::new(b":authority", b"example.com").expect("test must succeed"),
            Header::new(b":path", b"/ws").expect("test must succeed"),
        ];
        let frame = build_headers_frame(&headers);
        // WebSocket CONNECT は WebTransport のチェックを通らない
        let result = server.feed_stream(0, &frame, false);
        assert!(
            result.is_ok(),
            "WebSocket CONNECT は WT チェック対象外: {result:?}"
        );
    }
}

// =========================================================================
// フロー制御無効時の同時セッション数制限
// draft-ietf-webtrans-http3-15 Section 5.1, 5.2
// =========================================================================

mod no_flow_control_single_session {
    use super::*;

    /// FC を宣言しない (= initial_max_* を持たない) draft-15 クライアントを作る
    fn build_draft15_client_no_fc() -> shiguredo_http3::ClientConnection {
        let wt = webtransport::Settings::new().wt_enabled(vi(1));
        let settings = Settings::new()
            .h3_datagram(true)
            .enable_webtransport_server(wt);
        let mut client = shiguredo_http3::ClientConnection::new(settings);
        client.set_control_stream_id(2).expect("test must succeed");
        client
            .set_webtransport_transport_verified(true, true)
            .expect("test must succeed");
        client
    }

    /// サーバー側: FC 無効時に 2 本目の WT CONNECT が H3_REQUEST_REJECTED で
    /// 拒否されることを確認する。
    #[test]
    fn server_rejects_second_wt_connect_when_no_flow_control() {
        let mut server = setup_server(true);
        let client_ctrl = build_draft15_client_ctrl_with_ecp();
        feed_client_settings(&mut server, &client_ctrl);

        // 1 本目: 受理される
        let headers = wt_connect_headers(DraftVersion::Draft15);
        let frame = build_headers_frame(&headers);
        server
            .feed_stream(0, &frame, false)
            .expect("1 本目の WT CONNECT は受理されるべき");

        // 2 本目: H3_REQUEST_REJECTED で拒否される
        let frame2 = build_headers_frame(&headers);
        let err = server.feed_stream(4, &frame2, false).unwrap_err();
        assert!(
            matches!(err, Error::StreamError(ErrorCode::RequestRejected)),
            "FC 無効時の 2 本目は H3_REQUEST_REJECTED で拒否されるべき: {err:?}"
        );
    }

    /// クライアント側: FC 無効時に 2 本目の WT CONNECT 送信が拒否されることを確認する。
    #[test]
    fn client_rejects_second_wt_connect_when_no_flow_control() {
        // クライアント側: 2 本目の send_request が RequestRejected になることを検証する。
        // peer SETTINGS を注入するため、サーバー側の制御ストリームデータを feed する。
        let mut client = build_draft15_client_no_fc();

        // サーバー側 SETTINGS を構築 (FC 無効: wt_initial_max_* を持たない)
        let server_wt = webtransport::Settings::new().wt_enabled(vi(1));
        let server_settings = Settings::new()
            .h3_datagram(true)
            .enable_connect_protocol(true)
            .enable_webtransport_server(server_wt);
        let mut server = ServerConnection::new(server_settings);
        server.set_control_stream_id(3).expect("test must succeed");
        let (server_ctrl, _) = server.take_stream_data(3).expect("test must succeed");

        // クライアントにサーバー制御ストリームを feed して peer SETTINGS を確定
        client
            .feed_stream(3, &server_ctrl, false)
            .expect("test must succeed");
        // SETTINGS イベントを消費
        while let Some(_ev) = client.poll_event().expect("test must succeed") {}

        let headers = wt_connect_headers(DraftVersion::Draft15);

        // 1 本目: 受理される
        client
            .send_request(&headers, false)
            .expect("1 本目の WT CONNECT は送信できるべき");

        // 2 本目: RequestRejected で拒否される (接続は維持)
        let err = client.send_request(&headers, false).unwrap_err();
        assert!(
            matches!(err, Error::StreamError(ErrorCode::RequestRejected)),
            "FC 無効時の 2 本目は StreamError(RequestRejected) で拒否されるべき: {err:?}"
        );
    }
}

// =========================================================================
// WT-Available-Protocols 未送時の WT-Protocol 検証
// draft-ietf-webtrans-http3-15 Section 3.3
// =========================================================================

mod wt_protocol_without_available_protocols {
    use super::*;
    use shiguredo_http3::Event;
    use shiguredo_http3::webtransport::ErrorCode as WtErrorCode;

    /// サーバー側: client が WT-Available-Protocols を送っていないのに
    /// WT-Protocol 付き 2xx を返そうとすると send_response がエラーを返すこと
    #[test]
    fn server_rejects_wt_protocol_when_no_available_protocols() {
        let mut server = setup_server(true);
        let client_ctrl = build_draft15_client_ctrl_with_ecp();
        feed_client_settings(&mut server, &client_ctrl);

        // wt-available-protocols 無しの WT CONNECT を受信
        let headers = wt_connect_headers(DraftVersion::Draft15);
        let frame = build_headers_frame(&headers);
        server
            .feed_stream(0, &frame, false)
            .expect("WT CONNECT は受理されるべき");

        // RequestReceived イベントを消費する
        while let Some(_ev) = server.poll_event().expect("test must succeed") {}

        // wt-protocol を含む 2xx を返そうとする → エラー
        let response = vec![
            Header::new(b":status", b"200").expect("test must succeed"),
            Header::new(b"wt-protocol", b"\"echo\"").expect("test must succeed"),
        ];
        let err = server.send_response(0, &response, false).unwrap_err();
        assert!(
            matches!(err, Error::ConnectionError(ErrorCode::InternalError)),
            "WT-Available-Protocols 未送時の WT-Protocol 付き 2xx は拒否されるべき: {err:?}"
        );
    }

    /// クライアント側: 自分が WT-Available-Protocols を送っていないのに
    /// server から WT-Protocol 付き 2xx が来た場合、セッションが終了されること
    #[test]
    fn client_rejects_wt_protocol_when_no_available_protocols() {
        // クライアントを構築
        let wt = webtransport::Settings::new()
            .wt_enabled(vi(1))
            .wt_initial_max_streams_uni(vi(100))
            .wt_initial_max_streams_bidi(vi(100))
            .wt_initial_max_data(vi(8 * 1024 * 1024));
        let settings = Settings::new()
            .h3_datagram(true)
            .enable_webtransport_server(wt);
        let mut client = shiguredo_http3::ClientConnection::new(settings);
        client.set_control_stream_id(2).expect("test must succeed");
        client
            .set_webtransport_transport_verified(true, true)
            .expect("test must succeed");
        let _ = client.take_stream_data(2).expect("test must succeed");

        // サーバーの SETTINGS を構築して feed
        let server_wt = webtransport::Settings::new()
            .wt_enabled(vi(1))
            .wt_initial_max_streams_uni(vi(100))
            .wt_initial_max_streams_bidi(vi(100))
            .wt_initial_max_data(vi(8 * 1024 * 1024));
        let server_settings = Settings::new()
            .h3_datagram(true)
            .enable_connect_protocol(true)
            .enable_webtransport_server(server_wt);
        let mut server = ServerConnection::new(server_settings);
        server.set_control_stream_id(3).expect("test must succeed");
        let (server_ctrl, _) = server.take_stream_data(3).expect("test must succeed");
        client
            .feed_stream(3, &server_ctrl, false)
            .expect("test must succeed");
        while let Some(_ev) = client.poll_event().expect("test must succeed") {}

        // wt-available-protocols を含まない WT CONNECT を送信
        let headers = wt_connect_headers(DraftVersion::Draft15);
        let stream_id = client
            .send_request(&headers, false)
            .expect("test must succeed");
        // 送信バッファを取り出して破棄 (peer に送らない)
        let _ = client.take_stream_data(stream_id);

        // 偽の 2xx + WT-Protocol レスポンスをクライアントに feed する
        let response = vec![
            Header::new(b":status", b"200").expect("test must succeed"),
            Header::new(b"wt-protocol", b"\"echo\"").expect("test must succeed"),
        ];
        let frame = build_headers_frame(&response);
        client
            .feed_stream(stream_id, &frame, false)
            .expect("test must succeed");

        // セッションは Established に遷移せず、WebTransportEvent::SessionClosed が発火する
        let mut session_closed = false;
        let mut session_established = false;
        while let Some(ev) = client.poll_event().expect("test must succeed") {
            match ev {
                Event::WebTransport(WebTransportEvent::SessionClosed { error_code, .. }) => {
                    session_closed = true;
                    assert_eq!(
                        error_code,
                        WtErrorCode::AlpnError as u64,
                        "WT_ALPN_ERROR で閉じられるべき"
                    );
                }
                Event::WebTransport(WebTransportEvent::SessionEstablished { .. }) => {
                    session_established = true;
                }
                _ => {}
            }
        }
        assert!(
            !session_established,
            "違反レスポンスでセッションが確立されてはならない"
        );
        assert!(session_closed, "違反レスポンスでセッションが閉じられるべき");
    }
}

// =========================================================================
// :protocol と SETTINGS でネゴシエートしたドラフトの整合性検証
// (draft-ietf-webtrans-http3-15 Section 3.2 / 7.1)
// =========================================================================

mod protocol_draft_alignment {
    use super::*;

    /// `:protocol` を任意指定して WT CONNECT ヘッダーを構築する
    fn wt_connect_headers_with_protocol(protocol: &'static [u8]) -> Vec<Header> {
        vec![
            Header::new(b":method", b"CONNECT").expect("test must succeed"),
            Header::new(b":protocol", protocol).expect("test must succeed"),
            Header::new(b":scheme", b"https").expect("test must succeed"),
            Header::new(b":authority", b"example.com").expect("test must succeed"),
            Header::new(b":path", b"/wt").expect("test must succeed"),
        ]
    }

    #[test]
    fn server_rejects_legacy_protocol_on_draft15() {
        // draft-15 をネゴシエートした接続でクライアントが旧値 "webtransport" を
        // 送ってきた場合、サーバーは MessageError でストリームを拒否する
        let mut server = setup_server(true);
        let client_ctrl = build_draft15_client_ctrl_with_ecp();
        feed_client_settings(&mut server, &client_ctrl);

        let headers = wt_connect_headers_with_protocol(b"webtransport");
        let frame = build_headers_frame(&headers);
        let result = server.feed_stream(0, &frame, false);
        assert!(matches!(
            result,
            Err(Error::StreamError(ErrorCode::MessageError))
        ));
    }

    #[test]
    fn server_rejects_new_protocol_on_draft07() {
        // draft-07 をネゴシエートした接続でクライアントが新値 "webtransport-h3" を
        // 送ってきた場合、サーバーは MessageError でストリームを拒否する
        let mut server = setup_server(false);
        let client_ctrl = build_draft07_client_ctrl();
        feed_client_settings(&mut server, &client_ctrl);

        let headers = wt_connect_headers_with_protocol(b"webtransport-h3");
        let frame = build_headers_frame(&headers);
        let result = server.feed_stream(0, &frame, false);
        assert!(matches!(
            result,
            Err(Error::StreamError(ErrorCode::MessageError))
        ));
    }

    #[test]
    fn server_rejects_new_protocol_on_draft14() {
        // draft-14 をネゴシエートした接続でも :protocol は "webtransport"
        let mut server = setup_server(true);
        let client_ctrl = build_draft14_client_ctrl_with_ecp();
        feed_client_settings(&mut server, &client_ctrl);

        let headers = wt_connect_headers_with_protocol(b"webtransport-h3");
        let frame = build_headers_frame(&headers);
        let result = server.feed_stream(0, &frame, false);
        assert!(matches!(
            result,
            Err(Error::StreamError(ErrorCode::MessageError))
        ));
    }

    #[test]
    fn client_send_request_rejects_mismatched_protocol_on_draft15() {
        // クライアントが draft-15 の peer に旧値 "webtransport" を送ろうとした場合、
        // send_request は ConnectionError(InternalError) を返す
        let server_wt = webtransport::Settings::new()
            .wt_enabled(vi(1))
            .wt_initial_max_streams_uni(vi(100))
            .wt_initial_max_streams_bidi(vi(100))
            .wt_initial_max_data(vi(8 * 1024 * 1024));
        let server_settings = Settings::new()
            .h3_datagram(true)
            .enable_connect_protocol(true)
            .enable_webtransport_server(server_wt);
        let mut server = ServerConnection::new(server_settings);
        server.set_control_stream_id(3).expect("test must succeed");
        let (server_ctrl, _) = server.take_stream_data(3).expect("test must succeed");

        let client_wt = webtransport::Settings::new().wt_enabled(vi(1));
        let client_settings = Settings::new()
            .h3_datagram(true)
            .enable_webtransport_server(client_wt);
        let mut client = shiguredo_http3::ClientConnection::new(client_settings);
        client.set_control_stream_id(2).expect("test must succeed");
        client
            .set_webtransport_transport_verified(true, true)
            .expect("test must succeed");
        client
            .feed_stream(3, &server_ctrl, false)
            .expect("test must succeed");
        while let Some(_ev) = client.poll_event().expect("test must succeed") {}

        let headers = wt_connect_headers_with_protocol(b"webtransport");
        let result = client.send_request(&headers, false);
        assert!(matches!(
            result,
            Err(Error::WtSetup(WtSetupError::ProtocolMismatch))
        ));
    }
}

// =========================================================================
// Pending CONNECT ストリームへの DATA フレーム受信時の挙動
//
// Chrome の draft-02 互換実装は CONNECT 直後 (レスポンス受信前) に
// CONNECT ストリームへ DATA フレームを書き込んでくるため、draft-02 では
// Pending 中の DATA を黙って破棄する。draft-07/14/15 では仕様に従って
// H3_MESSAGE_ERROR を返す。
// =========================================================================

mod pending_data_frame {
    use super::*;

    /// HEADERS フレームの直後に DATA フレームを連結したものを返す
    fn build_headers_then_data(headers: &[Header], body: &[u8]) -> Vec<u8> {
        let mut frame = build_headers_frame(headers);
        // DATA フレーム: type=0x00 + length(varint) + body
        shiguredo_http3::varint::encode_into_vec(&mut frame, shiguredo_http3::VarInt::ZERO);
        shiguredo_http3::varint::encode_into_vec(
            &mut frame,
            shiguredo_http3::VarInt::new(body.len() as u64).expect("body length fits in VarInt"),
        );
        frame.extend_from_slice(body);
        frame
    }

    #[test]
    fn draft02_pending_data_is_silently_dropped() {
        // Chrome (draft-02) は CONNECT 直後にレスポンスを待たず DATA を送る。
        // draft-02 には Capsule Protocol が無いので Pending 中の DATA は破棄する。
        let mut server = setup_server(false);
        let client_ctrl = build_draft02_client_ctrl();
        feed_client_settings(&mut server, &client_ctrl);

        let headers = wt_connect_headers(DraftVersion::Draft02);
        let frame = build_headers_then_data(&headers, &[0xde, 0xad, 0xbe, 0xef]);
        let result = server.feed_stream(0, &frame, false);
        assert!(
            result.is_ok(),
            "draft-02 の Pending 中 DATA がエラーになった: {result:?}"
        );
    }

    #[test]
    fn draft07_draft14_draft15_pending_data_buffered() {
        // 楽観的カプセル送信: draft-07 以降の Pending 中の DATA は
        // バッファリングされる (draft-02 は破棄する)
        for (draft, reset_stream_at) in [
            (DraftVersion::Draft07, false),
            (DraftVersion::Draft14, true),
            (DraftVersion::Draft15, true),
        ] {
            let mut server = setup_server(reset_stream_at);
            let client_ctrl = match draft {
                DraftVersion::Draft07 => build_draft07_client_ctrl(),
                DraftVersion::Draft14 => build_draft14_client_ctrl_with_ecp(),
                _ => build_draft15_client_ctrl_with_ecp(),
            };
            feed_client_settings(&mut server, &client_ctrl);

            let headers = wt_connect_headers(draft);
            let frame = build_headers_then_data(&headers, &[0x00]);
            let result = server.feed_stream(0, &frame, false);
            assert!(
                result.is_ok(),
                "{draft:?} の Pending 中 DATA がバッファリングされずエラーになった: {result:?}"
            );
        }
    }

    /// WebTransport カプセルを DATA フレームとしてエンコードするヘルパー
    fn capsule_data_frame(capsule: &webtransport::Capsule) -> Vec<u8> {
        let mut buf = Vec::new();
        capsule.encode_as_data_frame(&mut buf);
        buf
    }

    #[test]
    fn draft15_pending_capsule_buffered_then_processed_on_accept() {
        // 楽観的カプセル送信: Pending 中にバッファリングされたカプセルが
        // 2xx レスポンス送信後に処理される
        let mut server = setup_server(true);
        let client_ctrl = build_draft15_client_ctrl_with_ecp();
        feed_client_settings(&mut server, &client_ctrl);

        let headers = wt_connect_headers(DraftVersion::Draft15);
        let frame = build_headers_frame(&headers);
        server
            .feed_stream(0, &frame, false)
            .expect("テスト用の CONNECT ヘッダー送信に成功すること");

        // Pending 中に WT_CLOSE_SESSION カプセルを送信 (楽観的送信)
        let close_session = webtransport::Capsule::CloseSession {
            error_code: 0,
            message: String::new(),
        };
        let data_frame = capsule_data_frame(&close_session);
        server
            .feed_stream(0, &data_frame, false)
            .expect("楽観的カプセル送信がバッファリングされること");

        // 2xx レスポンスを送信
        let response = vec![
            Header::new(b":status", b"200").expect("テスト用のヘッダー作成に成功すること"),
            Header::new(b"wt-protocol", b"test").expect("テスト用のヘッダー作成に成功すること"),
        ];
        server
            .send_response(0, &response, false)
            .expect("テスト用のレスポンス送信に成功すること");

        // セッション確立後、バッファリングされたカプセルが処理されて
        // SessionEstablished イベントが発火する
        let events = server
            .drain_events()
            .expect("テスト用のイベント取得に成功すること");
        let has_established = events.iter().any(|e| {
            matches!(
                e,
                Event::WebTransport(WebTransportEvent::SessionEstablished { .. })
            )
        });
        assert!(has_established, "SessionEstablished イベントが発火すること");
    }

    #[test]
    fn draft15_pending_data_discarded_on_reject() {
        // 非 2xx レスポンス送信時に Pending セッションのバッファが破棄される
        let mut server = setup_server(true);
        let client_ctrl = build_draft15_client_ctrl_with_ecp();
        feed_client_settings(&mut server, &client_ctrl);

        let headers = wt_connect_headers(DraftVersion::Draft15);
        let frame = build_headers_then_data(&headers, &[0x00]);
        server
            .feed_stream(0, &frame, false)
            .expect("楽観的カプセル送信がバッファリングされること");

        // 非 2xx レスポンスを送信
        let response =
            vec![Header::new(b":status", b"404").expect("テスト用のヘッダー作成に成功すること")];
        server
            .send_response(0, &response, true)
            .expect("テスト用のレスポンス送信に成功すること");

        // セッションが終了し、後続の DATA は H3_MESSAGE_ERROR になる
        let data_frame = capsule_data_frame(&webtransport::Capsule::CloseSession {
            error_code: 0,
            message: String::new(),
        });
        let err = server.feed_stream(0, &data_frame, false).unwrap_err();
        assert!(matches!(err, Error::StreamError(ErrorCode::MessageError)));
    }

    #[test]
    fn draft15_pending_fin_terminates_session() {
        // Pending 状態で CONNECT ストリームの FIN を受信した場合、
        // バッファを破棄してセッションを終了する
        let mut server = setup_server(true);
        let client_ctrl = build_draft15_client_ctrl_with_ecp();
        feed_client_settings(&mut server, &client_ctrl);

        let headers = wt_connect_headers(DraftVersion::Draft15);
        let frame = build_headers_then_data(&headers, &[0x00]);
        server
            .feed_stream(0, &frame, false)
            .expect("楽観的カプセル送信がバッファリングされること");

        // FIN でセッション終了
        server
            .feed_stream(0, &[], true)
            .expect("Pending 状態の FIN がエラーにならないこと");

        // 後続の DATA は tombstone 経由で H3_MESSAGE_ERROR
        let data_frame = capsule_data_frame(&webtransport::Capsule::CloseSession {
            error_code: 0,
            message: String::new(),
        });
        let err = server.feed_stream(0, &data_frame, false).unwrap_err();
        assert!(matches!(err, Error::StreamError(ErrorCode::MessageError)));
    }
}

// =========================================================================
// CONNECT ストリーム上の WT_CLOSE_SESSION カプセルの H3 DATA フレーム包装
// =========================================================================

/// CONNECT ストリーム上の WT_CLOSE_SESSION が H3 DATA フレームに
/// 包まれているときのみ受信側で処理されることを検証する回帰テスト。
///
/// RFC 9297 Section 3.1 / RFC 9114 Section 7.2.1 / draft-ietf-webtrans-http3-16 Section 6
/// により、CONNECT ストリーム上のカプセルは HTTP/3 の DATA フレーム
/// (0x00 + varint 長 + ペイロード) として送出する必要があり、WT_CLOSE_SESSION も
/// DATA フレームに包まなければならない。包まずに raw カプセルバイトのみを送ると、
/// 先頭 varint (0x2843) が H3 フレームタイプとして解釈され、未知フレームとして
/// 無視されるためセッション終了通知に到達しない。
///
/// テストは既存の draft-15 セットアップヘルパーを流用する。
/// WT_CLOSE_SESSION の DATA フレーム包装要件と `SessionClosed` イベント発火経路は
/// draft-15 と draft-16 で同一のため妥当。
mod close_session_capsule_framing {
    use super::*;

    /// draft-15 で CONNECT ストリームを確立し、セッション ID を返す
    fn establish_draft15_session(server: &mut ServerConnection) -> u64 {
        let client_ctrl = build_draft15_client_ctrl_with_ecp();
        feed_client_settings(server, &client_ctrl);

        let headers = wt_connect_headers(DraftVersion::Draft15);
        let frame = build_headers_frame(&headers);
        server
            .feed_stream(0, &frame, false)
            .expect("テスト用の CONNECT ヘッダー送信に成功すること");

        let response = vec![
            Header::new(b":status", b"200").expect("テスト用のヘッダー作成に成功すること"),
            Header::new(b"wt-protocol", b"test").expect("テスト用のヘッダー作成に成功すること"),
        ];
        server
            .send_response(0, &response, false)
            .expect("テスト用のレスポンス送信に成功すること");

        // SessionEstablished イベントを消費
        loop {
            match server
                .poll_event()
                .expect("SessionEstablished 待機中のイベント取得に成功すること")
            {
                Some(Event::WebTransport(WebTransportEvent::SessionEstablished {
                    session_id,
                    ..
                })) => return session_id,
                Some(_) => {}
                None => panic!("SessionEstablished イベントが発火していない"),
            }
        }
    }

    #[test]
    fn wt_close_session_wrapped_in_data_frame_is_processed() {
        // WT_CLOSE_SESSION を H3 DATA フレームに包んで送信すると
        // SessionClosed イベントとして処理される (正しい経路)
        let mut server = setup_server(true);
        let session_id = establish_draft15_session(&mut server);

        let close_session = webtransport::Capsule::CloseSession {
            error_code: 42,
            message: String::from("bye"),
        };
        let mut framed = Vec::new();
        close_session.encode_as_data_frame(&mut framed);
        server
            .feed_stream(session_id, &framed, false)
            .expect("DATA フレームに包んだ WT_CLOSE_SESSION 送信に成功すること");

        // SessionClosed イベントが発火し、カプセルの値が伝播する
        let events = server
            .drain_events()
            .expect("テスト用のイベント取得に成功すること");
        let closed = events.iter().find_map(|e| match e {
            Event::WebTransport(WebTransportEvent::SessionClosed {
                session_id: sid,
                close_error_code,
                close_message,
                ..
            }) if *sid == session_id => Some((*close_error_code, close_message.clone())),
            _ => None,
        });
        let (close_error_code, close_message) =
            closed.expect("DATA フレーム包装済み WT_CLOSE_SESSION で SessionClosed が発火すること");
        assert_eq!(
            close_error_code, 42,
            "SessionClosed の close_error_code がカプセルの値と一致すること"
        );
        assert_eq!(
            close_message, "bye",
            "SessionClosed の close_message がカプセルの値と一致すること"
        );
    }

    #[test]
    fn wt_close_session_raw_capsule_bytes_are_not_processed() {
        // WT_CLOSE_SESSION の raw カプセルバイト (DATA フレームで包まない) を送信すると
        // 先頭 varint 0x2843 が H3 フレームタイプとして解釈され、未知フレームとして
        // 無音で破棄されるため、SessionClosed イベントは発火せず、他のイベントも発火しない
        let mut server = setup_server(true);
        let session_id = establish_draft15_session(&mut server);

        let close_session = webtransport::Capsule::CloseSession {
            error_code: 42,
            message: String::from("bye"),
        };
        let mut raw = Vec::new();
        close_session.encode(&mut raw);
        // 未知フレームなのでエラーにならず、そのまま受理される (フレームボディはスキップされる)
        server
            .feed_stream(session_id, &raw, false)
            .expect("raw カプセルバイト送信が未知フレームとして受理されること");

        let events = server
            .drain_events()
            .expect("テスト用のイベント取得に成功すること");
        assert!(
            !events.iter().any(|e| matches!(
                e,
                Event::WebTransport(WebTransportEvent::SessionClosed { session_id: sid, .. })
                    if *sid == session_id
            )),
            "raw カプセルバイトからは SessionClosed が発火しないこと: {events:?}"
        );
    }
}

// =========================================================================
// draft-17 の CONNECT 受理
// (draft-ietf-webtrans-http3-17 Section 3.1, 7.1)
//
// 両端が draft-17 の新コードポイント (0x20b8933b) を広告する場合の CONNECT 受理と、
// 本文版だけに対応するクライアント (SETTINGS_WT_ENABLED を送らない) の CONNECT 受理を
// 公開 API 経由で検証する。SETTINGS 単体の保持・検出は
// tests/test_webtransport_settings.rs で扱う。
// =========================================================================

mod draft17_connect {
    use super::*;

    /// draft-17 の新コードポイントと初期フロー制御値を広告するサーバー SETTINGS を構築する
    fn draft17_server_settings() -> Settings {
        let wt = webtransport::Settings::new()
            .wt_enabled_draft17(vi(1))
            .wt_initial_max_streams_uni(vi(100))
            .wt_initial_max_streams_bidi(vi(100))
            .wt_initial_max_data(vi(8 * 1024 * 1024));
        Settings::new().enable_webtransport_server(wt)
    }

    /// draft-17 の新コードポイントを広告するクライアント SETTINGS を構築する
    ///
    /// draft-17 Section 3.1 がクライアントに求める送信項目に
    /// SETTINGS_ENABLE_CONNECT_PROTOCOL は含まれないため、
    /// `Settings::enable_webtransport_client` を使う。
    fn draft17_client_settings() -> Settings {
        let wt = webtransport::Settings::new()
            .wt_enabled_draft17(vi(1))
            .wt_initial_max_streams_uni(vi(100))
            .wt_initial_max_streams_bidi(vi(100))
            .wt_initial_max_data(vi(8 * 1024 * 1024));
        Settings::new().enable_webtransport_client(wt)
    }

    /// SETTINGS_WT_ENABLED を送らないクライアントの SETTINGS を構築する
    ///
    /// draft-17 の本文版だけに対応するクライアントは SETTINGS_WT_ENABLED を送らず、
    /// フロー制御は SETTINGS_WT_INITIAL_MAX_* だけで宣言する
    /// (draft-ietf-webtrans-http3-17 Section 3.1, 5.1)。
    fn draft17_client_settings_without_wt_enabled() -> Settings {
        let wt = webtransport::Settings::new().wt_initial_max_data(vi(1024));
        Settings::new().enable_webtransport_client(wt)
    }

    /// 両端が draft-17 の新コードポイントを広告する接続を作り、SETTINGS を交換する
    fn draft17_pair() -> (ServerConnection, shiguredo_http3::ClientConnection) {
        let mut server = ServerConnection::new(draft17_server_settings());
        server
            .set_control_stream_id(3)
            .expect("テスト用の制御ストリーム設定に成功すること");
        server
            .set_webtransport_transport_verified(true, true)
            .expect("テスト用の transport parameter 検証に成功すること");
        let (server_ctrl, _) = server
            .take_stream_data(3)
            .expect("テスト用のサーバー SETTINGS 取得に成功すること");

        let mut client = shiguredo_http3::ClientConnection::new(draft17_client_settings());
        client
            .set_control_stream_id(2)
            .expect("テスト用の制御ストリーム設定に成功すること");
        client
            .set_webtransport_transport_verified(true, true)
            .expect("テスト用の transport parameter 検証に成功すること");
        let (client_ctrl, _) = client
            .take_stream_data(2)
            .expect("テスト用のクライアント SETTINGS 取得に成功すること");

        client
            .feed_stream(3, &server_ctrl, false)
            .expect("クライアントのサーバー SETTINGS 受信に成功すること");
        while let Some(_ev) = client
            .poll_event()
            .expect("テスト用のイベント取得に成功すること")
        {}
        server
            .feed_stream(2, &client_ctrl, false)
            .expect("サーバーのクライアント SETTINGS 受信に成功すること");
        while let Some(_ev) = server
            .poll_event()
            .expect("テスト用のイベント取得に成功すること")
        {}
        (server, client)
    }

    /// draft-17 の CONNECT を送り、サーバーが 2xx を返してセッションを確立するまで進める
    ///
    /// 確立した WebTransport セッション ID を返す。
    fn establish_draft17_session(
        server: &mut ServerConnection,
        client: &mut shiguredo_http3::ClientConnection,
    ) -> u64 {
        let headers = wt_connect_headers(DraftVersion::Draft17);
        let stream_id = client
            .send_request(&headers, false)
            .expect("draft-17 の CONNECT 送信に成功すること");
        let (request, _) = client
            .take_stream_data(stream_id)
            .expect("テスト用の CONNECT ヘッダー取得に成功すること");
        server
            .feed_stream(stream_id, &request, false)
            .expect("draft-17 の CONNECT 受理に成功すること");

        let response =
            vec![Header::new(b":status", b"200").expect("テスト用のヘッダー作成に成功すること")];
        server
            .send_response(stream_id, &response, false)
            .expect("テスト用のレスポンス送信に成功すること");

        loop {
            match server
                .poll_event()
                .expect("SessionEstablished 待機中のイベント取得に成功すること")
            {
                Some(Event::WebTransport(WebTransportEvent::SessionEstablished {
                    session_id,
                    ..
                })) => {
                    assert_eq!(
                        session_id, stream_id,
                        "セッション ID は CONNECT ストリーム ID と一致すること"
                    );
                    return session_id;
                }
                Some(_) => {}
                None => panic!("SessionEstablished イベントが発火していない"),
            }
        }
    }

    /// 新旧両コードポイントと draft-02 を広告するサーバーとクライアントの接続を作り、
    /// SETTINGS を交換する
    ///
    /// 複数のドラフト版に対応する端点は、異なるコードポイントをそれぞれ 1 回ずつ
    /// 広告する (draft-ietf-webtrans-http3-17 Section 7.1)。draft-02 の ID を
    /// 併送することで、交差集合に旧トークン `webtransport` を返す版が含まれる
    /// 状態になる (選択版のトークンだけが通ることを検証するため)。
    fn both_codepoints_pair() -> (ServerConnection, shiguredo_http3::ClientConnection) {
        let server_wt = webtransport::Settings::new()
            .wt_enabled(vi(1))
            .wt_enabled_draft17(vi(1))
            .enable_webtransport_draft02(true)
            .wt_initial_max_data(vi(1024));
        let mut server =
            ServerConnection::new(Settings::new().enable_webtransport_server(server_wt));
        server
            .set_control_stream_id(3)
            .expect("テスト用の制御ストリーム設定に成功すること");
        server
            .set_webtransport_transport_verified(true, true)
            .expect("テスト用の transport parameter 検証に成功すること");
        let (server_ctrl, _) = server
            .take_stream_data(3)
            .expect("テスト用のサーバー SETTINGS 取得に成功すること");

        let client_wt = webtransport::Settings::new()
            .wt_enabled(vi(1))
            .wt_enabled_draft17(vi(1))
            .enable_webtransport_draft02(true)
            .wt_initial_max_data(vi(1024));
        let mut client = shiguredo_http3::ClientConnection::new(
            Settings::new().enable_webtransport_client(client_wt),
        );
        client
            .set_control_stream_id(2)
            .expect("テスト用の制御ストリーム設定に成功すること");
        client
            .set_webtransport_transport_verified(true, true)
            .expect("テスト用の transport parameter 検証に成功すること");
        let (client_ctrl, _) = client
            .take_stream_data(2)
            .expect("テスト用のクライアント SETTINGS 取得に成功すること");

        client
            .feed_stream(3, &server_ctrl, false)
            .expect("クライアントのサーバー SETTINGS 受信に成功すること");
        while let Some(_ev) = client
            .poll_event()
            .expect("テスト用のイベント取得に成功すること")
        {}
        server
            .feed_stream(2, &client_ctrl, false)
            .expect("サーバーのクライアント SETTINGS 受信に成功すること");
        while let Some(_ev) = server
            .poll_event()
            .expect("テスト用のイベント取得に成功すること")
        {}
        (server, client)
    }

    #[test]
    fn server_rejects_lower_version_token_when_new_codepoint_is_shared() {
        // 新旧両コードポイントを併送する接続では最も新しい版 (draft-17) が選ばれる。
        // `:protocol` は選択した版のトークン "webtransport-h3" でなければならず、
        // 旧 draft のトークン "webtransport" は受理しない
        // (draft-ietf-webtrans-http3-17 Section 7.1, 9.1)
        let (mut server, _client) = both_codepoints_pair();
        let headers = vec![
            Header::new(b":method", b"CONNECT").expect("テスト用のヘッダー作成に成功すること"),
            Header::new(b":protocol", b"webtransport")
                .expect("テスト用のヘッダー作成に成功すること"),
            Header::new(b":scheme", b"https").expect("テスト用のヘッダー作成に成功すること"),
            Header::new(b":authority", b"example.com")
                .expect("テスト用のヘッダー作成に成功すること"),
            Header::new(b":path", b"/wt").expect("テスト用のヘッダー作成に成功すること"),
        ];
        let frame = build_headers_frame(&headers);
        let result = server.feed_stream(0, &frame, false);
        assert!(
            matches!(result, Err(Error::StreamError(ErrorCode::MessageError))),
            "選択した版以外のトークンは受理しないこと: {result:?}"
        );
    }

    #[test]
    fn draft17_endpoints_establish_connect_session() {
        // 両端が新コードポイントを広告する構成で、`:protocol` = "webtransport-h3" の
        // CONNECT が受理されてセッションが確立する
        // (draft-ietf-webtrans-http3-17 Section 3.1, 7.1)
        let (mut server, mut client) = draft17_pair();
        let session_id = establish_draft17_session(&mut server, &mut client);
        assert!(
            server.wt_session_flow_control_enabled(session_id),
            "両端が SETTINGS_WT_INITIAL_MAX_* を広告するためフロー制御が有効になること"
        );
    }

    #[test]
    fn server_rejects_capsule_webtransport_token_on_draft17() {
        // 両端が新コードポイントを広告した接続でクライアントが "webtransport" を
        // 送ってきた場合、サーバーは MessageError でストリームを拒否する。
        // "webtransport" は capsule-based WebTransport (WEBTRANS-H2) のトークンであり、
        // 本文版のネイティブ実装では受け付けない
        // (draft-ietf-webtrans-http3-17 Section 2.1.2, 9.1)
        let (mut server, _client) = draft17_pair();
        let headers = vec![
            Header::new(b":method", b"CONNECT").expect("テスト用のヘッダー作成に成功すること"),
            Header::new(b":protocol", b"webtransport")
                .expect("テスト用のヘッダー作成に成功すること"),
            Header::new(b":scheme", b"https").expect("テスト用のヘッダー作成に成功すること"),
            Header::new(b":authority", b"example.com")
                .expect("テスト用のヘッダー作成に成功すること"),
            Header::new(b":path", b"/wt").expect("テスト用のヘッダー作成に成功すること"),
        ];
        let frame = build_headers_frame(&headers);
        let result = server.feed_stream(0, &frame, false);
        assert!(
            matches!(result, Err(Error::StreamError(ErrorCode::MessageError))),
            "draft-17 で旧 :protocol は拒否されること: {result:?}"
        );
    }

    /// draft-17 サーバーに、SETTINGS_WT_ENABLED を送らないクライアントの SETTINGS を
    /// 受信させた状態を作る
    ///
    /// 本文版だけに対応するクライアントは SETTINGS_WT_ENABLED を送らない
    /// (draft-ietf-webtrans-http3-17 Section 3.1)。サーバーの
    /// reset_stream_at 対応可否も引数で切り替える。
    fn draft17_server_with_client_without_codepoint(reset_stream_at: bool) -> ServerConnection {
        let mut server = ServerConnection::new(draft17_server_settings());
        server
            .set_control_stream_id(3)
            .expect("テスト用の制御ストリーム設定に成功すること");
        server
            .set_webtransport_transport_verified(true, reset_stream_at)
            .expect("テスト用の transport parameter 検証に成功すること");
        let _ = server
            .take_stream_data(3)
            .expect("テスト用のサーバー SETTINGS 取得に成功すること");

        let (client_ctrl, _) = {
            let mut client = shiguredo_http3::ClientConnection::new(
                draft17_client_settings_without_wt_enabled(),
            );
            client
                .set_control_stream_id(2)
                .expect("テスト用の制御ストリーム設定に成功すること");
            client
                .take_stream_data(2)
                .expect("テスト用のクライアント SETTINGS 取得に成功すること")
        };
        server
            .feed_stream(2, &client_ctrl, false)
            .expect("サーバーのクライアント SETTINGS 受信に成功すること");
        while let Some(_ev) = server
            .poll_event()
            .expect("テスト用のイベント取得に成功すること")
        {}

        let peer = server
            .peer_settings()
            .expect("ピア SETTINGS が保持されること");
        assert!(
            !peer.is_webtransport_enabled(),
            "本文版だけに対応するクライアントは SETTINGS_WT_ENABLED を送らないこと"
        );
        server
    }

    #[test]
    fn draft17_server_accepts_client_without_wt_codepoint() {
        // クライアントが SETTINGS_WT_ENABLED を送らない場合、サーバーは
        // 本文で定義された版を選び、その版の要件で CONNECT を検証する。
        // 本文版の要件を満たす接続は受理され、セッションが確立する
        // (draft-ietf-webtrans-http3-17 Section 3.1, 7.1)
        let mut server = draft17_server_with_client_without_codepoint(true);

        let headers = wt_connect_headers(DraftVersion::Draft17);
        let frame = build_headers_frame(&headers);
        server
            .feed_stream(0, &frame, false)
            .expect("クライアントの SETTINGS_WT_ENABLED がなくても CONNECT を受理すること");
        let response =
            vec![Header::new(b":status", b"200").expect("テスト用のヘッダー作成に成功すること")];
        server
            .send_response(0, &response, false)
            .expect("テスト用のレスポンス送信に成功すること");

        loop {
            match server
                .poll_event()
                .expect("SessionEstablished 待機中のイベント取得に成功すること")
            {
                Some(Event::WebTransport(WebTransportEvent::SessionEstablished {
                    session_id,
                    ..
                })) => {
                    assert_eq!(
                        session_id, 0,
                        "セッション ID は CONNECT ストリーム ID と一致すること"
                    );
                    return;
                }
                Some(_) => {}
                None => panic!("SessionEstablished イベントが発火していない"),
            }
        }
    }

    #[test]
    fn draft17_server_rejects_capsule_webtransport_token_without_codepoint() {
        // クライアントが SETTINGS_WT_ENABLED を送らない場合に本文版を選ぶと、
        // その版のトークンは "webtransport-h3" である。"webtransport" は
        // capsule-based WebTransport (WEBTRANS-H2) のトークンであり、
        // 本文版のネイティブ実装として受理してはならない
        // (draft-ietf-webtrans-http3-17 Section 2.1.2, 7.1, 9.1)
        let mut server = draft17_server_with_client_without_codepoint(true);

        let headers = vec![
            Header::new(b":method", b"CONNECT").expect("テスト用のヘッダー作成に成功すること"),
            Header::new(b":protocol", b"webtransport")
                .expect("テスト用のヘッダー作成に成功すること"),
            Header::new(b":scheme", b"https").expect("テスト用のヘッダー作成に成功すること"),
            Header::new(b":authority", b"example.com")
                .expect("テスト用のヘッダー作成に成功すること"),
            Header::new(b":path", b"/wt").expect("テスト用のヘッダー作成に成功すること"),
        ];
        let frame = build_headers_frame(&headers);
        let result = server.feed_stream(0, &frame, false);
        assert!(
            matches!(result, Err(Error::StreamError(ErrorCode::MessageError))),
            "capsule-based のトークンは受理しないこと: {result:?}"
        );
    }

    #[test]
    fn draft17_server_rejects_without_reset_stream_at_without_codepoint() {
        // クライアントが SETTINGS_WT_ENABLED を送らない場合に本文版を選ぶと、
        // reset_stream_at transport parameter の対応も要求される
        // (draft-ietf-webtrans-http3-17 Section 3.1, 7.1)
        let mut server = draft17_server_with_client_without_codepoint(false);

        let headers = wt_connect_headers(DraftVersion::Draft17);
        let frame = build_headers_frame(&headers);
        let result = server.feed_stream(0, &frame, false);
        assert!(
            matches!(result, Err(Error::StreamError(ErrorCode::MessageError))),
            "reset_stream_at 非対応なら拒否すること: {result:?}"
        );
    }
}

// =========================================================================
// 接続単位のドラフト版選択
// (draft-ietf-webtrans-http3-17 Section 3.1, 7.1)
//
// 完了条件の表の各行を、実際に生成した SETTINGS / CONNECT のバイト列を
// `ClientConnection` / `ServerConnection` に渡して検証する。
// =========================================================================

mod version_selection {
    use super::*;
    use shiguredo_http3::ClientConnection;
    use shiguredo_http3::webtransport::ConnectRequest;
    use shiguredo_http3::webtransport::Settings as WtSettings;
    use shiguredo_http3::webtransport::connect::ServerSettingsParams;

    /// 版の広告に使う既定パラメータ
    fn params() -> ServerSettingsParams {
        ServerSettingsParams::default()
    }

    /// 指定したドラフト版を広告するクライアント SETTINGS を構築する
    fn client_wt(versions: &[DraftVersion]) -> WtSettings {
        WtSettings::for_draft_versions(versions, params().max_sessions)
    }

    /// 指定したドラフト版を広告するサーバー接続を作成する
    fn make_server(versions: &[DraftVersion], transport_verified: bool) -> ServerConnection {
        make_server_with_wt(
            WtSettings::for_draft_versions(versions, params().max_sessions),
            transport_verified,
        )
    }

    /// 指定した WebTransport SETTINGS を広告するサーバー接続を作成する
    fn make_server_with_wt(wt: WtSettings, transport_verified: bool) -> ServerConnection {
        let mut server = ServerConnection::new(Settings::new().enable_webtransport_server(wt));
        server
            .set_control_stream_id(3)
            .expect("テスト用の制御ストリーム設定に成功すること");
        if transport_verified {
            server
                .set_webtransport_transport_verified(true, true)
                .expect("テスト用の transport parameter 検証に成功すること");
        }
        server
    }

    /// 指定した WebTransport SETTINGS を広告するクライアント接続を作成する
    fn make_client(wt: WtSettings, transport_verified: bool) -> ClientConnection {
        let mut client = ClientConnection::new(Settings::new().enable_webtransport_client(wt));
        client
            .set_control_stream_id(2)
            .expect("テスト用の制御ストリーム設定に成功すること");
        if transport_verified {
            client
                .set_webtransport_transport_verified(true, true)
                .expect("テスト用の transport parameter 検証に成功すること");
        }
        client
    }

    /// クライアントの制御ストリーム (SETTINGS) を取り出す
    fn take_client_control(client: &mut ClientConnection) -> Vec<u8> {
        client
            .take_stream_data(2)
            .expect("テスト用のクライアント SETTINGS 取得に成功すること")
            .0
    }

    /// サーバーの制御ストリーム (SETTINGS) を取り出す
    fn take_server_control(server: &mut ServerConnection) -> Vec<u8> {
        server
            .take_stream_data(3)
            .expect("テスト用のサーバー SETTINGS 取得に成功すること")
            .0
    }

    /// サーバーとクライアントの SETTINGS を交換する
    fn exchange_settings(server: &mut ServerConnection, client: &mut ClientConnection) {
        let client_ctrl = take_client_control(client);
        let server_ctrl = take_server_control(server);
        server
            .feed_stream(2, &client_ctrl, false)
            .expect("サーバーのクライアント SETTINGS 受信に成功すること");
        client
            .feed_stream(3, &server_ctrl, false)
            .expect("クライアントのサーバー SETTINGS 受信に成功すること");
        server
            .drain_events()
            .expect("テスト用のサーバーイベント取得に成功すること");
        client
            .drain_events()
            .expect("テスト用のクライアントイベント取得に成功すること");
    }

    /// 指定した HTTP/3 設定を持つクライアントの制御ストリームを作る
    fn control_stream_with(settings: Settings) -> Vec<u8> {
        let mut client = ClientConnection::new(settings);
        client
            .set_control_stream_id(2)
            .expect("テスト用の制御ストリーム設定に成功すること");
        client
            .take_stream_data(2)
            .expect("テスト用のクライアント SETTINGS 取得に成功すること")
            .0
    }

    /// 指定した版のトークンで CONNECT を構築し、サーバーに流し込む
    fn feed_connect_to_server(
        server: &mut ServerConnection,
        stream_id: u64,
        draft: DraftVersion,
    ) -> Result<(), Error> {
        let headers = ConnectRequest::new("https", "example.com", "/wt")
            .draft_version(draft)
            .to_headers()
            .expect("テスト用の CONNECT ヘッダー作成に成功すること");
        let frame = build_headers_frame(&headers);
        server.feed_stream(stream_id, &frame, false)
    }

    /// クライアントが選択結果に基づいて生成した CONNECT をサーバーに流し込む
    fn send_connect_via_client(
        server: &mut ServerConnection,
        client: &mut ClientConnection,
        draft: DraftVersion,
    ) -> Result<u64, Error> {
        let headers = ConnectRequest::new("https", "example.com", "/wt")
            .draft_version(draft)
            .to_headers()
            .expect("テスト用の CONNECT ヘッダー作成に成功すること");
        let stream_id = client.send_request(&headers, false)?;
        let (request, _) = client
            .take_stream_data(stream_id)
            .expect("テスト用の CONNECT ヘッダー取得に成功すること");
        server.feed_stream(stream_id, &request, false)?;
        Ok(stream_id)
    }

    /// サーバーが WT CONNECT を受理して 200 応答を返す
    fn accept_connect(server: &mut ServerConnection, stream_id: u64) {
        let response =
            vec![Header::new(b":status", b"200").expect("テスト用のヘッダー作成に成功すること")];
        server
            .send_response(stream_id, &response, false)
            .expect("テスト用のレスポンス送信に成功すること");
    }

    /// クライアントから見たセッションを確立する
    ///
    /// CONNECT の送信、サーバーの受理、200 応答の受信まで進める。
    /// サーバー側で発生したイベントも併せて返す (保留していたストリームや
    /// datagram の配送はセッション確立時に起きるため)。
    fn establish_session(
        server: &mut ServerConnection,
        client: &mut ClientConnection,
        draft: DraftVersion,
    ) -> (u64, Vec<Event>) {
        let stream_id = send_connect_via_client(server, client, draft)
            .expect("テスト用の WT CONNECT が受理されること");
        accept_connect(server, stream_id);
        let (response, _) = server
            .take_stream_data(stream_id)
            .expect("テスト用の 200 応答取得に成功すること");
        client
            .feed_stream(stream_id, &response, false)
            .expect("テスト用の 200 応答受信に成功すること");
        let server_events = server
            .drain_events()
            .expect("テスト用のサーバーイベント取得に成功すること");
        client
            .drain_events()
            .expect("テスト用のクライアントイベント取得に成功すること");
        (stream_id, server_events)
    }

    #[test]
    fn server_rejects_old_only_client_when_only_new_codepoints_exist() {
        // サーバーが SETTINGS_WT_ENABLED (0x2c7cf000) だけを広告し、クライアントが
        // 旧 draft のコードポイント (draft-02) だけを広告する場合、共通する版が
        // ないため CONNECT を受理しない (draft-ietf-webtrans-http3-17 Section 7.1)。
        // SETTINGS_WT_ENABLED を広告するサーバーはコードポイント交渉を行うため、
        // 旧 draft のクライアントは交差を作れない
        let mut server = make_server(&[DraftVersion::Draft15], true);
        let mut client = make_client(client_wt(&[DraftVersion::Draft02]), true);
        exchange_settings(&mut server, &mut client);

        assert_eq!(
            server.negotiated_wt_draft_version(),
            None,
            "共通する版がないこと"
        );
        let result = feed_connect_to_server(&mut server, 0, DraftVersion::Draft02);
        assert!(
            matches!(result, Err(Error::StreamError(ErrorCode::MessageError))),
            "共通する版がない場合は CONNECT を受理しないこと: {result:?}"
        );
    }

    #[test]
    fn server_selects_document_version_without_client_advertisement() {
        // 表の 1 行目: 本文版のみのサーバー + WT 広告なしのクライアント → 本文版
        let mut server = make_server(&[DraftVersion::Draft17], true);
        // 本文版だけに対応するクライアントは SETTINGS_WT_ENABLED を送らない
        // (draft-ietf-webtrans-http3-17 Section 7.1)。テストでは wt_settings を
        // 空でない値にするため SETTINGS_WT_INITIAL_MAX_DATA だけを設定する
        // (フロー制御の初期値: 同 Section 5.1)
        let mut client = make_client(WtSettings::new().wt_initial_max_data(vi(1024)), true);

        exchange_settings(&mut server, &mut client);
        assert_eq!(
            server.negotiated_wt_draft_version(),
            Some(DraftVersion::Draft17),
            "WT 広告のないクライアントには本文版を選ぶこと"
        );
        // 本文版だけに対応するクライアントは、ピアが本文版を広告していれば
        // 本文版を選んで CONNECT を開始できる
        assert_eq!(
            client.negotiated_wt_draft_version(),
            Some(DraftVersion::Draft17),
            "本文版だけのクライアントは本文版を選ぶこと"
        );
        let session_start =
            send_connect_via_client(&mut server, &mut client, DraftVersion::Draft17)
                .expect("本文版だけのクライアントの CONNECT が受理されること");
        assert_eq!(session_start, 0, "CONNECT は stream 0 で送られること");
    }

    #[test]
    fn document_version_only_client_receives_server_initiated_streams() {
        // 本文版だけに対応するクライアント (SETTINGS_WT_ENABLED を送らない) でも、
        // セッション確立後のサーバー開始 bidi / uni / datagram を受信できる
        // (draft-ietf-webtrans-http3-17 Section 4.3, 4.5, 4.6)
        let mut server = make_server(&[DraftVersion::Draft17], true);
        let mut client = make_client(WtSettings::new().wt_initial_max_data(vi(1024)), true);
        exchange_settings(&mut server, &mut client);
        let (session_id, _events) =
            establish_session(&mut server, &mut client, DraftVersion::Draft17);
        assert_eq!(session_id, 0);

        // サーバー開始 bidi (stream 1) の 0x41 bidi
        client
            .feed_stream(1, &[0x40, 0x41, 0x00], false)
            .expect("本文版だけのクライアントでもサーバー開始 bidi を受理すること");
        // サーバー開始 uni (stream 7。stream 3 はサーバーの制御ストリーム) の 0x54
        client
            .feed_stream(7, &[0x40, 0x54, 0x00], false)
            .expect("本文版だけのクライアントでもサーバー開始 uni を受理すること");
        // datagram (Quarter Stream ID = 0)
        client
            .feed_datagram(&[0x00, 0x01, 0x02])
            .expect("本文版だけのクライアントでも datagram を受理すること");

        let events = client
            .drain_events()
            .expect("テスト用のクライアントイベント取得に成功すること");
        assert!(
            events.iter().any(|e| matches!(
                e,
                Event::WebTransport(WebTransportEvent::UniStreamOpen { stream_id: 7, .. })
            )),
            "サーバー開始 uni ストリームが開くこと: {events:?}"
        );
        assert!(
            events.iter().any(|e| matches!(
                e,
                Event::WebTransport(WebTransportEvent::Datagram { session_id: 0, .. })
            )),
            "datagram が配送されること: {events:?}"
        );
    }

    #[test]
    fn server_selects_document_version_when_both_codepoints_are_advertised() {
        // 表の 2 行目: 本文版と旧 draft + クライアントが新 ID と旧 ID → 本文版
        let drafts = [DraftVersion::Draft15, DraftVersion::Draft17];
        let mut server = make_server(&drafts, true);
        let mut client = make_client(client_wt(&drafts), true);

        exchange_settings(&mut server, &mut client);
        assert_eq!(
            server.negotiated_wt_draft_version(),
            Some(DraftVersion::Draft17),
            "新 ID を広告するクライアントには本文版を選ぶこと"
        );
        assert_eq!(
            client.negotiated_wt_draft_version(),
            Some(DraftVersion::Draft17)
        );

        // クライアントが選択結果に基づいて生成した CONNECT が受理される
        send_connect_via_client(&mut server, &mut client, DraftVersion::Draft17)
            .expect("本文版の CONNECT が受理されること");

        // 選択版以外のトークンは受理しない
        assert!(
            matches!(
                feed_connect_to_server(&mut server, 4, DraftVersion::Draft02),
                Err(Error::StreamError(ErrorCode::MessageError))
            ),
            "選択版以外の :protocol トークンは拒否されること"
        );
    }

    #[test]
    fn server_selects_highest_common_old_draft_when_client_sends_only_old_id() {
        // 表の 3 行目: 本文版と旧 draft + クライアントは対応する旧 ID のみ
        let mut server = make_server(&[DraftVersion::Draft15, DraftVersion::Draft17], true);
        let mut client = make_client(client_wt(&[DraftVersion::Draft15]), true);

        exchange_settings(&mut server, &mut client);
        assert_eq!(
            server.negotiated_wt_draft_version(),
            Some(DraftVersion::Draft15),
            "対応する旧 ID だけなら共通する旧 draft の最上位を選ぶこと"
        );
        assert_eq!(
            client.negotiated_wt_draft_version(),
            Some(DraftVersion::Draft15)
        );
        send_connect_via_client(&mut server, &mut client, DraftVersion::Draft15)
            .expect("旧 draft の CONNECT が受理されること");
    }

    #[test]
    fn server_selects_highest_common_old_draft_among_multiple() {
        // 旧 draft が複数共通する場合は優先順位の最も高い版を選ぶ。
        // draft-07 と draft-14 は Safari 互換のため draft-07 を優先する
        let drafts = [
            DraftVersion::Draft07,
            DraftVersion::Draft14,
            DraftVersion::Draft15,
        ];
        let mut server = make_server(&drafts, true);
        let mut client = make_client(client_wt(&drafts), true);
        exchange_settings(&mut server, &mut client);
        assert_eq!(
            server.negotiated_wt_draft_version(),
            Some(DraftVersion::Draft15),
            "旧 draft のうち最も優先順位の高い版を選ぶこと"
        );

        // draft-07 と draft-14 だけなら Safari 互換で draft-07 を選ぶ
        let legacy = [DraftVersion::Draft07, DraftVersion::Draft14];
        let mut server = make_server(&legacy, true);
        let mut client = make_client(client_wt(&legacy), true);
        exchange_settings(&mut server, &mut client);
        assert_eq!(
            server.negotiated_wt_draft_version(),
            Some(DraftVersion::Draft07),
            "draft-07 と draft-14 の併送は draft-07 を優先すること"
        );
    }

    #[test]
    fn server_selects_document_version_when_client_has_no_supported_old_id() {
        // 表の 4 行目: 本文版と旧 draft + クライアントは対応しない旧 ID のみ
        let mut server = make_server(&[DraftVersion::Draft15, DraftVersion::Draft17], true);
        let mut client = make_client(client_wt(&[DraftVersion::Draft02]), true);

        exchange_settings(&mut server, &mut client);
        assert_eq!(
            server.negotiated_wt_draft_version(),
            Some(DraftVersion::Draft17),
            "対応する旧 ID がない場合は本文版を選ぶこと"
        );
        assert_eq!(
            client.negotiated_wt_draft_version(),
            None,
            "交差する版がないクライアントは版を選ばないこと"
        );
        let headers = ConnectRequest::new("https", "example.com", "/wt")
            .draft_version(DraftVersion::Draft17)
            .to_headers()
            .expect("テスト用の CONNECT ヘッダー作成に成功すること");
        let err = client.send_request(&headers, false).unwrap_err();
        assert!(
            matches!(err, Error::WtSetup(WtSetupError::UnknownDraftVersion)),
            "交差する版がないクライアントは CONNECT を開始しないこと: {err:?}"
        );

        // サーバーは実際に生成した CONNECT バイト列を本文版として受理する
        feed_connect_to_server(&mut server, 0, DraftVersion::Draft17)
            .expect("本文版の CONNECT が受理されること");
    }

    #[test]
    fn client_does_not_select_version_it_does_not_support() {
        // サーバーは本文版と draft-02 を広告するが、クライアントは draft-02 だけに
        // 対応する。クライアントはサーバーの広告だけから本文版を選ばない
        // (draft-ietf-webtrans-http3-17 Section 7.1)。
        let mut server = make_server(&[DraftVersion::Draft02, DraftVersion::Draft17], true);
        let mut client = make_client(client_wt(&[DraftVersion::Draft02]), true);
        exchange_settings(&mut server, &mut client);

        assert_eq!(
            client.negotiated_wt_draft_version(),
            Some(DraftVersion::Draft02),
            "クライアントは自分が対応する版だけを選ぶこと"
        );
        assert_eq!(
            server.negotiated_wt_draft_version(),
            Some(DraftVersion::Draft02),
            "サーバーも共通する旧 draft を選ぶこと"
        );
        send_connect_via_client(&mut server, &mut client, DraftVersion::Draft02)
            .expect("draft-02 の CONNECT が受理されること");
    }

    #[test]
    fn legacy_draft_connections_keep_their_selected_version() {
        // draft-02 / 07 / 14 / 15 (16 は 15 と同じコードポイント) の接続が
        // 従来どおり成立することを、選択結果と実際の CONNECT で確認する
        for expected in [
            DraftVersion::Draft02,
            DraftVersion::Draft07,
            DraftVersion::Draft14,
            DraftVersion::Draft15,
        ] {
            let mut server = make_server(&[expected], true);
            let mut client = make_client(client_wt(&[expected]), true);
            exchange_settings(&mut server, &mut client);
            assert_eq!(
                server.negotiated_wt_draft_version(),
                Some(expected),
                "{expected:?} の接続で従来の共通版が選ばれること"
            );
            assert_eq!(
                client.negotiated_wt_draft_version(),
                Some(expected),
                "{expected:?} の接続でクライアントも同じ版を選ぶこと"
            );
            send_connect_via_client(&mut server, &mut client, expected)
                .expect("従来の draft の CONNECT が受理されること");
        }
    }

    #[test]
    fn client_does_not_start_connect_without_common_version() {
        // サーバーは draft-15 まで、クライアントは draft-17 だけに対応する
        let mut server = make_server(&[DraftVersion::Draft15], true);
        let mut client = make_client(client_wt(&[DraftVersion::Draft17]), true);
        exchange_settings(&mut server, &mut client);

        assert_eq!(
            server.negotiated_wt_draft_version(),
            None,
            "本文版に対応しないサーバーは版を選ばないこと"
        );
        assert_eq!(client.negotiated_wt_draft_version(), None);

        let headers = ConnectRequest::new("https", "example.com", "/wt")
            .draft_version(DraftVersion::Draft17)
            .to_headers()
            .expect("テスト用の CONNECT ヘッダー作成に成功すること");
        let err = client.send_request(&headers, false).unwrap_err();
        assert!(
            matches!(err, Error::WtSetup(WtSetupError::UnknownDraftVersion)),
            "共通する版がないクライアントは CONNECT を開始しないこと: {err:?}"
        );

        // サーバーも同一の CONNECT バイト列を受理しない
        assert!(
            matches!(
                feed_connect_to_server(&mut server, 0, DraftVersion::Draft17),
                Err(Error::StreamError(ErrorCode::MessageError))
            ),
            "共通する版がないサーバーは CONNECT を受理しないこと"
        );
    }

    #[test]
    fn connect_is_deferred_until_settings_and_then_rejected_without_wt_advertisement() {
        // SETTINGS 未着の CONNECT は保留され、エラーにならない。
        // 受信済みの SETTINGS に WT 広告がない場合は保留した CONNECT を拒否する。
        // 両者を混同しないことを確認する (本文版に対応しないサーバーで確認する)。
        let mut server = make_server(&[DraftVersion::Draft15], true);
        feed_connect_to_server(&mut server, 0, DraftVersion::Draft17)
            .expect("SETTINGS 未着の CONNECT は保留されること");
        assert_eq!(
            server.negotiated_wt_draft_version(),
            None,
            "SETTINGS 未着では版を選択しないこと"
        );
        let events = server
            .drain_events()
            .expect("テスト用のサーバーイベント取得に成功すること");
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, Event::HeadersBegin { stream_id: 0 })),
            "保留中の CONNECT はヘッダーイベントを発火しないこと"
        );

        // WebTransport の広告を含まない SETTINGS を流すと、保留分の再検証で拒否される
        let control = control_stream_with(Settings::new().h3_datagram(true));
        let result = server.feed_stream(2, &control, false);
        assert!(
            matches!(result, Err(Error::StreamError(ErrorCode::MessageError))),
            "WT 広告のない SETTINGS 受信で保留中の CONNECT を拒否すること: {result:?}"
        );
    }

    #[test]
    fn connect_is_rejected_immediately_with_received_settings_without_wt_advertisement() {
        // SETTINGS 受信済みで WT 広告がない場合、CONNECT は保留せず即拒否する
        // (本文版に対応しないサーバーで確認する)
        let mut server = make_server(&[DraftVersion::Draft15], true);
        let control = control_stream_with(Settings::new().h3_datagram(true));
        server
            .feed_stream(2, &control, false)
            .expect("WT 非対応の SETTINGS 受信自体は成功すること");
        assert_eq!(
            server.negotiated_wt_draft_version(),
            None,
            "WT 広告がないため版を選ばないこと"
        );

        let result = feed_connect_to_server(&mut server, 0, DraftVersion::Draft17);
        assert!(
            matches!(result, Err(Error::StreamError(ErrorCode::MessageError))),
            "SETTINGS に WT 広告がない CONNECT は即拒否すること: {result:?}"
        );
    }

    #[test]
    fn connect_is_rejected_when_h3_datagram_is_missing() {
        // 必須設定 (SETTINGS_H3_DATAGRAM) の不足は WT 広告の有無とは別の拒否理由
        let mut server = make_server(&[DraftVersion::Draft15, DraftVersion::Draft17], true);
        let wt = client_wt(&[DraftVersion::Draft15, DraftVersion::Draft17]);
        let client_settings = Settings {
            h3_datagram: Some(false),
            ..Settings::new().enable_webtransport_client(wt)
        };
        let control = control_stream_with(client_settings);
        server
            .feed_stream(2, &control, false)
            .expect("テスト用の SETTINGS 受信に成功すること");
        assert_eq!(
            server.negotiated_wt_draft_version(),
            Some(DraftVersion::Draft17),
            "版の選択自体は成立すること"
        );

        let result = feed_connect_to_server(&mut server, 0, DraftVersion::Draft17);
        assert!(
            matches!(result, Err(Error::StreamError(ErrorCode::MessageError))),
            "H3_DATAGRAM 不足の CONNECT は拒否すること: {result:?}"
        );
    }

    #[test]
    fn connect_is_rejected_when_transport_parameters_are_not_verified() {
        // 必須 transport parameter の不足も WT 広告の有無とは別の拒否理由
        let mut server = make_server(&[DraftVersion::Draft15, DraftVersion::Draft17], false);
        let mut client = make_client(
            client_wt(&[DraftVersion::Draft15, DraftVersion::Draft17]),
            true,
        );
        exchange_settings(&mut server, &mut client);
        assert_eq!(
            server.negotiated_wt_draft_version(),
            Some(DraftVersion::Draft17),
            "版の選択自体は成立すること"
        );

        let result = feed_connect_to_server(&mut server, 0, DraftVersion::Draft17);
        assert!(
            matches!(result, Err(Error::StreamError(ErrorCode::MessageError))),
            "transport parameter 未検証の CONNECT は拒否すること: {result:?}"
        );
    }

    #[test]
    fn pre_negotiation_bidi_stream_is_dispatched_with_common_version() {
        // SETTINGS 前の 0x41 bidi は保留され、版選択の確定後に WT 経路へ流れる
        let drafts = [DraftVersion::Draft15, DraftVersion::Draft17];
        let mut server = make_server(&drafts, true);
        let mut client = make_client(client_wt(&drafts), true);

        // 0x41 + session_id 0 の bidi ストリーム (stream 8)
        server
            .feed_stream(8, &[0x40, 0x41, 0x00], false)
            .expect("SETTINGS 前の 0x41 bidi は保留されること");
        exchange_settings(&mut server, &mut client);

        let (session_id, events) =
            establish_session(&mut server, &mut client, DraftVersion::Draft17);
        assert_eq!(session_id, 0);
        assert!(
            events.iter().any(|e| matches!(
                e,
                Event::WebTransport(WebTransportEvent::BidiStreamOpen {
                    stream_id: 8,
                    session_id: 0,
                })
            )),
            "SETTINGS 受信後に保留した bidi ストリームが開くこと: {events:?}"
        );
    }

    #[test]
    fn pre_negotiation_bidi_stream_is_rejected_without_common_version() {
        // 共通する版がない場合、SETTINGS 受信時に保留した 0x41 bidi を拒否する
        let mut server = make_server(&[DraftVersion::Draft15], true);
        let mut client = make_client(client_wt(&[DraftVersion::Draft17]), true);

        server
            .feed_stream(8, &[0x40, 0x41, 0x00], false)
            .expect("SETTINGS 前の 0x41 bidi は保留されること");

        // 拒否イベントは SETTINGS 受信時に発火するため、ここでは
        // ヘルパーを使わずに SETTINGS を流し込んでイベントを観測する
        let client_ctrl = take_client_control(&mut client);
        let server_ctrl = take_server_control(&mut server);
        server
            .feed_stream(2, &client_ctrl, false)
            .expect("サーバーのクライアント SETTINGS 受信に成功すること");
        client
            .feed_stream(3, &server_ctrl, false)
            .expect("クライアントのサーバー SETTINGS 受信に成功すること");
        let events = server
            .drain_events()
            .expect("テスト用のサーバーイベント取得に成功すること");
        assert!(
            events.iter().any(|e| matches!(
                e,
                Event::WebTransport(WebTransportEvent::BufferedStreamRejected { stream_id: 8, .. })
            )),
            "共通する版がない場合は保留した bidi ストリームを拒否すること: {events:?}"
        );
        assert!(
            !events.iter().any(|e| matches!(
                e,
                Event::WebTransport(WebTransportEvent::BidiStreamOpen { stream_id: 8, .. })
            )),
            "拒否した bidi ストリームを WT 経路へ流さないこと: {events:?}"
        );
    }

    #[test]
    fn uni_stream_is_rejected_before_settings_and_accepted_after_negotiation() {
        // SETTINGS 前の 0x54 uni はストリームエラー。版選択の確定後は受理される
        let drafts = [DraftVersion::Draft15, DraftVersion::Draft17];
        let mut server = make_server(&drafts, true);
        let mut client = make_client(client_wt(&drafts), true);

        let err = server
            .feed_stream(6, &[0x40, 0x54, 0x00], false)
            .unwrap_err();
        assert!(
            matches!(err, Error::StreamError(ErrorCode::StreamCreationError)),
            "SETTINGS 前の 0x54 はストリームエラーになること: {err:?}"
        );

        exchange_settings(&mut server, &mut client);
        // CONNECT 受理前に届いた 0x54 は Pending セッションでバッファされる
        server
            .feed_stream(6, &[0x40, 0x54, 0x00], false)
            .expect("版選択の確定後は 0x54 を受理すること");

        let (session_id, events) =
            establish_session(&mut server, &mut client, DraftVersion::Draft17);
        assert_eq!(session_id, 0);
        assert!(
            events.iter().any(|e| matches!(
                e,
                Event::WebTransport(WebTransportEvent::UniStreamOpen {
                    stream_id: 6,
                    session_id: 0,
                })
            )),
            "セッション確立後にバッファした uni ストリームが開くこと: {events:?}"
        );
    }

    #[test]
    fn uni_stream_is_rejected_when_no_common_version() {
        // 共通する版がない場合、SETTINGS 後も 0x54 を受理しない
        let mut server = make_server(&[DraftVersion::Draft15], true);
        let mut client = make_client(client_wt(&[DraftVersion::Draft17]), true);
        exchange_settings(&mut server, &mut client);

        let err = server
            .feed_stream(6, &[0x40, 0x54, 0x00], false)
            .unwrap_err();
        assert!(
            matches!(err, Error::StreamError(ErrorCode::StreamCreationError)),
            "共通する版がない場合は 0x54 を拒否すること: {err:?}"
        );
    }

    #[test]
    fn datagram_is_dropped_before_settings_and_delivered_after_negotiation() {
        // SETTINGS 前の datagram は静かに破棄し、版選択の確定後は受理する
        let drafts = [DraftVersion::Draft15, DraftVersion::Draft17];
        let mut server = make_server(&drafts, true);
        let mut client = make_client(client_wt(&drafts), true);

        let datagram = webtransport::Datagram::new(0, b"hello".to_vec())
            .expect("テスト用の datagram 作成に成功すること");
        let mut buf = Vec::new();
        datagram.encode(&mut buf);

        server
            .feed_datagram(&buf)
            .expect("SETTINGS 前の datagram は破棄されること");
        let events = server
            .drain_events()
            .expect("テスト用のサーバーイベント取得に成功すること");
        assert!(
            events.is_empty(),
            "SETTINGS 前の datagram はイベントを発火しないこと: {events:?}"
        );

        exchange_settings(&mut server, &mut client);
        server
            .feed_datagram(&buf)
            .expect("版選択の確定後は datagram を受理すること");
        let (session_id, events) =
            establish_session(&mut server, &mut client, DraftVersion::Draft17);
        assert_eq!(session_id, 0);
        assert!(
            events.iter().any(|e| matches!(
                e,
                Event::WebTransport(WebTransportEvent::Datagram {
                    session_id: 0,
                    payload,
                }) if payload == b"hello"
            )),
            "セッション確立後にバッファした datagram が配送されること: {events:?}"
        );
    }

    #[test]
    fn datagram_is_dropped_when_no_common_version() {
        // 共通する版がない場合は datagram を受理しない
        let mut server = make_server(&[DraftVersion::Draft15], true);
        let mut client = make_client(client_wt(&[DraftVersion::Draft17]), true);
        exchange_settings(&mut server, &mut client);

        let datagram = webtransport::Datagram::new(0, b"hello".to_vec())
            .expect("テスト用の datagram 作成に成功すること");
        let mut buf = Vec::new();
        datagram.encode(&mut buf);
        server
            .feed_datagram(&buf)
            .expect("共通する版がない datagram は破棄されること");
        let events = server
            .drain_events()
            .expect("テスト用のサーバーイベント取得に成功すること");
        assert!(
            events.is_empty(),
            "共通する版がない datagram はイベントを発火しないこと: {events:?}"
        );
    }

    #[test]
    fn client_accepts_server_bidi_stream_with_common_version() {
        // クライアント側も選択結果に基づいてサーバー開始 bidi を受理する
        let drafts = [DraftVersion::Draft15];
        let mut server = make_server(&drafts, true);
        let mut client = make_client(client_wt(&drafts), true);
        exchange_settings(&mut server, &mut client);
        let (session_id, _events) =
            establish_session(&mut server, &mut client, DraftVersion::Draft15);
        assert_eq!(session_id, 0);

        // サーバー開始 bidi (stream 1) の 0x41 bidi
        client
            .feed_stream(1, &[0x40, 0x41, 0x00], false)
            .expect("版選択が確定していればサーバー開始 bidi を受理すること");
        let events = client
            .drain_events()
            .expect("テスト用のクライアントイベント取得に成功すること");
        assert!(
            events.iter().any(|e| matches!(
                e,
                Event::WebTransport(WebTransportEvent::BidiStreamOpen {
                    stream_id: 1,
                    session_id: 0,
                })
            )),
            "サーバー開始 bidi ストリームが開くこと: {events:?}"
        );
    }

    #[test]
    fn client_rejects_server_bidi_stream_without_common_version() {
        // 共通する版がない場合、クライアントはサーバー開始 bidi を受理しない
        let mut server = make_server(&[DraftVersion::Draft15], true);
        let mut client = make_client(client_wt(&[DraftVersion::Draft17]), true);
        exchange_settings(&mut server, &mut client);
        assert_eq!(client.negotiated_wt_draft_version(), None);

        let err = client
            .feed_stream(1, &[0x40, 0x41, 0x00], false)
            .unwrap_err();
        assert!(
            matches!(err, Error::ConnectionError(ErrorCode::StreamCreationError)),
            "共通する版がないクライアントはサーバー開始 bidi を拒否すること: {err:?}"
        );
    }

    // =========================================================================
    // 0-RTT 再開時の前回 WT SETTINGS の扱い
    // (draft-ietf-webtrans-http3-16 Section 3.2,
    //  draft-ietf-webtrans-http3-17 Section 3.2)
    // =========================================================================

    /// 比較用の初期上限を持つ前回接続の draft-17 の WT SETTINGS
    fn previous_wt_draft17() -> WtSettings {
        WtSettings::new()
            .wt_enabled_draft17(vi(1))
            .wt_initial_max_streams_uni(vi(100))
            .wt_initial_max_streams_bidi(vi(50))
            .wt_initial_max_data(vi(1_048_576))
    }

    #[test]
    fn previous_wt_settings_do_not_enable_webtransport_for_connect() {
        // 前回接続の SETTINGS に WT 有効化が含まれていても、現在接続のサーバー
        // SETTINGS に WT 広告がなければ CONNECT を開始できない
        let previous = WtSettings::new()
            .wt_enabled(vi(1))
            .wt_enabled_draft17(vi(1))
            .wt_initial_max_data(vi(1_048_576));
        // draft-17 に対応する端点は前回接続の WT SETTINGS を破棄する
        let mut client = make_client(client_wt(&[DraftVersion::Draft17]), true);
        client.set_previous_wt_settings(previous);

        // 現在のサーバー SETTINGS は WT の広告を含まない
        let server_ctrl = control_stream_with(Settings::new().h3_datagram(true));
        client
            .feed_stream(3, &server_ctrl, false)
            .expect("テスト用のサーバー SETTINGS 受信に成功すること");
        client
            .drain_events()
            .expect("テスト用のクライアントイベント取得に成功すること");

        let headers = ConnectRequest::new("https", "example.com", "/wt")
            .draft_version(DraftVersion::Draft17)
            .to_headers()
            .expect("テスト用の CONNECT ヘッダー作成に成功すること");
        let err = client.send_request(&headers, false).unwrap_err();
        assert!(
            matches!(
                err,
                Error::WtSetup(shiguredo_http3::WtSetupError::WebTransportNotEnabled)
            ),
            "前回値があっても WT が広告されていなければ CONNECT を開始しないこと: {err:?}"
        );
    }

    /// draft-17 の再開接続で、現在の SETTINGS が前回値と無関係に受理されることを確認する
    ///
    /// SETTINGS の交換だけでなく、実際に WebTransport セッションを確立できることまで
    /// 確認する。現在の SETTINGS の値だけで版選択とセッション確立が進む。
    fn assert_draft17_resumption_ok(previous: WtSettings, current: WtSettings) {
        let mut server = make_server_with_wt(current, true);
        let mut client = make_client(client_wt(&[DraftVersion::Draft17]), true);
        // 前回接続の SETTINGS を注入してから、現在接続の SETTINGS を受信させる
        client.set_previous_wt_settings(previous);
        exchange_settings(&mut server, &mut client);

        assert_eq!(
            server.negotiated_wt_draft_version(),
            Some(DraftVersion::Draft17),
            "サーバーも新コードポイントの交差で draft-17 を選ぶこと"
        );
        assert_eq!(
            client.negotiated_wt_draft_version(),
            Some(DraftVersion::Draft17),
            "クライアントは前回値ではなく現在の SETTINGS で draft-17 を選ぶこと"
        );
        let (session_id, _events) =
            establish_session(&mut server, &mut client, DraftVersion::Draft17);
        assert_eq!(
            session_id, 0,
            "現在の SETTINGS で WebTransport セッションを確立できること"
        );
    }

    #[test]
    fn draft17_resumption_accepts_decreased_initial_limits() {
        // draft-17 では前回接続の WT SETTINGS を引き継がず、現在の SETTINGS の
        // 初期上限だけを使う。前回値より小さい値でも H3_SETTINGS_ERROR にしない
        // (draft-ietf-webtrans-http3-17 Section 3.2)。
        let current = WtSettings::new()
            .wt_enabled_draft17(vi(1))
            .wt_initial_max_streams_uni(vi(50))
            .wt_initial_max_streams_bidi(vi(25))
            .wt_initial_max_data(vi(512_000));
        assert_draft17_resumption_ok(previous_wt_draft17(), current);
    }
}
