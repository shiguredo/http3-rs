//! ConnectRequest / ConnectResponse のバリデーションとプロトコルネゴシエーション
//! (draft-ietf-webtrans-http3-15 Section 3.2, 3.3)

use pbt::strategies::sample_len;
use shiguredo_http3::webtransport::{ConnectRequest, ConnectResponse};

/// 任意の非空文字列 (HTTP ヘッダー値として安全な文字のみ)
fn non_empty_string(ctx: &mut noprop::TestCaseContext) -> String {
    let len = sample_len(ctx, 1..=63);
    let mut s = String::new();
    for _ in 0..len {
        s.push((0x20 + noprop::sample_usize_in(ctx, 0..=0x5f)) as u8 as char);
    }
    s
}

/// プロトコル名として安全な文字列を生成
///
/// Structured Fields List のカンマ区切りや
/// クォート文字列のエスケープ対象文字 (',', '"', '\\') を除外する。
fn safe_protocol_name(ctx: &mut noprop::TestCaseContext) -> String {
    let chars = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-./+";
    let len = sample_len(ctx, 1..=32);
    let mut s = String::new();
    for _ in 0..len {
        s.push(noprop::sample_choice(ctx, chars) as char);
    }
    s
}

// =============================================================================
// ConnectRequest バリデーション (draft-ietf-webtrans-http3-15 Section 3.2)
// =============================================================================

/// Property: 有効なリクエストは validate() が Ok を返す
#[test]
fn prop_connect_request_valid() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_CONNECT_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let authority = non_empty_string(ctx);
        let path = non_empty_string(ctx);
        let req = ConnectRequest::new("https", authority, path);
        assert!(req.validate().is_ok());
        Ok(())
    })?;
    Ok(())
}

/// Property: scheme が "https" 以外なら InvalidScheme
#[test]
fn prop_connect_request_invalid_scheme() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_CONNECT_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let scheme = noprop::sample_with_rejection(ctx, 64, |ctx| {
            let s = non_empty_string(ctx);
            (s != "https").then_some(s)
        });
        let authority = non_empty_string(ctx);
        let path = non_empty_string(ctx);
        let req = ConnectRequest::new(scheme, authority, path);
        assert!(req.validate().is_err());
        Ok(())
    })?;
    Ok(())
}

/// Property: WT-Available-Protocols の文字列型のみを抽出
#[test]
fn prop_parse_available_protocols_strings_only() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_CONNECT_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let protocol_count = noprop::sample_usize_in(ctx, 1..5);
        let mut protocols = Vec::new();
        for _ in 0..protocol_count {
            protocols.push(safe_protocol_name(ctx));
        }

        let header_value = protocols
            .iter()
            .map(|p| format!("\"{}\"", p))
            .collect::<Vec<_>>()
            .join(", ");

        let result = ConnectRequest::parse_available_protocols(&header_value);
        assert_eq!(result, protocols);
        Ok(())
    })?;
    Ok(())
}

/// Property: WT-Protocol 文字列型のラウンドトリップ
#[test]
fn prop_parse_protocol_string_roundtrip() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_CONNECT_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let proto = safe_protocol_name(ctx);
        let header_value = format!("\"{}\"", proto);
        let result = ConnectResponse::parse_protocol(&header_value);
        assert_eq!(result, Some(proto));
        Ok(())
    })?;
    Ok(())
}

/// Property: available_protocols が空なら selected_protocol なしで true
#[test]
fn prop_connect_response_no_protocol_no_negotiation_valid() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_CONNECT_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let authority = non_empty_string(ctx);
        let path = non_empty_string(ctx);
        let req = ConnectRequest::new("https", authority, path);
        let resp = ConnectResponse::new(200);
        assert!(resp.is_protocol_valid(&req));
        Ok(())
    })?;
    Ok(())
}

/// Property: available_protocols が非空なら selected_protocol なしで false (draft-15)
#[test]
fn prop_connect_response_no_protocol_with_negotiation_invalid() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_CONNECT_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let authority = non_empty_string(ctx);
        let path = non_empty_string(ctx);
        let protocol_count = noprop::sample_usize_in(ctx, 1..5);
        let mut protocols = Vec::new();
        for _ in 0..protocol_count {
            protocols.push(non_empty_string(ctx));
        }
        let req = ConnectRequest::new("https", authority, path).available_protocols(protocols);
        let resp = ConnectResponse::new(200);
        assert!(!resp.is_protocol_valid(&req));
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// プロトコルネゴシエーション (draft-ietf-webtrans-http3-15 Section 3.3)
// =============================================================================

/// Property: selected_protocol が available_protocols に含まれる場合は valid、
/// 含まれない場合は invalid (draft-ietf-webtrans-http3-15 Section 3.3)
#[test]
fn prop_connect_response_protocol_selection() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_CONNECT_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let protocol_count = noprop::sample_usize_in(ctx, 1..5);
        let mut protocols = Vec::new();
        for _ in 0..protocol_count {
            protocols.push(safe_protocol_name(ctx));
        }
        let selected_idx = noprop::sample_usize_in(ctx, 0..10);
        let extra_protocol = safe_protocol_name(ctx);

        let req =
            ConnectRequest::new("https", "example.com", "/").available_protocols(protocols.clone());

        if selected_idx < protocols.len() {
            let resp = ConnectResponse::new(200).with_protocol(&protocols[selected_idx]);
            assert!(resp.is_protocol_valid(&req));
        }

        if !protocols.contains(&extra_protocol) {
            let resp = ConnectResponse::new(200).with_protocol(&extra_protocol);
            assert!(!resp.is_protocol_valid(&req));
        }
        Ok(())
    })?;
    Ok(())
}
