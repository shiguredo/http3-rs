//! ApplicationErrorCode / Error のプロパティと CloseSession メッセージ長制約
//! (draft-ietf-webtrans-http3-15 Section 6, 9.5)

use proptest::prelude::*;
use shiguredo_http3::webtransport::{ApplicationErrorCode, Error, ErrorCode};

prop_compose! {
    /// 有効なエラーメッセージ (最大 1024 バイト)
    fn valid_error_message()(
        len in 0usize..=1024,
    )(
        msg in prop::collection::vec(0x20u8..0x7f, len)
    ) -> String {
        String::from_utf8(msg).expect("ASCII range bytes are always valid UTF-8")
    }
}

prop_compose! {
    /// 1024 バイト超のメッセージ (ASCII)
    fn long_message()(
        len in 1025usize..2048,
    )(
        msg in prop::collection::vec(0x20u8..0x7f, len)
    ) -> String {
        String::from_utf8(msg).expect("ASCII range bytes are always valid UTF-8")
    }
}

// =============================================================================
// ApplicationErrorCode (draft-ietf-webtrans-http3-15 Section 9.5)
// =============================================================================

proptest! {
    /// Property: アプリケーションエラーコードのラウンドトリップ
    #[test]
    fn prop_application_error_code_roundtrip(app_code in any::<u32>()) {
        let http3_code = ApplicationErrorCode::to_http3_code(app_code);
        let back = ApplicationErrorCode::from_http3_code(http3_code);

        prop_assert_eq!(back, Some(app_code));
    }

    /// Property: HTTP/3 コードが範囲内
    #[test]
    fn prop_http3_code_in_range(app_code in any::<u32>()) {
        let http3_code = ApplicationErrorCode::to_http3_code(app_code);

        prop_assert!(
            http3_code >= ApplicationErrorCode::FIRST,
            "HTTP/3 code {} < FIRST {}",
            http3_code, ApplicationErrorCode::FIRST
        );
        prop_assert!(
            http3_code <= ApplicationErrorCode::LAST,
            "HTTP/3 code {} > LAST {}",
            http3_code, ApplicationErrorCode::LAST
        );
    }

    /// Property: is_application_error が正しく判定
    #[test]
    fn prop_is_application_error_consistent(app_code in any::<u32>()) {
        let http3_code = ApplicationErrorCode::to_http3_code(app_code);
        prop_assert!(ApplicationErrorCode::is_application_error(http3_code));
    }

    /// Property: Error::application のラウンドトリップ
    #[test]
    fn prop_error_application_roundtrip(
        code in any::<u32>(),
        message in valid_error_message(),
    ) {
        let error = Error::application(code, message.clone());

        if let Error::Application {
            code: dec_code,
            message: dec_msg,
        } = error
        {
            prop_assert_eq!(dec_code, code);
            prop_assert!(dec_msg.len() <= 1024);
            if message.len() <= 1024 {
                prop_assert_eq!(dec_msg, message);
            }
        } else {
            prop_assert!(false, "Expected Application error");
        }
    }

    /// Property: Error の HTTP/3 コード変換
    #[test]
    fn prop_error_to_http3_code(code in any::<u32>()) {
        let error = Error::Application {
            code,
            message: String::new(),
        };

        let http3_code = error.to_http3_code();
        prop_assert_eq!(http3_code, ApplicationErrorCode::to_http3_code(code));
    }

    /// Property: Error::from_http3_code で Protocol エラーを正しく復元
    #[test]
    fn prop_error_from_http3_code_protocol(
        error_code in prop::sample::select(vec![
            ErrorCode::BufferedStreamRejected,
            ErrorCode::SessionGone,
            ErrorCode::FlowControlError,
        ])
    ) {
        let http3_code = error_code as u64;
        let error = Error::from_http3_code(http3_code);

        prop_assert_eq!(error, Error::Protocol(error_code));
    }
}

// =============================================================================
// CloseSession メッセージ長制約 (draft-ietf-webtrans-http3-15 Section 6)
// =============================================================================

proptest! {
    /// Property: 1024 バイト以下のメッセージはそのまま保存
    #[test]
    fn prop_close_session_message_preserved(message in valid_error_message()) {
        let error = Error::application(0, message.clone());

        if let Error::Application { message: msg, .. } = error {
            prop_assert_eq!(msg, message);
        } else {
            prop_assert!(false, "Expected Application error");
        }
    }

    /// Property: 1024 バイト超のメッセージは切り詰められる
    #[test]
    fn prop_close_session_message_truncated(message in long_message()) {
        let error = Error::application(0, message.clone());

        if let Error::Application { message: msg, .. } = error {
            prop_assert!(msg.len() <= 1024, "Message should be truncated to 1024 bytes");
            prop_assert!(message.starts_with(&msg), "Truncated message should be a prefix");
        } else {
            prop_assert!(false, "Expected Application error");
        }
    }

    /// Property: UTF-8 マルチバイト文字の境界で正しく切り詰め
    #[test]
    fn prop_close_session_message_utf8_boundary(
        prefix_len in 1020usize..1024,
    ) {
        let prefix: String = (0..prefix_len).map(|_| 'a').collect();
        let message = format!("{}日本語テスト", prefix);

        let error = Error::application(0, message);

        if let Error::Application { message: msg, .. } = error {
            prop_assert!(msg.len() <= 1024);
            prop_assert!(msg.is_ascii() || msg.chars().count() > 0);
        } else {
            prop_assert!(false, "Expected Application error");
        }
    }
}

// =============================================================================
// 予約コードポイント衝突回避 (draft-ietf-webtrans-http3-15 Section 9.5)
// =============================================================================

proptest! {
    /// Property: to_http3_code() の結果が予約コードポイント (x - 0x21) % 0x1f == 0 でない
    #[test]
    fn prop_application_error_code_no_reserved_collision(app_code in any::<u32>()) {
        let http3_code = ApplicationErrorCode::to_http3_code(app_code);

        prop_assert!(
            !(http3_code.wrapping_sub(0x21)).is_multiple_of(0x1f),
            "HTTP/3 code {} is a reserved codepoint",
            http3_code
        );
    }
}
