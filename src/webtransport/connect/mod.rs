//! WebTransport CONNECT リクエスト/レスポンス (draft-ietf-webtrans-http3-15 Section 3)
//!
//! 拡張 CONNECT (RFC 8441, RFC 9220) を使用した WebTransport セッションの
//! 確立リクエストのバリデーションとプロトコルネゴシエーションを提供。
//! SF パーサーは `sf_parser.rs` で管理する。
//!
//! # 参照
//!
//! - RFC 8441: Bootstrapping WebSockets with HTTP/2 (拡張 CONNECT の定義)
//! - RFC 9220: Bootstrapping WebSockets with HTTP/3 (HTTP/3 への適用)
//! - draft-ietf-webtrans-http3-15 / 16 / 17 Section 3.2: Creating a New Session
//! - draft-ietf-webtrans-http3-15 / 16 / 17 Section 3.3: Application Protocol Negotiation

mod connect_error;
mod draft;
mod request;
mod response;
mod sf_parser;

pub use connect_error::{CapabilityError, ConnectError, TransportCapabilities};
pub use draft::{DraftVersion, ServerSettingsParams};
pub use request::ConnectRequest;
pub use response::ConnectResponse;

use core::fmt;

/// `:protocol` 疑似ヘッダーの値
/// (draft-ietf-webtrans-http3-15 / 16 / 17 Section 3.2, Section 9.1)
///
/// draft-15 / 16 / 17 で共通の native QUIC モードのプロトコル識別子。
pub const PROTOCOL_WEBTRANSPORT_H3: &str = "webtransport-h3";

/// `:protocol` 疑似ヘッダーの値 (draft-ietf-webtrans-http3-02 Section 3.3)
///
/// draft-02 / 07 / 14 で定義されたプロトコル識別子。Chrome 等の実装が
/// draft-02 互換でこの値を送信する場合がある。draft-17 Section 2.1.2 では
/// capsule-based WebTransport (WEBTRANS-H2) のトークンとして位置付けられ、
/// 本実装が扱うネイティブ版 (`webtransport-h3`) とは別プロトコルになる。
/// 将来のドラフトで廃止される可能性がある。
pub const PROTOCOL_WEBTRANSPORT_DRAFT02: &str = "webtransport";

/// `:protocol` 疑似ヘッダーが示すプロトコル系統
///
/// `:protocol` のトークンは複数のドラフト版で共有されるため、ヘッダーだけでは
/// 接続で使うドラフト版を一意に決められない。たとえば `webtransport-h3` は
/// draft-15 / 16 / 17 の共通トークンである (draft-ietf-webtrans-http3-17
/// Section 9.1)。そのためヘッダー解析の結果はこの型で表し、接続の版判定には
/// 使わない。接続で使う版は SETTINGS のコードポイントによる接続単位の選択
/// ([`crate::Connection::negotiated_wt_draft_version`]) で決める。
///
/// 後方互換のない変更: [`ConnectRequest`] の `draft_version` フィールド
/// ([`DraftVersion`]) をこの型の `protocol_token` フィールドに置き換えた。
/// ヘッダーからトークン系統しか分からないため、この型では版を表現できない。
///
/// draft-ietf-webtrans-http3-02 Section 3.3, draft-ietf-webtrans-http3-17 Section 2.1.2, 9.1
/// 将来のドラフトで変更される可能性がある
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProtocolToken {
    /// `webtransport-h3` (draft-15 / 16 / 17 の共通トークン)
    ///
    /// draft-ietf-webtrans-http3-17 Section 9.1
    WebTransportH3,
    /// `webtransport` (draft-02 / 07 / 14 の共通トークン)
    ///
    /// draft-ietf-webtrans-http3-17 Section 2.1.2 では capsule-based
    /// WebTransport (WEBTRANS-H2) のトークンとして位置付けられており、
    /// 本文書の native QUIC モードとは別プロトコルになる。
    WebTransport,
}

