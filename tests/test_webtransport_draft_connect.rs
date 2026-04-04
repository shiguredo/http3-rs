//! WebTransport CONNECT の draft バージョン別バリデーションテスト
//!
//! サーバーが各 draft (02/07/14/15) クライアントからの CONNECT を正しく受理/拒否するか検証。
//!
//! 特に SETTINGS_ENABLE_CONNECT_PROTOCOL (0x08) の要否がドラフトバージョンによって異なる:
//! - draft-02: 不要 (SETTINGS_ENABLE_WEBTRANSPORT_DRAFT02 が暗示)
//! - draft-07: 不要 (クライアントは SETTINGS_WEBTRANSPORT_MAX_SESSIONS のみ MUST,
//!   draft-ietf-webtrans-http3-07 Section 3.2)
//! - draft-14: 必須 (RFC 9220)
//! - draft-15: 必須 (RFC 9220)

use shiguredo_http3::qpack::Encoder;
use shiguredo_http3::webtransport::DraftVersion;
use shiguredo_http3::{Error, ErrorCode, Event, Header, ServerConnection, Settings, webtransport};

// =========================================================================
// ヘルパー関数
// =========================================================================

/// QPACK エンコードされた HEADERS フレームを手動構築する
fn build_headers_frame(headers: &[Header]) -> Vec<u8> {
    let encoder = Encoder::new();
    let mut qpack_buf = vec![0u8; 4096];
    let qpack_len = encoder.encode(&mut qpack_buf, headers).unwrap();
    qpack_buf.truncate(qpack_len);

    let mut frame = Vec::new();
    // HEADERS フレームタイプ: 0x01
    shiguredo_http3::varint::encode_into_vec(&mut frame, 0x01);
    // ペイロード長
    shiguredo_http3::varint::encode_into_vec(&mut frame, qpack_len as u64);
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
    client.set_control_stream_id(2).unwrap();
    let (ctrl, _) = client.take_stream_data(2).unwrap();
    ctrl
}

/// draft-07 クライアントの SETTINGS を持つ Connection を作成し、
/// 制御ストリームデータを返す
/// Safari と同じパターン: ENABLE_CONNECT_PROTOCOL なし
fn build_draft07_client_ctrl() -> Vec<u8> {
    let wt = webtransport::Settings::new().webtransport_max_sessions_draft07(1);
    let settings = Settings {
        enable_connect_protocol: None,
        ..Settings::new()
            .h3_datagram(true)
            .enable_webtransport_server(wt)
    };
    let mut client = shiguredo_http3::ClientConnection::new(settings);
    client.set_control_stream_id(2).unwrap();
    let (ctrl, _) = client.take_stream_data(2).unwrap();
    ctrl
}

/// draft-14 クライアントの SETTINGS を持つ Connection を作成し、
/// 制御ストリームデータを返す
/// ENABLE_CONNECT_PROTOCOL あり
fn build_draft14_client_ctrl_with_ecp() -> Vec<u8> {
    let wt = webtransport::Settings::new()
        .wt_max_sessions_draft14(1)
        .wt_initial_max_streams_uni(100)
        .wt_initial_max_streams_bidi(100)
        .wt_initial_max_data(8 * 1024 * 1024);
    let settings = Settings::new()
        .h3_datagram(true)
        .enable_connect_protocol(true)
        .enable_webtransport_server(wt);
    let mut client = shiguredo_http3::ClientConnection::new(settings);
    client.set_control_stream_id(2).unwrap();
    let (ctrl, _) = client.take_stream_data(2).unwrap();
    ctrl
}

/// draft-14 クライアントの SETTINGS (ENABLE_CONNECT_PROTOCOL なし)
fn build_draft14_client_ctrl_without_ecp() -> Vec<u8> {
    let wt = webtransport::Settings::new()
        .wt_max_sessions_draft14(1)
        .wt_initial_max_streams_uni(100)
        .wt_initial_max_streams_bidi(100)
        .wt_initial_max_data(8 * 1024 * 1024);
    let settings = Settings {
        enable_connect_protocol: None,
        ..Settings::new()
            .h3_datagram(true)
            .enable_webtransport_server(wt)
    };
    let mut client = shiguredo_http3::ClientConnection::new(settings);
    client.set_control_stream_id(2).unwrap();
    let (ctrl, _) = client.take_stream_data(2).unwrap();
    ctrl
}

