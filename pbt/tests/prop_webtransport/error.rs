//! ApplicationErrorCode / Error のプロパティと CloseSession メッセージ長制約
//! (draft-ietf-webtrans-http3-15 Section 6, 9.5)

use pbt::strategies::sample_len;
use shiguredo_http3::webtransport::{ApplicationErrorCode, Error, ErrorCode};

/// 有効なエラーメッセージ (最大 1024 バイト)
fn valid_error_message(ctx: &mut noprop::TestCaseContext) -> String {
    let len = sample_len(ctx, 0..=1024);
    let mut msg = Vec::new();
    for _ in 0..len {
        msg.push(0x20 + noprop::sample_usize_in(ctx, 0..=0x5f) as u8);
    }
    String::from_utf8(msg).expect("ASCII range bytes are always valid UTF-8")
}

/// 1024 バイト超のメッセージ (ASCII)
fn long_message(ctx: &mut noprop::TestCaseContext) -> String {
    let len = sample_len(ctx, 1025..=2047);
    let mut msg = Vec::new();
    for _ in 0..len {
        msg.push(0x20 + noprop::sample_usize_in(ctx, 0..=0x5f) as u8);
    }
    String::from_utf8(msg).expect("ASCII range bytes are always valid UTF-8")
}

// =============================================================================
// ApplicationErrorCode (draft-ietf-webtrans-http3-15 Section 9.5)
// =============================================================================

/// Property: アプリケーションエラーコードのラウンドトリップ
#[test]
fn prop_application_error_code_roundtrip() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_ERROR_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let app_code = noprop::sample_u32(ctx);
        let http3_code = ApplicationErrorCode::to_http3_code(app_code);
        let back = ApplicationErrorCode::from_http3_code(http3_code);

        assert_eq!(back, Some(app_code));
        Ok(())
    })?;
    Ok(())
}

/// Property: HTTP/3 コードが範囲内
#[test]
fn prop_http3_code_in_range() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_ERROR_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let app_code = noprop::sample_u32(ctx);
        let http3_code = ApplicationErrorCode::to_http3_code(app_code);

        assert!(
            http3_code >= ApplicationErrorCode::FIRST,
            "HTTP/3 code {} < FIRST {}",
            http3_code,
            ApplicationErrorCode::FIRST
        );
        assert!(
            http3_code <= ApplicationErrorCode::LAST,
            "HTTP/3 code {} > LAST {}",
            http3_code,
            ApplicationErrorCode::LAST
        );
        Ok(())
    })?;
    Ok(())
}

/// Property: is_application_error が正しく判定
#[test]
fn prop_is_application_error_consistent() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_ERROR_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let app_code = noprop::sample_u32(ctx);
        let http3_code = ApplicationErrorCode::to_http3_code(app_code);
        assert!(ApplicationErrorCode::is_application_error(http3_code));
        Ok(())
    })?;
    Ok(())
}

/// Property: Error::application のラウンドトリップ
#[test]
fn prop_error_application_roundtrip() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_ERROR_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let code = noprop::sample_u32(ctx);
        let message = valid_error_message(ctx);
        let error = Error::application(code, message.clone());

        if let Error::Application {
            code: dec_code,
            message: dec_msg,
        } = error
        {
            assert_eq!(dec_code, code);
            assert!(dec_msg.len() <= 1024);
            if message.len() <= 1024 {
                assert_eq!(dec_msg, message);
            }
        } else {
            panic!("Expected Application error");
        }
        Ok(())
    })?;
    Ok(())
}

/// Property: Error の HTTP/3 コード変換
#[test]
fn prop_error_to_http3_code() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_ERROR_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let code = noprop::sample_u32(ctx);
        let error = Error::Application {
            code,
            message: String::new(),
        };

        let http3_code = error.to_http3_code();
        assert_eq!(http3_code, ApplicationErrorCode::to_http3_code(code));
        Ok(())
    })?;
    Ok(())
}

/// Property: Error::from_http3_code で Protocol エラーを正しく復元
#[test]
fn prop_error_from_http3_code_protocol() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_ERROR_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let error_code = noprop::sample_choice(
            ctx,
            &[
                ErrorCode::BufferedStreamRejected,
                ErrorCode::SessionGone,
                ErrorCode::FlowControlError,
            ],
        );
        let http3_code = error_code as u64;
        let error = Error::from_http3_code(http3_code);

        assert_eq!(error, Error::Protocol(error_code));
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// CloseSession メッセージ長制約 (draft-ietf-webtrans-http3-15 Section 6)
// =============================================================================

/// Property: 1024 バイト以下のメッセージはそのまま保存
#[test]
fn prop_close_session_message_preserved() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_ERROR_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let message = valid_error_message(ctx);
        let error = Error::application(0, message.clone());

        if let Error::Application { message: msg, .. } = error {
            assert_eq!(msg, message);
        } else {
            panic!("Expected Application error");
        }
        Ok(())
    })?;
    Ok(())
}

/// Property: 1024 バイト超のメッセージは切り詰められる
#[test]
fn prop_close_session_message_truncated() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_ERROR_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let message = long_message(ctx);
        let error = Error::application(0, message.clone());

        if let Error::Application { message: msg, .. } = error {
            assert!(
                msg.len() <= 1024,
                "Message should be truncated to 1024 bytes"
            );
            assert!(
                message.starts_with(&msg),
                "Truncated message should be a prefix"
            );
        } else {
            panic!("Expected Application error");
        }
        Ok(())
    })?;
    Ok(())
}

/// Property: UTF-8 マルチバイト文字の境界で正しく切り詰め
#[test]
fn prop_close_session_message_utf8_boundary() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_ERROR_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let prefix_len = noprop::sample_usize_in(ctx, 1020..1024);
        let prefix: String = (0..prefix_len).map(|_| 'a').collect();
        let message = format!("{}日本語テスト", prefix);

        let error = Error::application(0, message);

        if let Error::Application { message: msg, .. } = error {
            assert!(msg.len() <= 1024);
            assert!(msg.is_ascii() || msg.chars().count() > 0);
        } else {
            panic!("Expected Application error");
        }
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// 予約コードポイント衝突回避 (draft-ietf-webtrans-http3-15 Section 9.5)
// =============================================================================

/// Property: to_http3_code() の結果が予約コードポイント (x - 0x21) % 0x1f == 0 でない
#[test]
fn prop_application_error_code_no_reserved_collision() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_ERROR_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let app_code = noprop::sample_u32(ctx);
        let http3_code = ApplicationErrorCode::to_http3_code(app_code);

        assert!(
            !(http3_code.wrapping_sub(0x21)).is_multiple_of(0x1f),
            "HTTP/3 code {} is a reserved codepoint",
            http3_code
        );
        Ok(())
    })?;
    Ok(())
}