impl ProtocolToken {
    /// トークンの文字列値を返す
    ///
    /// 将来のドラフトで変更される可能性がある
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WebTransportH3 => PROTOCOL_WEBTRANSPORT_H3,
            Self::WebTransport => PROTOCOL_WEBTRANSPORT_DRAFT02,
        }
    }

    /// `:protocol` の値からトークン系統を判定する
    ///
    /// 未知のトークンは `None` を返す。
    /// 将来のドラフトで変更される可能性がある
    pub fn from_bytes(value: &[u8]) -> Option<Self> {
        if value == PROTOCOL_WEBTRANSPORT_H3.as_bytes() {
            return Some(Self::WebTransportH3);
        }
        if value == PROTOCOL_WEBTRANSPORT_DRAFT02.as_bytes() {
            return Some(Self::WebTransport);
        }
        None
    }

    /// ドラフト版に対応するトークン系統を返す
    ///
    /// 同じトークン系統に複数のドラフト版が対応するため、逆方向の変換
    /// (トークンからドラフト版を求めること) はできない。
    /// 将来のドラフトで変更される可能性がある
    pub fn from_draft_version(version: DraftVersion) -> Self {
        match version {
            DraftVersion::Draft02 | DraftVersion::Draft07 | DraftVersion::Draft14 => {
                Self::WebTransport
            }
            DraftVersion::Draft15 | DraftVersion::Draft17 => Self::WebTransportH3,
        }
    }
}

impl fmt::Display for ConnectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidScheme => write!(f, ":scheme must be \"https\""),
            Self::MissingAuthority => write!(f, ":authority is required"),
            Self::MissingPath => write!(f, ":path is required"),
            Self::InvalidMethod => write!(f, ":method must be \"CONNECT\""),
            Self::InvalidProtocol => {
                write!(
                    f,
                    ":protocol must be \"{}\" or \"{}\"",
                    PROTOCOL_WEBTRANSPORT_H3, PROTOCOL_WEBTRANSPORT_DRAFT02
                )
            }
            Self::InvalidEncoding => write!(f, "header value is not valid UTF-8"),
        }
    }
}

/// WebTransport セッション開始前提の検証エラー (draft-ietf-webtrans-http3-15 Section 3.1)
impl fmt::Display for CapabilityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingWebTransportSetting => {
                write!(f, "SETTINGS_WT_ENABLED with value of 1 is required")
            }
            Self::MissingEnableConnectProtocol => {
                write!(
                    f,
                    "SETTINGS_ENABLE_CONNECT_PROTOCOL with value 1 is required"
                )
            }
            Self::MissingH3Datagram => write!(f, "SETTINGS_H3_DATAGRAM with value 1 is required"),
            Self::MissingQuicDatagram => {
                write!(
                    f,
                    "max_datagram_frame_size transport parameter > 0 is required"
                )
            }
            Self::MissingResetStreamAt => {
                write!(f, "reset_stream_at transport parameter is required")
            }
        }
    }
}

/// WebTransport 前提機能のネゴシエーション結果
///
/// 呼び出し側が SETTINGS / transport parameters から値を埋めて検証する。
///
/// WebTransport CONNECT リクエスト (draft-ietf-webtrans-http3-15 Section 3.2)
///
/// クライアントが新しい WebTransport セッションを確立するための拡張 CONNECT リクエスト。
/// `:protocol` ヘッダーの値はドラフトバージョンに依存する。
///
/// # 必須疑似ヘッダー
///
/// - `:method` = `CONNECT` (拡張 CONNECT)
/// - `:protocol` = ドラフトバージョンに依存 (`webtransport` or `webtransport-h3`)
/// - `:scheme` = `https`
/// - `:authority` = サーバーリソースの識別子
/// - `:path` = サーバーリソースのパス
///
/// # 任意ヘッダー
///
/// - `Origin` = クライアントオリジン (ブラウザクライアントの場合は MUST)
/// - `WT-Available-Protocols` = 利用可能なアプリケーションプロトコルリスト
///
/// WebTransport CONNECT レスポンス (draft-ietf-webtrans-http3-15 Section 3.2)
///
/// サーバーが CONNECT リクエストに対して返すレスポンス。
/// 2xx ステータスコードでセッション確立成功。
/// クライアントは 3xx リダイレクトを自動追従してはならない (MUST NOT)。
///
/// Structured Fields List から文字列型アイテムを抽出 (RFC 9651 簡易実装)
///
/// WebTransport の用途に特化した実装:
/// - カンマ区切りのリストを解析
/// - 全アイテムがクォート文字列の場合のみ結果を返す
#[cfg(test)]
mod tests {
    use super::*;
    use crate::VarInt;