/// draft-15 クライアントの SETTINGS (ENABLE_CONNECT_PROTOCOL あり)
fn build_draft15_client_ctrl_with_ecp() -> Vec<u8> {
    let wt = webtransport::Settings::new().wt_enabled(1);
    let settings = Settings::new()
        .h3_datagram(true)
        .enable_webtransport_server(wt);
    let mut client = shiguredo_http3::ClientConnection::new(settings);
    client.set_control_stream_id(2).unwrap();
    let (ctrl, _) = client.take_stream_data(2).unwrap();
    ctrl
}

/// draft-15 クライアントの SETTINGS (ENABLE_CONNECT_PROTOCOL なし)
fn build_draft15_client_ctrl_without_ecp() -> Vec<u8> {
    let wt = webtransport::Settings::new().wt_enabled(1);
    let settings = Settings {
        enable_connect_protocol: None,
        ..Settings::new()
            .h3_datagram(true)
            .enable_webtransport_server(wt)
    };
    let mut client = shiguredo_http3::ClientConnection::new(settings);
    client.set_control_stream_id(2).unwrap();
    let (ctrl, _) = client.take_stream_data(2).unwrap();
    ctrl
}

/// 全 draft 対応のサーバーを構築する
fn setup_server(reset_stream_at: bool) -> ServerConnection {
    let wt = webtransport::Settings::new()
        .wt_enabled(1)
        .enable_webtransport_draft02(true)
        .webtransport_max_sessions_draft07(100)
        .wt_max_sessions_draft14(100);
    let settings = Settings::new().enable_webtransport_server(wt);
    let mut server = ServerConnection::new(settings);
    server.set_control_stream_id(3).unwrap();
    server
        .set_webtransport_transport_verified(true, reset_stream_at)
        .unwrap();
    // サーバーの制御ストリームデータを消費
    let _ = server.take_stream_data(3).unwrap();
    server
}

/// サーバーにクライアントの制御ストリームデータを feed し、SETTINGS イベントを消費する
fn feed_client_settings(server: &mut ServerConnection, client_ctrl: &[u8]) {
    server.feed_stream(2, client_ctrl, false).unwrap();
    // SETTINGS イベントを消費
    loop {
        match server.poll_event().unwrap() {
            Some(Event::SettingsReceived { .. }) => break,
            Some(_) => {}
            None => break,
        }
    }
}

