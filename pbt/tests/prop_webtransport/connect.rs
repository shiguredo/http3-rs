//! ConnectRequest / ConnectResponse のバリデーションとプロトコルネゴシエーション
//! (draft-ietf-webtrans-http3-15 Section 3.2, 3.3)

use proptest::prelude::*;
use shiguredo_http3::webtransport::{ConnectRequest, ConnectResponse};

prop_compose! {
    /// 任意の非空文字列 (HTTP ヘッダー値として安全な文字のみ)
    fn non_empty_string()(
        len in 1usize..64,
    )(
        s in prop::collection::vec(0x20u8..0x7f, len)
    ) -> String {
        String::from_utf8(s).unwrap_or_else(|_| "x".to_string())
    }
}

/// プロトコル名として安全な文字列の Strategy
///
/// Structured Fields List のカンマ区切りや
/// クォート文字列のエスケープ対象文字 (',', '"', '\\') を除外する。
fn safe_protocol_name() -> impl Strategy<Value = String> {
    prop::string::string_regex("[a-zA-Z0-9_\\-\\.\\+/]{1,32}")
        .expect("valid regex")
        .prop_filter("non-empty", |s| !s.is_empty())
}

// =============================================================================
// ConnectRequest バリデーション (draft-ietf-webtrans-http3-15 Section 3.2)
// =============================================================================

proptest! {
    /// Property: 有効なリクエストは validate() が Ok を返す
    #[test]
    fn prop_connect_request_valid(
        authority in non_empty_string(),
        path in non_empty_string(),
    ) {
        let req = ConnectRequest::new("https", authority, path);
        prop_assert!(req.validate().is_ok());
    }

    /// Property: scheme が "https" 以外なら InvalidScheme
    #[test]
    fn prop_connect_request_invalid_scheme(
        scheme in non_empty_string(),
        authority in non_empty_string(),
        path in non_empty_string(),
    ) {
        prop_assume!(scheme != "https");
        let req = ConnectRequest::new(scheme, authority, path);
        prop_assert!(req.validate().is_err());
    }

    /// Property: WT-Available-Protocols の文字列型のみを抽出
    #[test]
    fn prop_parse_available_protocols_strings_only(
        protocols in prop::collection::vec(safe_protocol_name(), 1..5),
    ) {
        let header_value = protocols
            .iter()
            .map(|p| format!("\"{}\"", p))
            .collect::<Vec<_>>()
            .join(", ");

        let result = ConnectRequest::parse_available_protocols(&header_value);
        prop_assert_eq!(result, protocols);
    }

    /// Property: WT-Protocol 文字列型のラウンドトリップ
    #[test]
    fn prop_parse_protocol_string_roundtrip(proto in safe_protocol_name()) {
        let header_value = format!("\"{}\"", proto);
        let result = ConnectResponse::parse_protocol(&header_value);
        prop_assert_eq!(result, Some(proto));
    }

    /// Property: available_protocols が空なら selected_protocol なしで true
    #[test]
    fn prop_connect_response_no_protocol_no_negotiation_valid(
        authority in non_empty_string(),
        path in non_empty_string(),
    ) {
        let req = ConnectRequest::new("https", authority, path);
        let resp = ConnectResponse::new(200);
        prop_assert!(resp.is_protocol_valid(&req));
    }

    /// Property: available_protocols が非空なら selected_protocol なしで false (draft-15)
    #[test]
    fn prop_connect_response_no_protocol_with_negotiation_invalid(
        authority in non_empty_string(),
        path in non_empty_string(),
        protocols in prop::collection::vec(non_empty_string(), 1..5),
    ) {
        let req = ConnectRequest::new("https", authority, path)
            .available_protocols(protocols);
        let resp = ConnectResponse::new(200);
        prop_assert!(!resp.is_protocol_valid(&req));
    }
}

// =============================================================================
// プロトコルネゴシエーション (draft-ietf-webtrans-http3-15 Section 3.3)
// =============================================================================

proptest! {
    /// Property: selected_protocol が available_protocols に含まれる場合は valid、
    /// 含まれない場合は invalid (draft-ietf-webtrans-http3-15 Section 3.3)
    #[test]
    fn prop_connect_response_protocol_selection(
        protocols in prop::collection::vec(safe_protocol_name(), 1..5),
        selected_idx in 0usize..10,
        extra_protocol in safe_protocol_name(),
    ) {
        let req = ConnectRequest::new("https", "example.com", "/")
            .available_protocols(protocols.clone());

        if selected_idx < protocols.len() {
            let resp = ConnectResponse::new(200)
                .with_protocol(&protocols[selected_idx]);
            prop_assert!(resp.is_protocol_valid(&req));
        }

        if !protocols.contains(&extra_protocol) {
            let resp = ConnectResponse::new(200)
                .with_protocol(&extra_protocol);
            prop_assert!(!resp.is_protocol_valid(&req));
        }
    }
}