    #[test]
    fn test_connect_request_validate_success() {
        let req = ConnectRequest::new("https", "example.com", "/path");
        assert!(req.validate().is_ok());
    }

    #[test]
    fn test_connect_request_validate_invalid_scheme() {
        let req = ConnectRequest::new("http", "example.com", "/");
        assert_eq!(req.validate(), Err(ConnectError::InvalidScheme));
    }

    #[test]
    fn test_connect_request_validate_missing_authority() {
        let req = ConnectRequest::new("https", "", "/");
        assert_eq!(req.validate(), Err(ConnectError::MissingAuthority));
    }

    #[test]
    fn test_connect_request_validate_missing_path() {
        let req = ConnectRequest::new("https", "example.com", "");
        assert_eq!(req.validate(), Err(ConnectError::MissingPath));
    }

    #[test]
    fn test_parse_available_protocols_basic() {
        let result = ConnectRequest::parse_available_protocols("\"foo\", \"bar\"");
        assert_eq!(result, vec!["foo".to_string(), "bar".to_string()]);
    }

    #[test]
    fn test_parse_available_protocols_ignores_non_strings() {
        // 非 String 要素を含む場合はフィールド全体を無視する
        // (draft-ietf-webtrans-http3-15 Section 3.3)
        let result = ConnectRequest::parse_available_protocols("\"foo\", token, 42");
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_available_protocols_with_parameters() {
        // パラメータは無視される
        let result = ConnectRequest::parse_available_protocols("\"foo\";q=1, \"bar\";q=0.5");
        assert_eq!(result, vec!["foo".to_string(), "bar".to_string()]);
    }

    #[test]
    fn test_parse_available_protocols_empty() {
        let result = ConnectRequest::parse_available_protocols("");
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_protocol_basic() {
        let result = ConnectResponse::parse_protocol("\"foo\"");
        assert_eq!(result, Some("foo".to_string()));
    }

    #[test]
    fn test_parse_protocol_non_string() {
        // Token 型 (クォートなし) は None
        let result = ConnectResponse::parse_protocol("token");
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_protocol_with_parameters() {
        // パラメータは無視される
        let result = ConnectResponse::parse_protocol("\"foo\";q=1");
        assert_eq!(result, Some("foo".to_string()));
    }

    #[test]
    fn test_parse_protocol_escape_quote() {
        let result = ConnectResponse::parse_protocol("\"foo\\\"bar\"");
        assert_eq!(result, Some("foo\"bar".to_string()));
    }

    #[test]
    fn test_parse_protocol_escape_backslash() {
        let result = ConnectResponse::parse_protocol("\"foo\\\\bar\"");
        assert_eq!(result, Some("foo\\bar".to_string()));
    }

    #[test]
    fn test_connect_response_is_success() {
        assert!(ConnectResponse::new(200).is_success());
        assert!(ConnectResponse::new(201).is_success());
        assert!(!ConnectResponse::new(301).is_success());
        assert!(!ConnectResponse::new(404).is_success());
        assert!(!ConnectResponse::new(500).is_success());
    }

    #[test]
    fn test_is_protocol_valid_no_available_protocols() {
        // WT-Available-Protocols なしで WT-Protocol あり = 無効
        let req = ConnectRequest::new("https", "example.com", "/");
        let resp = ConnectResponse::new(200).with_protocol("foo");
        assert!(!resp.is_protocol_valid(&req));
    }

    #[test]
    fn test_is_protocol_valid_match() {
        let req = ConnectRequest::new("https", "example.com", "/")
            .available_protocols(vec!["foo".to_string(), "bar".to_string()]);
        let resp = ConnectResponse::new(200).with_protocol("foo");
        assert!(resp.is_protocol_valid(&req));
    }

    #[test]
    fn test_is_protocol_valid_no_match() {
        let req = ConnectRequest::new("https", "example.com", "/")
            .available_protocols(vec!["foo".to_string()]);
        let resp = ConnectResponse::new(200).with_protocol("baz");
        assert!(!resp.is_protocol_valid(&req));
    }

    #[test]
    fn test_is_protocol_valid_no_protocol_in_response_no_negotiation() {
        // リクエストにも WT-Available-Protocols なし = ネゴシエーション不要なので有効
        let req = ConnectRequest::new("https", "example.com", "/");
        let resp = ConnectResponse::new(200);
        assert!(resp.is_protocol_valid(&req));
    }

    #[test]
    fn test_is_protocol_valid_no_protocol_in_response_with_negotiation() {
        // リクエストに WT-Available-Protocols あり、レスポンスに WT-Protocol なし
        // → draft-15 Section 3.3: MUST close with WT_ALPN_ERROR
        let req = ConnectRequest::new("https", "example.com", "/")
            .available_protocols(vec!["foo".to_string()]);
        let resp = ConnectResponse::new(200);
        assert!(!resp.is_protocol_valid(&req));
    }

    #[test]
    fn test_connect_error_display() {
        assert_eq!(
            format!("{}", ConnectError::InvalidScheme),
            ":scheme must be \"https\""
        );
        assert_eq!(
            format!("{}", ConnectError::MissingAuthority),
            ":authority is required"
        );
        assert_eq!(
            format!("{}", ConnectError::MissingPath),
            ":path is required"
        );
    }

    // ---------------------------------------------------------------
    // to_headers / from_headers テスト
    // ---------------------------------------------------------------

    #[test]
    fn test_connect_request_to_headers_basic() {
        let req = ConnectRequest::new("https", "example.com", "/wt");
        let headers = req.to_headers().expect("test must succeed");
        assert_eq!(headers.len(), 5);
        assert_eq!(headers[0].name(), b":method");
        assert_eq!(headers[0].value(), b"CONNECT");
        assert_eq!(headers[1].name(), b":protocol");
        assert_eq!(headers[1].value(), PROTOCOL_WEBTRANSPORT_H3.as_bytes());
        assert_eq!(headers[2].name(), b":scheme");
        assert_eq!(headers[2].value(), b"https");
        assert_eq!(headers[3].name(), b":authority");
        assert_eq!(headers[3].value(), b"example.com");
        assert_eq!(headers[4].name(), b":path");
        assert_eq!(headers[4].value(), b"/wt");
    }

    #[test]
    fn test_connect_request_to_headers_draft02() {
        let req =
            ConnectRequest::new("https", "example.com", "/wt").draft_version(DraftVersion::Draft02);
        let headers = req.to_headers().expect("test must succeed");
        assert_eq!(headers[1].name(), b":protocol");
        assert_eq!(headers[1].value(), PROTOCOL_WEBTRANSPORT_DRAFT02.as_bytes());
    }

    #[test]
    fn test_connect_request_to_headers_draft07() {
        let req =
            ConnectRequest::new("https", "example.com", "/wt").draft_version(DraftVersion::Draft07);
        let headers = req.to_headers().expect("test must succeed");
        assert_eq!(headers[1].name(), b":protocol");
        assert_eq!(headers[1].value(), PROTOCOL_WEBTRANSPORT_DRAFT02.as_bytes());
    }

    #[test]
    fn test_connect_request_to_headers_with_origin() {
        let req =
            ConnectRequest::new("https", "example.com", "/wt").origin("https://client.example");
        let headers = req.to_headers().expect("test must succeed");
        assert_eq!(headers.len(), 6);
        assert_eq!(headers[5].name(), b"origin");
        assert_eq!(headers[5].value(), b"https://client.example");
    }

    #[test]
    fn test_connect_request_to_headers_with_available_protocols() {
        let req = ConnectRequest::new("https", "example.com", "/wt")
            .available_protocols(vec!["moq".to_string(), "chat".to_string()]);
        let headers = req.to_headers().expect("test must succeed");
        assert_eq!(headers.len(), 6);
        assert_eq!(headers[5].name(), b"wt-available-protocols");
        assert_eq!(headers[5].value(), b"\"moq\", \"chat\"");
    }

    #[test]
    fn test_connect_request_from_headers_draft15() {
        let headers: Vec<(&[u8], &[u8])> = vec![
            (b":method", b"CONNECT"),
            (b":protocol", b"webtransport-h3"),
            (b":scheme", b"https"),
            (b":authority", b"example.com"),
            (b":path", b"/wt"),
        ];
        let req = ConnectRequest::from_headers(&headers).expect("test must succeed");
        assert_eq!(req.protocol_token, ProtocolToken::WebTransportH3);
        assert_eq!(req.scheme, "https");
        assert_eq!(req.authority, "example.com");
        assert_eq!(req.path, "/wt");
        assert!(req.origin.is_none());
        assert!(req.available_protocols.is_empty());
    }

    #[test]
    fn test_connect_request_from_headers_draft02_compat() {
        // draft-02 互換の :protocol 値も受け入れる
        let headers: Vec<(&[u8], &[u8])> = vec![
            (b":method", b"CONNECT"),
            (b":protocol", b"webtransport"),
            (b":scheme", b"https"),
            (b":authority", b"example.com"),
            (b":path", b"/"),
        ];
        let req = ConnectRequest::from_headers(&headers).expect("test must succeed");
        assert_eq!(req.protocol_token, ProtocolToken::WebTransport);
        assert_eq!(req.scheme, "https");
    }

    #[test]
    fn test_connect_request_from_headers_invalid_method() {
        let headers: Vec<(&[u8], &[u8])> =
            vec![(b":method", b"GET"), (b":protocol", b"webtransport-h3")];
        assert!(matches!(
            ConnectRequest::from_headers(&headers),
            Err(ConnectError::InvalidMethod)
        ));
    }

    #[test]
    fn test_connect_request_from_headers_invalid_protocol() {
        let headers: Vec<(&[u8], &[u8])> = vec![(b":method", b"CONNECT"), (b":protocol", b"h2c")];
        assert!(matches!(
            ConnectRequest::from_headers(&headers),
            Err(ConnectError::InvalidProtocol)
        ));
    }

    #[test]
    fn test_protocol_token_string_roundtrip() {
        // トークンの文字列表現と判定が一致すること
        for (token, value) in [
            (ProtocolToken::WebTransportH3, PROTOCOL_WEBTRANSPORT_H3),
            (ProtocolToken::WebTransport, PROTOCOL_WEBTRANSPORT_DRAFT02),
        ] {
            assert_eq!(token.as_str(), value);
            assert_eq!(ProtocolToken::from_bytes(value.as_bytes()), Some(token));
        }
        // 未知のトークンは判定できない
        assert_eq!(ProtocolToken::from_bytes(b"h2c"), None);
        assert_eq!(ProtocolToken::from_bytes(b""), None);
    }

    #[test]
    fn test_protocol_token_from_draft_version() {
        // 同じトークン系統に複数のドラフト版が対応する
        for version in [
            DraftVersion::Draft02,
            DraftVersion::Draft07,
            DraftVersion::Draft14,
        ] {
            assert_eq!(
                ProtocolToken::from_draft_version(version),
                ProtocolToken::WebTransport
            );
        }
        for version in [DraftVersion::Draft15, DraftVersion::Draft17] {
            assert_eq!(
                ProtocolToken::from_draft_version(version),
                ProtocolToken::WebTransportH3
            );
        }
    }

    #[test]
    fn test_connect_request_draft_version_shares_token_across_versions() {
        // 同じトークン系統の版を指定した場合、生成されるヘッダーは同一になる。
        // 接続で使う版は SETTINGS のコードポイントで決まるため、ヘッダーだけでは
        // draft-15 と draft-17 を区別できない
        // (draft-ietf-webtrans-http3-17 Section 9.1)。
        let draft15 = ConnectRequest::new("https", "example.com", "/wt")
            .draft_version(DraftVersion::Draft15)
            .to_headers()
            .expect("test must succeed");
        let draft17 = ConnectRequest::new("https", "example.com", "/wt")
            .draft_version(DraftVersion::Draft17)
            .to_headers()
            .expect("test must succeed");
        assert_eq!(draft15[1].value(), draft17[1].value());
        assert_eq!(draft15[1].value(), PROTOCOL_WEBTRANSPORT_H3.as_bytes());
    }

    #[test]
    fn test_connect_request_from_headers_missing_method() {
        let headers: Vec<(&[u8], &[u8])> = vec![(b":protocol", b"webtransport-h3")];
        assert!(matches!(
            ConnectRequest::from_headers(&headers),
            Err(ConnectError::InvalidMethod)
        ));
    }

    #[test]
    fn test_connect_request_from_headers_with_origin() {
        let headers: Vec<(&[u8], &[u8])> = vec![
            (b":method", b"CONNECT"),
            (b":protocol", b"webtransport-h3"),
            (b":scheme", b"https"),
            (b":authority", b"example.com"),
            (b":path", b"/wt"),
            (b"origin", b"https://client.example"),
        ];
        let req = ConnectRequest::from_headers(&headers).expect("test must succeed");
        assert_eq!(req.origin, Some("https://client.example".to_string()));
    }

    #[test]
    fn test_connect_request_roundtrip() {
        // to_headers で生成したヘッダーを from_headers でパースする
        let original =
            ConnectRequest::new("https", "example.com", "/wt").origin("https://client.example");
        let headers = original.to_headers().expect("test must succeed");
        let pairs: Vec<(&[u8], &[u8])> = headers.iter().map(|h| (h.name(), h.value())).collect();
        let parsed = ConnectRequest::from_headers(&pairs).expect("test must succeed");
        assert_eq!(parsed.scheme, original.scheme);
        assert_eq!(parsed.authority, original.authority);
        assert_eq!(parsed.path, original.path);
        assert_eq!(parsed.origin, original.origin);
    }

    #[test]
    fn test_connect_response_to_headers_basic() {
        let resp = ConnectResponse::new(200);
        let headers = resp.to_headers().expect("test must succeed");
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0].name(), b":status");
        assert_eq!(headers[0].value(), b"200");
    }

    #[test]
    fn test_connect_response_to_headers_with_protocol() {
        let resp = ConnectResponse::new(200).with_protocol("moq");
        let headers = resp.to_headers().expect("test must succeed");
        assert_eq!(headers.len(), 2);
        assert_eq!(headers[1].name(), b"wt-protocol");
        assert_eq!(headers[1].value(), b"\"moq\"");
    }

    #[test]
    fn test_connect_error_display_new_variants() {
        assert_eq!(
            format!("{}", ConnectError::InvalidMethod),
            ":method must be \"CONNECT\""
        );
        assert!(format!("{}", ConnectError::InvalidProtocol).contains("webtransport-h3"));
        assert_eq!(
            format!("{}", ConnectError::InvalidEncoding),
            "header value is not valid UTF-8"
        );
    }

    #[test]
    fn test_transport_capabilities_validate_success() {
        let caps = TransportCapabilities {
            wt_enabled: true,
            enable_connect_protocol: true,
            h3_datagram: true,
            max_datagram_frame_size: true,
            reset_stream_at: true,
        };
        assert!(caps.validate().is_ok());
    }

    #[test]
    fn test_transport_capabilities_validate_missing_setting() {
        let caps = TransportCapabilities::new();
        assert_eq!(
            caps.validate(),
            Err(CapabilityError::MissingWebTransportSetting)
        );
    }

    fn vi(value: u64) -> VarInt {
        VarInt::new(value).expect("test must succeed")
    }

    #[test]
    fn test_build_server_settings_draft15() {
        let params = ServerSettingsParams {
            initial_max_streams_uni: vi(500),
            initial_max_streams_bidi: vi(300),
            ..Default::default()
        };
        let s = DraftVersion::Draft15.build_server_settings(&params);
        assert_eq!(s.wt_enabled, vi(1));
        assert_eq!(s.wt_initial_max_streams_uni, vi(500));
        assert_eq!(s.wt_initial_max_streams_bidi, vi(300));
        // draft-15 でも wt_initial_max_data を反映する (Section 5.5.3)
        assert_eq!(
            s.wt_initial_max_data,
            ServerSettingsParams::default().initial_max_data
        );
        assert_eq!(s.webtransport_max_sessions_draft07, None);
        assert_eq!(s.wt_max_sessions_draft14, None);
    }

    #[test]
    fn test_build_server_settings_draft14() {
        let params = ServerSettingsParams {
            max_sessions: vi(100),
            initial_max_streams_uni: vi(1000),
            initial_max_streams_bidi: vi(1000),
            initial_max_data: vi(8 * 1024 * 1024),
        };
        let s = DraftVersion::Draft14.build_server_settings(&params);
        // Safari 互換: draft-07 と draft-14 の両方の max_sessions を設定し、
        // 初期フロー制御値はカプセルで通知するため SETTINGS には含めない。
        assert_eq!(s.wt_max_sessions_draft14, Some(vi(100)));
        assert_eq!(s.webtransport_max_sessions_draft07, Some(vi(100)));
        assert_eq!(s.wt_initial_max_streams_uni, VarInt::ZERO);
        assert_eq!(s.wt_initial_max_streams_bidi, VarInt::ZERO);
        assert_eq!(s.wt_initial_max_data, VarInt::ZERO);
    }

    #[test]
    fn test_build_server_settings_draft07() {
        let params = ServerSettingsParams::default();
        let s = DraftVersion::Draft07.build_server_settings(&params);
        assert_eq!(s.webtransport_max_sessions_draft07, Some(vi(1)));
        assert_eq!(s.wt_enabled, VarInt::ZERO);
        assert_eq!(s.wt_max_sessions_draft14, None);
        assert_eq!(s.enable_webtransport_draft02, None);
    }

    #[test]
    fn test_build_server_settings_draft02() {
        let params = ServerSettingsParams::default();
        let s = DraftVersion::Draft02.build_server_settings(&params);
        assert_eq!(s.enable_webtransport_draft02, Some(true));
        assert_eq!(s.wt_enabled, VarInt::ZERO);
        assert_eq!(s.webtransport_max_sessions_draft07, None);
    }

    #[test]
    fn test_build_server_settings_draft14_safari_roundtrip() {
        // Draft14 用のサーバー SETTINGS は draft-07 / draft-14 両方の ID を含む。
        // SETTINGS ネゴシエーション優先順位では draft-07 を優先するため、
        // detect_draft_pattern は Draft07 を返す。draft-14 固有のカプセルベース
        // フロー制御はセッション確立後に別途扱う。
        let params = ServerSettingsParams::default();
        let s = DraftVersion::Draft14.build_server_settings(&params);
        assert_eq!(s.detect_draft_pattern(), Some(DraftVersion::Draft07));
        assert_eq!(s.wt_max_sessions_draft14, Some(params.max_sessions));
        assert_eq!(
            s.webtransport_max_sessions_draft07,
            Some(params.max_sessions)
        );
    }

    #[test]
    fn test_requires_initial_capsule_flow_control() {
        assert!(!DraftVersion::Draft02.requires_initial_capsule_flow_control());
        assert!(!DraftVersion::Draft07.requires_initial_capsule_flow_control());
        assert!(DraftVersion::Draft14.requires_initial_capsule_flow_control());
        assert!(!DraftVersion::Draft15.requires_initial_capsule_flow_control());
        assert!(!DraftVersion::Draft17.requires_initial_capsule_flow_control());
    }

    #[test]
    fn test_requires_enable_connect_protocol() {
        assert!(!DraftVersion::Draft02.requires_enable_connect_protocol());
        assert!(DraftVersion::Draft07.requires_enable_connect_protocol());
        assert!(DraftVersion::Draft14.requires_enable_connect_protocol());
        assert!(DraftVersion::Draft15.requires_enable_connect_protocol());
        assert!(DraftVersion::Draft17.requires_enable_connect_protocol());
    }

    #[test]
    fn test_requires_reset_stream_at() {
        assert!(!DraftVersion::Draft02.requires_reset_stream_at());
        assert!(!DraftVersion::Draft07.requires_reset_stream_at());
        assert!(DraftVersion::Draft14.requires_reset_stream_at());
        assert!(DraftVersion::Draft15.requires_reset_stream_at());
        assert!(DraftVersion::Draft17.requires_reset_stream_at());
    }

    #[test]
    fn test_protocol_value() {
        // draft-15 / 16 / 17 は `webtransport-h3` (draft-ietf-webtrans-http3-17 Section 9.1)、
        // draft-02 / 07 / 14 は `webtransport` (draft-ietf-webtrans-http3-02 Section 3.3)
        for draft in [
            DraftVersion::Draft02,
            DraftVersion::Draft07,
            DraftVersion::Draft14,
        ] {
            assert_eq!(
                draft.protocol_value(),
                PROTOCOL_WEBTRANSPORT_DRAFT02,
                "{draft:?} の :protocol トークン"
            );
        }
        for draft in [DraftVersion::Draft15, DraftVersion::Draft17] {
            assert_eq!(
                draft.protocol_value(),
                PROTOCOL_WEBTRANSPORT_H3,
                "{draft:?} の :protocol トークン"
            );
        }
    }

    #[test]
    fn test_build_server_settings_draft17() {
        let params = ServerSettingsParams {
            initial_max_streams_uni: vi(500),
            initial_max_streams_bidi: vi(300),
            ..Default::default()
        };
        let s = DraftVersion::Draft17.build_server_settings(&params);
        // draft-17 は新コードポイントの SETTINGS_WT_ENABLED を送り、
        // 旧コードポイントは送らない
        assert_eq!(s.wt_enabled_draft17, vi(1));
        assert_eq!(s.wt_enabled, VarInt::ZERO);
        assert_eq!(s.wt_initial_max_streams_uni, vi(500));
        assert_eq!(s.wt_initial_max_streams_bidi, vi(300));
        assert_eq!(
            s.wt_initial_max_data,
            ServerSettingsParams::default().initial_max_data
        );
        assert_eq!(s.webtransport_max_sessions_draft07, None);
        assert_eq!(s.wt_max_sessions_draft14, None);
        assert_eq!(s.detect_draft_pattern(), Some(DraftVersion::Draft17));
    }

    #[test]
    fn test_build_client_settings_draft15() {
        let params = ServerSettingsParams::default();
        let s = DraftVersion::Draft15.build_client_settings(&params);
        assert_eq!(s.wt_enabled, vi(1));
        assert_eq!(s.wt_initial_max_streams_uni, params.initial_max_streams_uni);
        assert_eq!(
            s.wt_initial_max_streams_bidi,
            params.initial_max_streams_bidi
        );
        assert_eq!(s.wt_initial_max_data, params.initial_max_data);
        assert_eq!(s.webtransport_max_sessions_draft07, None);
        assert_eq!(s.wt_max_sessions_draft14, None);
        assert_eq!(s.detect_draft_pattern(), Some(DraftVersion::Draft15));
    }

    #[test]
    fn test_build_client_settings_draft17() {
        let params = ServerSettingsParams::default();
        let s = DraftVersion::Draft17.build_client_settings(&params);
        // draft-17 のクライアントは新コードポイントの SETTINGS_WT_ENABLED を送る
        assert_eq!(s.wt_enabled_draft17, vi(1));
        assert_eq!(s.wt_enabled, VarInt::ZERO);
        assert_eq!(s.wt_initial_max_streams_uni, params.initial_max_streams_uni);
        assert_eq!(
            s.wt_initial_max_streams_bidi,
            params.initial_max_streams_bidi
        );
        assert_eq!(s.wt_initial_max_data, params.initial_max_data);
        assert_eq!(s.webtransport_max_sessions_draft07, None);
        assert_eq!(s.wt_max_sessions_draft14, None);
        assert_eq!(s.detect_draft_pattern(), Some(DraftVersion::Draft17));
    }

    #[test]
    fn test_build_client_settings_draft14() {
        let params = ServerSettingsParams::default();
        let s = DraftVersion::Draft14.build_client_settings(&params);
        // クライアントは Safari 互換の draft-07 ID を送らない (純粋な draft-14)
        assert_eq!(s.wt_max_sessions_draft14, Some(params.max_sessions));
        assert_eq!(s.webtransport_max_sessions_draft07, None);
        assert_eq!(s.wt_initial_max_streams_uni, params.initial_max_streams_uni);
        assert_eq!(
            s.wt_initial_max_streams_bidi,
            params.initial_max_streams_bidi
        );
        assert_eq!(s.wt_initial_max_data, params.initial_max_data);
        assert_eq!(s.wt_enabled, VarInt::ZERO);
        assert_eq!(s.detect_draft_pattern(), Some(DraftVersion::Draft14));
    }

    #[test]
    fn test_build_client_settings_draft07() {
        let params = ServerSettingsParams::default();
        let s = DraftVersion::Draft07.build_client_settings(&params);
        assert_eq!(
            s.webtransport_max_sessions_draft07,
            Some(params.max_sessions)
        );
        assert_eq!(s.wt_max_sessions_draft14, None);
        assert_eq!(s.wt_enabled, VarInt::ZERO);
        assert_eq!(s.enable_webtransport_draft02, None);
        assert_eq!(s.detect_draft_pattern(), Some(DraftVersion::Draft07));
    }

    #[test]
    fn test_build_client_settings_draft02() {
        let params = ServerSettingsParams::default();
        let s = DraftVersion::Draft02.build_client_settings(&params);
        assert_eq!(s.enable_webtransport_draft02, Some(true));
        assert_eq!(s.wt_enabled, VarInt::ZERO);
        assert_eq!(s.webtransport_max_sessions_draft07, None);
        assert_eq!(s.wt_max_sessions_draft14, None);
        assert_eq!(s.detect_draft_pattern(), Some(DraftVersion::Draft02));
    }
}