/// WebTransport CONNECT ヘッダーを構築する
fn wt_connect_headers(draft: DraftVersion) -> Vec<Header> {
    vec![
        Header::new(b":method", b"CONNECT"),
        Header::new(
            b":protocol",
            match draft {
                DraftVersion::Draft15 => b"webtransport-h3" as &[u8],
                _ => b"webtransport",
            },
        ),
        Header::new(b":scheme", b"https"),
        Header::new(b":authority", b"example.com"),
        Header::new(b":path", b"/wt"),
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
        let peer = server.peer_settings().unwrap();
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

    #[test]
    fn draft_detection_correct() {
        let mut server = setup_server(false);
        let client_ctrl = build_draft02_client_ctrl();
        feed_client_settings(&mut server, &client_ctrl);

        let peer = server.peer_settings().unwrap();
        assert_eq!(
            peer.webtransport_draft_pattern(),
            Some(DraftVersion::Draft02)
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
        let peer = server.peer_settings().unwrap();
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
            .webtransport_max_sessions_draft07(1)
            .wt_max_sessions_draft14(1)
            .wt_initial_max_data(8 * 1024 * 1024)
            .wt_initial_max_streams_uni(100)
            .wt_initial_max_streams_bidi(100);
        let settings = Settings {
            enable_connect_protocol: None,
            ..Settings::new()
                .h3_datagram(true)
                .enable_webtransport_server(wt)
        };
        let mut client = shiguredo_http3::ClientConnection::new(settings);
        client.set_control_stream_id(2).unwrap();
        let (client_ctrl, _) = client.take_stream_data(2).unwrap();

        let mut server = setup_server(true);
        feed_client_settings(&mut server, &client_ctrl);

        let peer = server.peer_settings().unwrap();
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

    #[test]
    fn draft_detection_correct() {
        let mut server = setup_server(false);
        let client_ctrl = build_draft07_client_ctrl();
        feed_client_settings(&mut server, &client_ctrl);

        let peer = server.peer_settings().unwrap();
        assert_eq!(
            peer.webtransport_draft_pattern(),
            Some(DraftVersion::Draft07)
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

        let peer = server.peer_settings().unwrap();
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

        let peer = server.peer_settings().unwrap();
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

    #[test]
    fn draft_detection_correct() {
        let mut server = setup_server(false);
        let client_ctrl = build_draft14_client_ctrl_with_ecp();
        feed_client_settings(&mut server, &client_ctrl);

        let peer = server.peer_settings().unwrap();
        assert_eq!(
            peer.webtransport_draft_pattern(),
            Some(DraftVersion::Draft14)
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

        let peer = server.peer_settings().unwrap();
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

        let peer = server.peer_settings().unwrap();
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

    #[test]
    fn draft_detection_correct() {
        let mut server = setup_server(true);
        let client_ctrl = build_draft15_client_ctrl_with_ecp();
        feed_client_settings(&mut server, &client_ctrl);

        let peer = server.peer_settings().unwrap();
        assert_eq!(
            peer.webtransport_draft_pattern(),
            Some(DraftVersion::Draft15)
        );
    }
}

// =========================================================================
// 共通バリデーション (全 draft 共通)
// =========================================================================

mod common {
    use super::*;

    #[test]
    fn connect_rejected_when_h3_datagram_disabled() {
        // H3_DATAGRAM が無効なクライアントからの CONNECT は全 draft で拒否
        let wt = webtransport::Settings::new().webtransport_max_sessions_draft07(1);
        let settings = Settings {
            enable_connect_protocol: None,
            h3_datagram: Some(false),
            ..Settings::new().enable_webtransport_server(wt)
        };
        let mut client = shiguredo_http3::ClientConnection::new(settings);
        client.set_control_stream_id(2).unwrap();
        let (client_ctrl, _) = client.take_stream_data(2).unwrap();

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
        client.set_control_stream_id(2).unwrap();
        let (client_ctrl, _) = client.take_stream_data(2).unwrap();

        let mut server = setup_server(false);
        feed_client_settings(&mut server, &client_ctrl);

        let peer = server.peer_settings().unwrap();
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
            Header::new(b":method", b"CONNECT"),
            Header::new(b":protocol", b"webtransport"),
            Header::new(b":scheme", b"http"),
            Header::new(b":authority", b"example.com"),
            Header::new(b":path", b"/wt"),
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
        let wt = webtransport::Settings::new().webtransport_max_sessions_draft07(1);
        let settings = Settings::new().enable_webtransport_server(wt);
        let mut server = ServerConnection::new(settings);
        server.set_control_stream_id(3).unwrap();
        // set_webtransport_transport_verified を呼ばない
        let _ = server.take_stream_data(3).unwrap();

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
        client.set_control_stream_id(2).unwrap();
        let (client_ctrl, _) = client.take_stream_data(2).unwrap();
        feed_client_settings(&mut server, &client_ctrl);

        let headers = vec![
            Header::new(b":method", b"CONNECT"),
            Header::new(b":protocol", b"websocket"),
            Header::new(b":scheme", b"https"),
            Header::new(b":authority", b"example.com"),
            Header::new(b":path", b"/ws"),
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
// フロー制御無効時の同時セッション数制限 (issue #0051)
// draft-ietf-webtrans-http3-15 Section 5.1, 5.2
// =========================================================================

mod no_flow_control_single_session {
    use super::*;

    /// FC を宣言しない (= initial_max_* を持たない) draft-15 クライアントを作る
    fn build_draft15_client_no_fc() -> shiguredo_http3::ClientConnection {
        let wt = webtransport::Settings::new().wt_enabled(1);
        let settings = Settings::new()
            .h3_datagram(true)
            .enable_webtransport_server(wt);
        let mut client = shiguredo_http3::ClientConnection::new(settings);
        client.set_control_stream_id(2).unwrap();
        client
            .set_webtransport_transport_verified(true, true)
            .unwrap();
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
        let server_wt = webtransport::Settings::new().wt_enabled(1);
        let server_settings = Settings::new()
            .h3_datagram(true)
            .enable_connect_protocol(true)
            .enable_webtransport_server(server_wt);
        let mut server = ServerConnection::new(server_settings);
        server.set_control_stream_id(3).unwrap();
        let (server_ctrl, _) = server.take_stream_data(3).unwrap();

        // クライアントにサーバー制御ストリームを feed して peer SETTINGS を確定
        client.feed_stream(3, &server_ctrl, false).unwrap();
        // SETTINGS イベントを消費
        while let Some(_ev) = client.poll_event().unwrap() {}

        let headers = wt_connect_headers(DraftVersion::Draft15);

        // 1 本目: 受理される
        client
            .send_request(&headers, false)
            .expect("1 本目の WT CONNECT は送信できるべき");

        // 2 本目: RequestRejected で拒否される
        let err = client.send_request(&headers, false).unwrap_err();
        assert!(
            matches!(err, Error::ConnectionError(ErrorCode::RequestRejected)),
            "FC 無効時の 2 本目は RequestRejected で拒否されるべき: {err:?}"
        );
    }
}

// =========================================================================
// WT-Available-Protocols 未送時の WT-Protocol 検証 (issue #0052)
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
        while let Some(_ev) = server.poll_event().unwrap() {}

        // wt-protocol を含む 2xx を返そうとする → エラー
        let response = vec![
            Header::new(b":status", b"200"),
            Header::new(b"wt-protocol", b"\"echo\""),
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
            .wt_enabled(1)
            .wt_initial_max_streams_uni(100)
            .wt_initial_max_streams_bidi(100)
            .wt_initial_max_data(8 * 1024 * 1024);
        let settings = Settings::new()
            .h3_datagram(true)
            .enable_webtransport_server(wt);
        let mut client = shiguredo_http3::ClientConnection::new(settings);
        client.set_control_stream_id(2).unwrap();
        client
            .set_webtransport_transport_verified(true, true)
            .unwrap();
        let _ = client.take_stream_data(2).unwrap();

        // サーバーの SETTINGS を構築して feed
        let server_wt = webtransport::Settings::new()
            .wt_enabled(1)
            .wt_initial_max_streams_uni(100)
            .wt_initial_max_streams_bidi(100)
            .wt_initial_max_data(8 * 1024 * 1024);
        let server_settings = Settings::new()
            .h3_datagram(true)
            .enable_connect_protocol(true)
            .enable_webtransport_server(server_wt);
        let mut server = ServerConnection::new(server_settings);
        server.set_control_stream_id(3).unwrap();
        let (server_ctrl, _) = server.take_stream_data(3).unwrap();
        client.feed_stream(3, &server_ctrl, false).unwrap();
        while let Some(_ev) = client.poll_event().unwrap() {}

        // wt-available-protocols を含まない WT CONNECT を送信
        let headers = wt_connect_headers(DraftVersion::Draft15);
        let stream_id = client.send_request(&headers, false).unwrap();
        // 送信バッファを取り出して破棄 (peer に送らない)
        let _ = client.take_stream_data(stream_id);

        // 偽の 2xx + WT-Protocol レスポンスをクライアントに feed する
        let response = vec![
            Header::new(b":status", b"200"),
            Header::new(b"wt-protocol", b"\"echo\""),
        ];
        let frame = build_headers_frame(&response);
        client.feed_stream(stream_id, &frame, false).unwrap();

        // セッションは Established に遷移せず、WebTransportSessionClosed が発火する
        let mut session_closed = false;
        let mut session_established = false;
        while let Some(ev) = client.poll_event().unwrap() {
            match ev {
                Event::WebTransportSessionClosed { error_code, .. } => {
                    session_closed = true;
                    assert_eq!(
                        error_code,
                        WtErrorCode::AlpnError as u64,
                        "WT_ALPN_ERROR で閉じられるべき"
                    );
                }
                Event::WebTransportSessionEstablished { .. } => {
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
            Header::new(b":method", b"CONNECT"),
            Header::new(b":protocol", protocol),
            Header::new(b":scheme", b"https"),
            Header::new(b":authority", b"example.com"),
            Header::new(b":path", b"/wt"),
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
            .wt_enabled(1)
            .wt_initial_max_streams_uni(100)
            .wt_initial_max_streams_bidi(100)
            .wt_initial_max_data(8 * 1024 * 1024);
        let server_settings = Settings::new()
            .h3_datagram(true)
            .enable_connect_protocol(true)
            .enable_webtransport_server(server_wt);
        let mut server = ServerConnection::new(server_settings);
        server.set_control_stream_id(3).unwrap();
        let (server_ctrl, _) = server.take_stream_data(3).unwrap();

        let client_wt = webtransport::Settings::new().wt_enabled(1);
        let client_settings = Settings::new()
            .h3_datagram(true)
            .enable_webtransport_server(client_wt);
        let mut client = shiguredo_http3::ClientConnection::new(client_settings);
        client.set_control_stream_id(2).unwrap();
        client
            .set_webtransport_transport_verified(true, true)
            .unwrap();
        client.feed_stream(3, &server_ctrl, false).unwrap();
        while let Some(_ev) = client.poll_event().unwrap() {}

        let headers = wt_connect_headers_with_protocol(b"webtransport");
        let result = client.send_request(&headers, false);
        assert!(matches!(
            result,
            Err(Error::ConnectionError(ErrorCode::InternalError))
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
        shiguredo_http3::varint::encode_into_vec(&mut frame, 0x00);
        shiguredo_http3::varint::encode_into_vec(&mut frame, body.len() as u64);
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
    fn draft07_pending_data_rejected() {
        let mut server = setup_server(false);
        let client_ctrl = build_draft07_client_ctrl();
        feed_client_settings(&mut server, &client_ctrl);

        let headers = wt_connect_headers(DraftVersion::Draft07);
        let frame = build_headers_then_data(&headers, &[0x00]);
        let err = server.feed_stream(0, &frame, false).unwrap_err();
        assert!(matches!(err, Error::StreamError(ErrorCode::MessageError)));
    }

    #[test]
    fn draft14_pending_data_rejected() {
        let mut server = setup_server(true);
        let client_ctrl = build_draft14_client_ctrl_with_ecp();
        feed_client_settings(&mut server, &client_ctrl);

        let headers = wt_connect_headers(DraftVersion::Draft14);
        let frame = build_headers_then_data(&headers, &[0x00]);
        let err = server.feed_stream(0, &frame, false).unwrap_err();
        assert!(matches!(err, Error::StreamError(ErrorCode::MessageError)));
    }

    #[test]
    fn draft15_pending_data_rejected() {
        let mut server = setup_server(true);
        let client_ctrl = build_draft15_client_ctrl_with_ecp();
        feed_client_settings(&mut server, &client_ctrl);

        let headers = wt_connect_headers(DraftVersion::Draft15);
        let frame = build_headers_then_data(&headers, &[0x00]);
        let err = server.feed_stream(0, &frame, false).unwrap_err();
        assert!(matches!(err, Error::StreamError(ErrorCode::MessageError)));
    }
}
