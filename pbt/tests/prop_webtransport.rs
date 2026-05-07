//! Property-Based Testing for WebTransport (draft-ietf-webtrans-http3-15)

use proptest::prelude::*;
use shiguredo_http3::webtransport::{
    ApplicationErrorCode, Capsule, ConnectRequest, ConnectResponse, Datagram, Error, ErrorCode,
    FlowControlLimits, MAX_STREAMS_LIMIT, Session, SessionState, Settings, SettingsId, Stream,
    StreamHeader, stream_type,
};

// =============================================================================
// Capsule Properties
// =============================================================================

prop_compose! {
    /// 有効なエラーコード (32-bit)
    fn valid_error_code()(code in any::<u32>()) -> u32 {
        code
    }
}

prop_compose! {
    /// 有効なエラーメッセージ (最大 1024 バイト)
    fn valid_error_message()(
        len in 0usize..=1024,
    )(
        msg in prop::collection::vec(0x20u8..0x7f, len)
    ) -> String {
        String::from_utf8(msg).unwrap_or_default()
    }
}

prop_compose! {
    /// 有効な最大値 (可変長整数範囲)
    fn valid_maximum()(max in 0u64..=(1 << 62) - 1) -> u64 {
        max
    }
}

prop_compose! {
    /// 有効なセッション ID (client-initiated bidirectional stream ID: id % 4 == 0)
    fn valid_session_id()(id in 0u64..250_000) -> u64 {
        id * 4
    }
}

prop_compose! {
    /// 有効な Unknown Capsule
    fn valid_unknown_capsule()(
        capsule_type in 0x100000u64..0x200000,
        payload_len in 0usize..256,
    )(
        capsule_type in Just(capsule_type),
        payload in prop::collection::vec(any::<u8>(), payload_len)
    ) -> Capsule {
        Capsule::Unknown { capsule_type, payload: bytes::Bytes::from(payload) }
    }
}

proptest! {
    /// Property: CloseSession Capsule のラウンドトリップ
    #[test]
    fn prop_close_session_roundtrip(
        error_code in valid_error_code(),
        message in valid_error_message(),
    ) {
        let capsule = Capsule::CloseSession {
            error_code,
            message: message.clone(),
        };

        let mut buf = Vec::new();
        capsule.encode(&mut buf);

        let (decoded, consumed) = Capsule::decode(&buf).unwrap().unwrap();
        prop_assert_eq!(consumed, buf.len());

        if let Capsule::CloseSession {
            error_code: dec_code,
            message: dec_msg,
        } = decoded
        {
            prop_assert_eq!(dec_code, error_code);
            prop_assert_eq!(dec_msg, message);
        } else {
            prop_assert!(false, "Expected CloseSession capsule");
        }
    }

    /// Property: DrainSession Capsule のラウンドトリップ
    #[test]
    fn prop_drain_session_roundtrip(_dummy in Just(())) {
        let capsule = Capsule::DrainSession;

        let mut buf = Vec::new();
        capsule.encode(&mut buf);

        let (decoded, consumed) = Capsule::decode(&buf).unwrap().unwrap();
        prop_assert_eq!(consumed, buf.len());
        prop_assert_eq!(decoded, Capsule::DrainSession);
    }

    /// Property: MaxData Capsule のラウンドトリップ
    #[test]
    fn prop_max_data_roundtrip(maximum in valid_maximum()) {
        let capsule = Capsule::MaxData { maximum };

        let mut buf = Vec::new();
        capsule.encode(&mut buf);

        let (decoded, consumed) = Capsule::decode(&buf).unwrap().unwrap();
        prop_assert_eq!(consumed, buf.len());

        if let Capsule::MaxData { maximum: dec_max } = decoded {
            prop_assert_eq!(dec_max, maximum);
        } else {
            prop_assert!(false, "Expected MaxData capsule");
        }
    }

    /// Property: MaxStreams Capsule のラウンドトリップ
    #[test]
    fn prop_max_streams_roundtrip(
        bidirectional in any::<bool>(),
        maximum in valid_maximum(),
    ) {
        let capsule = Capsule::MaxStreams {
            bidirectional,
            maximum,
        };

        let mut buf = Vec::new();
        capsule.encode(&mut buf);

        let (decoded, consumed) = Capsule::decode(&buf).unwrap().unwrap();
        prop_assert_eq!(consumed, buf.len());

        if let Capsule::MaxStreams {
            bidirectional: dec_bidi,
            maximum: dec_max,
        } = decoded
        {
            prop_assert_eq!(dec_bidi, bidirectional);
            prop_assert_eq!(dec_max, maximum);
        } else {
            prop_assert!(false, "Expected MaxStreams capsule");
        }
    }

    /// Property: DataBlocked Capsule のラウンドトリップ
    #[test]
    fn prop_data_blocked_roundtrip(maximum in valid_maximum()) {
        let capsule = Capsule::DataBlocked { maximum };

        let mut buf = Vec::new();
        capsule.encode(&mut buf);

        let (decoded, consumed) = Capsule::decode(&buf).unwrap().unwrap();
        prop_assert_eq!(consumed, buf.len());

        if let Capsule::DataBlocked { maximum: dec_max } = decoded {
            prop_assert_eq!(dec_max, maximum);
        } else {
            prop_assert!(false, "Expected DataBlocked capsule");
        }
    }

    /// Property: StreamsBlocked Capsule のラウンドトリップ
    #[test]
    fn prop_streams_blocked_roundtrip(
        bidirectional in any::<bool>(),
        maximum in valid_maximum(),
    ) {
        let capsule = Capsule::StreamsBlocked {
            bidirectional,
            maximum,
        };

        let mut buf = Vec::new();
        capsule.encode(&mut buf);

        let (decoded, consumed) = Capsule::decode(&buf).unwrap().unwrap();
        prop_assert_eq!(consumed, buf.len());

        if let Capsule::StreamsBlocked {
            bidirectional: dec_bidi,
            maximum: dec_max,
        } = decoded
        {
            prop_assert_eq!(dec_bidi, bidirectional);
            prop_assert_eq!(dec_max, maximum);
        } else {
            prop_assert!(false, "Expected StreamsBlocked capsule");
        }
    }

    /// Property: Unknown Capsule のラウンドトリップ
    #[test]
    fn prop_unknown_capsule_roundtrip(capsule in valid_unknown_capsule()) {
        let mut buf = Vec::new();
        capsule.encode(&mut buf);

        let (decoded, consumed) = Capsule::decode(&buf).unwrap().unwrap();
        prop_assert_eq!(consumed, buf.len());
        prop_assert_eq!(decoded, capsule);
    }

    /// Property: Capsule の capsule_type() が正しい値を返す
    #[test]
    fn prop_capsule_type_consistent(
        error_code in valid_error_code(),
        maximum in 0u64..1000,
        bidirectional in any::<bool>(),
    ) {
        let close = Capsule::CloseSession {
            error_code,
            message: String::new(),
        };
        prop_assert_eq!(close.capsule_type(), 0x2843);

        let drain = Capsule::DrainSession;
        prop_assert_eq!(drain.capsule_type(), 0x78ae);

        let max_data = Capsule::MaxData { maximum };
        prop_assert_eq!(max_data.capsule_type(), 0x190B4D3D);

        let max_streams = Capsule::MaxStreams { bidirectional, maximum };
        if bidirectional {
            prop_assert_eq!(max_streams.capsule_type(), 0x190B4D3F);
        } else {
            prop_assert_eq!(max_streams.capsule_type(), 0x190B4D40);
        }

        let data_blocked = Capsule::DataBlocked { maximum };
        prop_assert_eq!(data_blocked.capsule_type(), 0x190B4D41);

        let streams_blocked = Capsule::StreamsBlocked { bidirectional, maximum };
        if bidirectional {
            prop_assert_eq!(streams_blocked.capsule_type(), 0x190B4D43);
        } else {
            prop_assert_eq!(streams_blocked.capsule_type(), 0x190B4D44);
        }
    }
}

// =============================================================================
// ApplicationErrorCode Properties
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
            // メッセージは最大 1024 バイトに切り詰められる
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
// StreamHeader Properties
// =============================================================================

proptest! {
    /// Property: 単方向ストリームヘッダーのラウンドトリップ
    #[test]
    fn prop_unidirectional_header_roundtrip(session_id in valid_session_id()) {
        let header = StreamHeader::new(session_id).unwrap();

        let mut buf = Vec::new();
        header.encode_unidirectional(&mut buf);

        let (decoded, consumed) = StreamHeader::decode_unidirectional(&buf).unwrap();
        prop_assert_eq!(consumed, buf.len());
        prop_assert_eq!(decoded.session_id, session_id);
    }

    /// Property: 双方向ストリームヘッダーのラウンドトリップ
    #[test]
    fn prop_bidirectional_header_roundtrip(session_id in valid_session_id()) {
        let header = StreamHeader::new(session_id).unwrap();

        let mut buf = Vec::new();
        header.encode_bidirectional(&mut buf);

        let (decoded, consumed) = StreamHeader::decode_bidirectional(&buf).unwrap();
        prop_assert_eq!(consumed, buf.len());
        prop_assert_eq!(decoded.session_id, session_id);
    }

    /// Property: 単方向ストリームヘッダーのフォーマット検証
    #[test]
    fn prop_unidirectional_header_format(session_id in valid_session_id()) {
        let header = StreamHeader::new(session_id).unwrap();

        let mut buf = Vec::new();
        header.encode_unidirectional(&mut buf);

        // 先頭は Stream Type (0x54) の varint エンコード
        // 0x54 = 84 は 2 バイト varint: 0x40 | (84 >> 8), 84 & 0xff = 0x40, 0x54
        prop_assert_eq!(buf[0], 0x40, "First byte should be 0x40 for 2-byte varint");
        prop_assert_eq!(buf[1], 0x54, "Second byte should be 0x54 (stream type)");
    }

    /// Property: 双方向ストリームヘッダーのフォーマット検証
    #[test]
    fn prop_bidirectional_header_format(session_id in valid_session_id()) {
        let header = StreamHeader::new(session_id).unwrap();

        let mut buf = Vec::new();
        header.encode_bidirectional(&mut buf);

        // 先頭は Signal Value (0x41) の varint エンコード
        // 0x41 = 65 は 2 バイト varint: 0x40 | (65 >> 8), 65 & 0xff = 0x40, 0x41
        prop_assert_eq!(buf[0], 0x40, "First byte should be 0x40 for 2-byte varint");
        prop_assert_eq!(buf[1], 0x41, "Second byte should be 0x41 (signal value)");
    }

    /// Property: encoded_size が実際のエンコード長と一致
    #[test]
    fn prop_encoded_size_matches_actual(session_id in valid_session_id()) {
        let header = StreamHeader::new(session_id).unwrap();

        let predicted_size = header.encoded_size();

        let mut buf_uni = Vec::new();
        header.encode_unidirectional(&mut buf_uni);

        let mut buf_bidi = Vec::new();
        header.encode_bidirectional(&mut buf_bidi);

        // 単方向と双方向は Signal/Type 部分が同じサイズなので同じ長さになる
        prop_assert_eq!(buf_uni.len(), buf_bidi.len());
        prop_assert_eq!(predicted_size, buf_uni.len());
    }
}

// =============================================================================
// Stream ID Properties (RFC Section 4)
// =============================================================================

proptest! {
    /// Property: クライアント開始/サーバー開始の判定は相互排他的
    #[test]
    fn prop_stream_id_client_server_initiated(stream_id in any::<u64>()) {
        let is_client = stream_type::is_client_initiated(stream_id);
        let is_server = stream_type::is_server_initiated(stream_id);

        // 相互排他的
        prop_assert!(is_client != is_server, "Stream ID must be either client or server initiated");
    }

    /// Property: 双方向/単方向の判定は相互排他的
    #[test]
    fn prop_stream_id_bidirectional_unidirectional(stream_id in any::<u64>()) {
        let is_bidi = stream_type::is_bidirectional(stream_id);
        let is_uni = stream_type::is_unidirectional(stream_id);

        // 相互排他的
        prop_assert!(is_bidi != is_uni, "Stream ID must be either bidirectional or unidirectional");
    }

    /// Property: 全ストリームタイプの組み合わせ (4パターン)
    #[test]
    fn prop_stream_id_all_combinations(base_id in 0u64..1_000_000) {
        // Client-initiated bidirectional (0b00)
        let id_00 = base_id * 4;
        prop_assert!(stream_type::is_client_initiated(id_00));
        prop_assert!(stream_type::is_bidirectional(id_00));

        // Server-initiated bidirectional (0b01)
        let id_01 = base_id * 4 + 1;
        prop_assert!(stream_type::is_server_initiated(id_01));
        prop_assert!(stream_type::is_bidirectional(id_01));

        // Client-initiated unidirectional (0b10)
        let id_10 = base_id * 4 + 2;
        prop_assert!(stream_type::is_client_initiated(id_10));
        prop_assert!(stream_type::is_unidirectional(id_10));

        // Server-initiated unidirectional (0b11)
        let id_11 = base_id * 4 + 3;
        prop_assert!(stream_type::is_server_initiated(id_11));
        prop_assert!(stream_type::is_unidirectional(id_11));
    }
}

// =============================================================================
// Session State Transition Properties (RFC Section 3, 6)
// =============================================================================

proptest! {
    /// Property: 有効な状態遷移パス (Pending → Connecting → Established → Draining → Closed)
    #[test]
    fn prop_session_valid_transitions(_dummy in Just(())) {
        let mut session = Session::new(0);
        prop_assert_eq!(session.state(), SessionState::Pending);

        session.set_connecting();
        prop_assert_eq!(session.state(), SessionState::Connecting);

        session.set_established();
        prop_assert_eq!(session.state(), SessionState::Established);

        session.set_draining();
        prop_assert_eq!(session.state(), SessionState::Draining);

        session.close(None);
        prop_assert_eq!(session.state(), SessionState::Closed);
    }

    /// Property: Pending → Established の直接遷移も有効
    #[test]
    fn prop_session_direct_establish(_dummy in Just(())) {
        let mut session = Session::new(0);
        prop_assert_eq!(session.state(), SessionState::Pending);

        session.set_established();
        prop_assert_eq!(session.state(), SessionState::Established);

        session.close(None);
        prop_assert_eq!(session.state(), SessionState::Closed);
    }

    /// Property: 各状態でのストリーム作成可否
    #[test]
    fn prop_session_state_can_create_stream(_dummy in Just(())) {
        prop_assert!(!SessionState::Pending.can_create_stream());
        prop_assert!(!SessionState::Connecting.can_create_stream());
        prop_assert!(SessionState::Established.can_create_stream());
        prop_assert!(SessionState::Draining.can_create_stream());
        prop_assert!(!SessionState::Closed.can_create_stream());
    }

    /// Property: 各状態での送信可否
    #[test]
    fn prop_session_state_can_send(_dummy in Just(())) {
        prop_assert!(!SessionState::Pending.can_send());
        prop_assert!(!SessionState::Connecting.can_send());
        prop_assert!(SessionState::Established.can_send());
        prop_assert!(SessionState::Draining.can_send());
        prop_assert!(!SessionState::Closed.can_send());
    }
}

// =============================================================================
// Flow Control Non-Monotonic Detection (RFC Section 5.6.2, 5.6.4)
// =============================================================================

proptest! {
    /// Property: MaxData が減少した場合にエラー
    #[test]
    fn prop_session_max_data_non_monotonic_error(
        initial in 100u64..10000,
        decrease in 1u64..100,
    ) {
        let mut session = Session::new(0);

        // 初期値を設定
        let result = session.process_capsule(&Capsule::MaxData { maximum: initial });
        prop_assert!(result.is_ok());
        prop_assert_eq!(session.remote_limits().max_data, initial);

        // 減少した値を送信するとエラー
        let smaller = initial.saturating_sub(decrease);
        let result = session.process_capsule(&Capsule::MaxData { maximum: smaller });
        prop_assert!(result.is_err());

        // 元の値が保持されている
        prop_assert_eq!(session.remote_limits().max_data, initial);
    }

    /// Property: MaxData が増加または同値なら成功
    #[test]
    fn prop_session_max_data_monotonic_ok(
        initial in 0u64..10000,
        increase in 0u64..10000,
    ) {
        let mut session = Session::new(0);

        // 初期値を設定
        let result = session.process_capsule(&Capsule::MaxData { maximum: initial });
        prop_assert!(result.is_ok());

        // 増加または同値なら成功
        let larger = initial.saturating_add(increase);
        let result = session.process_capsule(&Capsule::MaxData { maximum: larger });
        prop_assert!(result.is_ok());
        prop_assert_eq!(session.remote_limits().max_data, larger);
    }

    /// Property: MaxStreams (双方向) が減少した場合にエラー
    #[test]
    fn prop_session_max_streams_bidi_non_monotonic_error(
        initial in 100u64..10000,
        decrease in 1u64..100,
    ) {
        let mut session = Session::new(0);

        // 初期値を設定
        let result = session.process_capsule(&Capsule::MaxStreams {
            bidirectional: true,
            maximum: initial,
        });
        prop_assert!(result.is_ok());

        // 減少した値を送信するとエラー
        let smaller = initial.saturating_sub(decrease);
        let result = session.process_capsule(&Capsule::MaxStreams {
            bidirectional: true,
            maximum: smaller,
        });
        prop_assert!(result.is_err());
    }

    /// Property: MaxStreams (単方向) が減少した場合にエラー
    #[test]
    fn prop_session_max_streams_uni_non_monotonic_error(
        initial in 100u64..10000,
        decrease in 1u64..100,
    ) {
        let mut session = Session::new(0);

        // 初期値を設定
        let result = session.process_capsule(&Capsule::MaxStreams {
            bidirectional: false,
            maximum: initial,
        });
        prop_assert!(result.is_ok());

        // 減少した値を送信するとエラー
        let smaller = initial.saturating_sub(decrease);
        let result = session.process_capsule(&Capsule::MaxStreams {
            bidirectional: false,
            maximum: smaller,
        });
        prop_assert!(result.is_err());
    }
}

// =============================================================================
// CloseSession Message Length Limit (RFC Section 6)
// =============================================================================

prop_compose! {
    /// 1024 バイト以下のメッセージ (ASCII)
    fn short_message()(
        len in 0usize..=1024,
    )(
        msg in prop::collection::vec(0x20u8..0x7f, len)
    ) -> String {
        String::from_utf8(msg).unwrap_or_default()
    }
}

prop_compose! {
    /// 1024 バイト超のメッセージ (ASCII)
    fn long_message()(
        len in 1025usize..2048,
    )(
        msg in prop::collection::vec(0x20u8..0x7f, len)
    ) -> String {
        String::from_utf8(msg).unwrap_or_default()
    }
}

proptest! {
    /// Property: 1024 バイト以下のメッセージはそのまま保存
    #[test]
    fn prop_close_session_message_preserved(message in short_message()) {
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
            // 切り詰められたメッセージは元のメッセージのプレフィックス
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
        // プレフィックス + マルチバイト文字 (合計 1024 超) を生成
        let prefix: String = (0..prefix_len).map(|_| 'a').collect();
        // 日本語文字 (3 バイト/文字) を追加
        let message = format!("{}日本語テスト", prefix);

        let error = Error::application(0, message);

        if let Error::Application { message: msg, .. } = error {
            prop_assert!(msg.len() <= 1024);
            // UTF-8 として有効であることを確認
            prop_assert!(msg.is_ascii() || msg.chars().count() > 0);
        } else {
            prop_assert!(false, "Expected Application error");
        }
    }
}

// =============================================================================
// Capsule Incomplete Buffer (Sans I/O behavior)
// =============================================================================

proptest! {
    /// Property: 不完全なバッファでは None を返す (CloseSession)
    #[test]
    fn prop_capsule_incomplete_buffer_close_session(
        error_code in any::<u32>(),
        message in short_message(),
        cut_ratio in 0.1f64..0.9,
    ) {
        let capsule = Capsule::CloseSession {
            error_code,
            message,
        };

        let mut buf = Vec::new();
        capsule.encode(&mut buf);

        // バッファを途中で切断
        let cut_at = ((buf.len() as f64) * cut_ratio) as usize;
        let incomplete = &buf[..cut_at.max(1)];

        let result = Capsule::decode(incomplete);
        prop_assert!(matches!(result, Ok(None)), "Incomplete buffer should return None");
    }

    /// Property: 不完全なバッファでは None を返す (MaxData)
    #[test]
    fn prop_capsule_incomplete_buffer_max_data(
        maximum in valid_maximum(),
        cut_ratio in 0.1f64..0.9,
    ) {
        let capsule = Capsule::MaxData { maximum };

        let mut buf = Vec::new();
        capsule.encode(&mut buf);

        if buf.len() > 1 {
            let cut_at = ((buf.len() as f64) * cut_ratio) as usize;
            let incomplete = &buf[..cut_at.max(1)];

            let result = Capsule::decode(incomplete);
            prop_assert!(matches!(result, Ok(None)), "Incomplete buffer should return None");
        }
    }

    /// Property: 空バッファでは None を返す
    #[test]
    fn prop_capsule_empty_buffer_returns_none(_dummy in Just(())) {
        let result = Capsule::decode(&[]);
        prop_assert!(matches!(result, Ok(None)));
    }
}

// =============================================================================
// StreamHeader Invalid Type (Sans I/O behavior)
// =============================================================================

prop_compose! {
    /// 不正な単方向ストリームタイプ (0x54 以外)
    fn invalid_unidirectional_type()(
        value in (0u64..0x54).prop_union(0x55u64..0x1000)
    ) -> u64 {
        value
    }
}

prop_compose! {
    /// 不正な双方向シグナル値 (0x41 以外)
    fn invalid_bidirectional_signal()(
        value in (0u64..0x41).prop_union(0x42u64..0x1000)
    ) -> u64 {
        value
    }
}

/// 可変長整数をエンコード
fn encode_varint_for_test(buf: &mut Vec<u8>, value: u64) {
    shiguredo_http3::varint::encode_into(buf, value);
}

proptest! {
    /// Property: 不正な stream type では単方向デコード失敗
    #[test]
    fn prop_unidirectional_header_invalid_type_fails(
        invalid_type in invalid_unidirectional_type(),
        session_id in valid_session_id(),
    ) {
        let mut buf = Vec::new();
        encode_varint_for_test(&mut buf, invalid_type);
        encode_varint_for_test(&mut buf, session_id);

        let result = StreamHeader::decode_unidirectional(&buf);
        prop_assert!(result.is_none(), "Invalid stream type should fail decode");
    }

    /// Property: 不正な signal value では双方向デコード失敗
    #[test]
    fn prop_bidirectional_header_invalid_type_fails(
        invalid_signal in invalid_bidirectional_signal(),
        session_id in valid_session_id(),
    ) {
        let mut buf = Vec::new();
        encode_varint_for_test(&mut buf, invalid_signal);
        encode_varint_for_test(&mut buf, session_id);

        let result = StreamHeader::decode_bidirectional(&buf);
        prop_assert!(result.is_none(), "Invalid signal value should fail decode");
    }

    /// Property: 空バッファでは StreamHeader デコード失敗
    #[test]
    fn prop_stream_header_empty_buffer_fails(_dummy in Just(())) {
        prop_assert!(StreamHeader::decode_unidirectional(&[]).is_none());
        prop_assert!(StreamHeader::decode_bidirectional(&[]).is_none());
    }
}

// =============================================================================
// Settings Flow Control Enabled (RFC Section 5.1)
// =============================================================================

proptest! {
    /// Property: wt_enabled のみではフロー制御無効 (draft-15)
    #[test]
    fn prop_settings_flow_control_wt_enabled_only(wt_enabled in 1u64..100) {
        let settings = Settings::new().wt_enabled(wt_enabled);
        prop_assert!(!settings.flow_control_enabled());
    }

    /// Property: INITIAL_MAX_* が 0 以外ならフロー制御有効 (draft-15)
    #[test]
    fn prop_settings_flow_control_enabled_by_initial_values(
        max_streams_uni in 1u64..100,
        max_streams_bidi in 1u64..100,
        max_data in 1u64..100000,
    ) {
        // max_streams_uni != 0
        let settings = Settings::new()
            .wt_enabled(1)
            .wt_initial_max_streams_uni(max_streams_uni);
        prop_assert!(settings.flow_control_enabled());

        // max_streams_bidi != 0
        let settings = Settings::new()
            .wt_enabled(1)
            .wt_initial_max_streams_bidi(max_streams_bidi);
        prop_assert!(settings.flow_control_enabled());

        // max_data != 0
        let settings = Settings::new()
            .wt_enabled(1)
            .wt_initial_max_data(max_data);
        prop_assert!(settings.flow_control_enabled());
    }

    /// Property: 全て 0 または wt_enabled = 0 なら無効
    #[test]
    fn prop_settings_disabled_when_zero(_dummy in Just(())) {
        let settings = Settings::new();
        prop_assert!(!settings.is_enabled());
        prop_assert!(!settings.flow_control_enabled());
    }
}

// =============================================================================
// Session Flow Control Limits (boundary tests)
// =============================================================================

proptest! {
    /// Property: ストリーム作成可否の境界テスト (単方向)
    #[test]
    fn prop_session_can_create_stream_boundary_uni(
        limit in 1u64..100,
    ) {
        let mut session = Session::new(0);
        session.set_established();
        session.remote_limits_mut().max_streams_uni = limit;

        // 制限未満: 作成可能
        session.flow_state_mut().streams_uni_opened = limit - 1;
        prop_assert!(session.can_create_unidirectional_stream());

        // 制限と等しい: 作成不可
        session.flow_state_mut().streams_uni_opened = limit;
        prop_assert!(!session.can_create_unidirectional_stream());

        // 制限超過: 作成不可
        session.flow_state_mut().streams_uni_opened = limit + 1;
        prop_assert!(!session.can_create_unidirectional_stream());
    }

    /// Property: ストリーム作成可否の境界テスト (双方向)
    #[test]
    fn prop_session_can_create_stream_boundary_bidi(
        limit in 1u64..100,
    ) {
        let mut session = Session::new(0);
        session.set_established();
        session.remote_limits_mut().max_streams_bidi = limit;

        // 制限未満: 作成可能
        session.flow_state_mut().streams_bidi_opened = limit - 1;
        prop_assert!(session.can_create_bidirectional_stream());

        // 制限と等しい: 作成不可
        session.flow_state_mut().streams_bidi_opened = limit;
        prop_assert!(!session.can_create_bidirectional_stream());
    }

    /// Property: データ送信可否の境界テスト
    #[test]
    fn prop_session_can_send_data_boundary(
        limit in 100u64..10000,
        sent in 0u64..100,
    ) {
        let mut session = Session::new(0);
        session.set_established();
        session.remote_limits_mut().max_data = limit;
        session.flow_state_mut().data_sent = sent;

        let remaining = limit - sent;

        // 残量以下: 送信可能
        prop_assert!(session.can_send_data(remaining));

        // 残量 + 1: 送信不可
        prop_assert!(!session.can_send_data(remaining + 1));
    }

    /// Property: Established/Draining 以外ではストリーム作成不可
    #[test]
    fn prop_session_cannot_create_stream_in_wrong_state(
        limit in 1u64..100,
    ) {
        let mut session = Session::new(0);
        session.remote_limits_mut().max_streams_uni = limit;
        session.remote_limits_mut().max_streams_bidi = limit;

        // Pending
        prop_assert!(!session.can_create_unidirectional_stream());
        prop_assert!(!session.can_create_bidirectional_stream());

        // Connecting
        session.set_connecting();
        prop_assert!(!session.can_create_unidirectional_stream());
        prop_assert!(!session.can_create_bidirectional_stream());

        // Closed
        session.close(None);
        prop_assert!(!session.can_create_unidirectional_stream());
        prop_assert!(!session.can_create_bidirectional_stream());
    }

    /// Property: Established/Draining 以外ではデータ送信不可
    #[test]
    fn prop_session_cannot_send_in_wrong_state(
        limit in 100u64..10000,
    ) {
        let mut session = Session::new(0);
        session.remote_limits_mut().max_data = limit;

        // Pending
        prop_assert!(!session.can_send_data(1));

        // Connecting
        session.set_connecting();
        prop_assert!(!session.can_send_data(1));

        // Closed
        session.close(None);
        prop_assert!(!session.can_send_data(1));
    }
}

// =============================================================================
// Datagram Properties (RFC Section 4.5)
// =============================================================================

prop_compose! {
    /// 4 の倍数のセッション ID (クライアント開始双方向ストリーム)
    fn valid_wt_session_id()(n in 0u64..1_000_000) -> u64 {
        n * 4
    }
}

proptest! {
    /// Property: 4 の倍数セッション ID のラウンドトリップ
    #[test]
    fn prop_datagram_roundtrip(
        session_id in valid_wt_session_id(),
        payload in prop::collection::vec(any::<u8>(), 0..1024),
    ) {
        let d = Datagram::new(session_id, payload.clone()).unwrap();

        let mut buf = Vec::new();
        d.encode(&mut buf);

        let (decoded, consumed) = Datagram::decode(&buf).unwrap();
        prop_assert_eq!(consumed, buf.len());
        prop_assert_eq!(decoded.session_id, session_id);
        prop_assert_eq!(decoded.payload, payload);
    }

    /// Property: Quarter Stream ID = session_id / 4
    #[test]
    fn prop_datagram_quarter_stream_id(session_id in valid_wt_session_id()) {
        let d = Datagram::new(session_id, vec![]).unwrap();
        prop_assert_eq!(d.quarter_stream_id(), session_id / 4);
    }

    /// Property: エンコード後の先頭 varint が Quarter Stream ID に一致
    #[test]
    fn prop_datagram_encoded_quarter_stream_id(
        session_id in valid_wt_session_id(),
        payload in prop::collection::vec(any::<u8>(), 0..64),
    ) {
        let d = Datagram::new(session_id, payload.clone()).unwrap();

        let mut buf = Vec::new();
        d.encode(&mut buf);

        // デコードして Quarter Stream ID を検証
        let (decoded, _) = Datagram::decode(&buf).unwrap();
        prop_assert_eq!(decoded.quarter_stream_id(), session_id / 4);
    }

    /// Property: 空バッファでは None を返す
    #[test]
    fn prop_datagram_empty_buffer_returns_none(_dummy in proptest::strategy::Just(())) {
        prop_assert!(Datagram::decode(&[]).is_none());
    }

    /// Property: ペイロードサイズが大きくても正しくラウンドトリップ
    #[test]
    fn prop_datagram_large_payload(
        session_id in valid_wt_session_id(),
        payload in prop::collection::vec(any::<u8>(), 1000..4096),
    ) {
        let d = Datagram::new(session_id, payload.clone()).unwrap();

        let mut buf = Vec::new();
        d.encode(&mut buf);

        let (decoded, _) = Datagram::decode(&buf).unwrap();
        prop_assert_eq!(decoded.session_id, session_id);
        prop_assert_eq!(decoded.payload, payload);
    }
}

// =============================================================================
// ConnectRequest Validation Properties (RFC Section 3.2)
// =============================================================================

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
        .unwrap()
        .prop_filter("non-empty", |s| !s.is_empty())
}

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
        // クォート文字列として組み立て
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
// Session Buffering Properties (RFC Section 4.6)
// =============================================================================

proptest! {
    /// Property: MAX_BUFFERED_STREAMS (100) までバッファリング成功
    #[test]
    fn prop_session_buffer_streams_up_to_limit(
        count in 1usize..=100,
    ) {
        let mut session = Session::new(0);

        for i in 0..count {
            prop_assert!(session.buffer_incoming_stream(i as u64 * 4, false));
        }

        let buffered = session.take_buffered_streams();
        prop_assert_eq!(buffered.len(), count);
    }

    /// Property: 101 個目のバッファリングは失敗
    #[test]
    fn prop_session_buffer_streams_over_limit(_dummy in proptest::strategy::Just(())) {
        let mut session = Session::new(0);

        for i in 0..100 {
            prop_assert!(session.buffer_incoming_stream(i as u64 * 4, false));
        }

        // 101 個目は失敗
        prop_assert!(!session.buffer_incoming_stream(99999, false));
    }

    /// Property: MAX_BUFFERED_DATAGRAMS (100) までバッファリング成功
    #[test]
    fn prop_session_buffer_datagrams_up_to_limit(
        count in 1usize..=100,
    ) {
        let mut session = Session::new(0);

        for i in 0..count {
            prop_assert!(session.buffer_datagram(vec![i as u8]));
        }

        let buffered = session.take_buffered_datagrams();
        prop_assert_eq!(buffered.len(), count);
    }

    /// Property: take 後のバッファは空になる
    #[test]
    fn prop_session_take_buffered_empties_buffer(
        stream_count in 1usize..=10,
        datagram_count in 1usize..=10,
    ) {
        let mut session = Session::new(0);

        for i in 0..stream_count {
            session.buffer_incoming_stream(i as u64 * 4, false);
        }
        for _ in 0..datagram_count {
            session.buffer_datagram(vec![0]);
        }

        let _streams = session.take_buffered_streams();
        let _datagrams = session.take_buffered_datagrams();

        prop_assert!(session.take_buffered_streams().is_empty());
        prop_assert!(session.take_buffered_datagrams().is_empty());
    }
}

// =============================================================================
// GOAWAY Properties (RFC Section 4.7)
// =============================================================================

proptest! {
    /// Property: handle_goaway 後は goaway_received が true でドレイン状態
    #[test]
    fn prop_session_goaway_sets_draining(_dummy in proptest::strategy::Just(())) {
        let mut session = Session::new(0);
        session.set_established();

        prop_assert!(!session.is_goaway_received());
        prop_assert_eq!(session.state(), SessionState::Established);

        session.handle_goaway();

        prop_assert!(session.is_goaway_received());
        prop_assert_eq!(session.state(), SessionState::Draining);
    }

    /// Property: handle_goaway 後も既存ストリームは保持され can_send() == true (RFC Section 4.7)
    #[test]
    fn prop_session_handle_goaway_preserves_streams(
        stream_count in 1usize..10,
    ) {
        let mut session = Session::new(0);
        session.set_established();
        session.remote_limits_mut().max_data = 100000;

        // ストリームを追加
        for i in 0..stream_count {
            session.add_stream(Stream::new(i as u64 * 4, 0, true));
        }

        session.handle_goaway();

        // ドレイン状態でもストリームは保持される
        prop_assert_eq!(session.stream_count(), stream_count);
        for i in 0..stream_count {
            prop_assert!(session.get_stream(i as u64 * 4).is_some());
        }
        // ドレイン状態では送信は可能
        prop_assert!(session.state().can_send());
    }
}

// =============================================================================
// Session Termination Properties (RFC Section 6)
// =============================================================================

proptest! {
    /// Property: 任意の初期状態 (Established/Draining) で on_connect_stream_closed() →
    /// is_close_session_received() == true かつ is_closed() == true (RFC Section 6)
    #[test]
    fn prop_session_on_connect_stream_closed_sets_flags(
        start_draining in any::<bool>(),
    ) {
        let mut session = Session::new(0);
        session.set_established();

        if start_draining {
            session.set_draining();
        }

        session.on_connect_stream_closed();

        prop_assert!(session.is_close_session_received());
        prop_assert!(session.is_closed());
    }

    /// Property: CloseSession Capsule 受信後に on_connect_stream_closed() →
    /// 最初のエラー情報が保持される (RFC Section 6)
    #[test]
    fn prop_session_on_connect_stream_closed_preserves_first_error(
        error_code in valid_error_code(),
        message in valid_error_message(),
    ) {
        let mut session = Session::new(0);
        session.set_established();

        // CloseSession Capsule を先に受信
        session.process_capsule(&Capsule::CloseSession {
            error_code,
            message: message.clone(),
        }).unwrap();

        let first_error = session.close_error().cloned();

        // その後 CONNECT ストリームがクローズ
        session.on_connect_stream_closed();

        // 最初のエラー情報が上書きされていない
        prop_assert_eq!(session.close_error(), first_error.as_ref());
        prop_assert!(session.is_close_session_received());
    }

    /// Property: 任意のストリーム追加/削除後、stream_ids_to_reset() が
    /// 残存ストリーム ID と一致 (RFC Section 6)
    #[test]
    fn prop_session_stream_ids_to_reset_matches_streams(
        add_count in 1usize..20,
        remove_indices in prop::collection::vec(0usize..20, 0..10),
    ) {
        let mut session = Session::new(0);
        session.set_established();

        // ストリームを追加
        let stream_ids: Vec<u64> = (0..add_count).map(|i| i as u64 * 4).collect();
        for &sid in &stream_ids {
            session.add_stream(Stream::new(sid, 0, true));
        }

        // 一部を削除
        for &idx in &remove_indices {
            if idx < stream_ids.len() {
                session.remove_stream(stream_ids[idx]);
            }
        }

        // stream_ids_to_reset() が残存ストリームと一致
        let mut reset_ids = session.stream_ids_to_reset();
        reset_ids.sort();

        let mut expected: Vec<u64> = session.streams().map(|s| s.stream_id()).collect();
        expected.sort();

        let reset_count = reset_ids.len();
        prop_assert_eq!(reset_ids, expected);
        prop_assert_eq!(reset_count, session.stream_count());
    }
}

// =============================================================================
// Capsule Interleave Properties (RFC Section 5.6, 6)
// =============================================================================

proptest! {
    /// Property: MaxData/MaxStreams/DataBlocked/StreamsBlocked をランダム順で処理しても
    /// 最終リミットが整合的 (RFC Section 5.6)
    #[test]
    fn prop_session_interleaved_capsule_processing(
        max_data_values in prop::collection::vec(0u64..10000, 1..5),
        max_streams_bidi_values in prop::collection::vec(0u64..1000, 1..5),
        max_streams_uni_values in prop::collection::vec(0u64..1000, 1..5),
    ) {
        let mut session = Session::new(0);

        // 各種 Capsule をソートして単調増加列にしてから処理
        let mut sorted_data = max_data_values.clone();
        sorted_data.sort();
        sorted_data.dedup();
        let mut sorted_bidi = max_streams_bidi_values.clone();
        sorted_bidi.sort();
        sorted_bidi.dedup();
        let mut sorted_uni = max_streams_uni_values.clone();
        sorted_uni.sort();
        sorted_uni.dedup();

        // 単調増加列は全て成功する
        for &v in &sorted_data {
            let result = session.process_capsule(&Capsule::MaxData { maximum: v });
            prop_assert!(result.is_ok());
        }
        for &v in &sorted_bidi {
            let result = session.process_capsule(&Capsule::MaxStreams {
                bidirectional: true,
                maximum: v,
            });
            prop_assert!(result.is_ok());
        }
        for &v in &sorted_uni {
            let result = session.process_capsule(&Capsule::MaxStreams {
                bidirectional: false,
                maximum: v,
            });
            prop_assert!(result.is_ok());
        }

        // DataBlocked/StreamsBlocked は常に成功 (情報目的のみ)
        let _ = session.process_capsule(&Capsule::DataBlocked { maximum: 999 });
        let _ = session.process_capsule(&Capsule::StreamsBlocked {
            bidirectional: true,
            maximum: 999,
        });

        // 最終リミットが最大値と一致
        if let Some(&max) = sorted_data.last() {
            prop_assert_eq!(session.remote_limits().max_data, max);
        }
        if let Some(&max) = sorted_bidi.last() {
            prop_assert_eq!(session.remote_limits().max_streams_bidi, max);
        }
        if let Some(&max) = sorted_uni.last() {
            prop_assert_eq!(session.remote_limits().max_streams_uni, max);
        }
    }

    /// Property: 単調増加列の後に減少値を送ると FlowControlError (RFC Section 5.6)
    #[test]
    fn prop_session_flow_control_violation_after_increase(
        first in 100u64..10000,
        increase in 1u64..10000,
        decrease in 1u64..100,
    ) {
        let mut session = Session::new(0);
        let second = first.saturating_add(increase);

        // 単調増加
        session.process_capsule(&Capsule::MaxData { maximum: first }).unwrap();
        session.process_capsule(&Capsule::MaxData { maximum: second }).unwrap();

        // 減少値 → FlowControlError
        let smaller = second.saturating_sub(decrease);
        if smaller < second {
            let result = session.process_capsule(&Capsule::MaxData { maximum: smaller });
            prop_assert!(result.is_err());
        }
    }
}

// =============================================================================
// Connect Protocol Negotiation Properties (RFC Section 3.3)
// =============================================================================

proptest! {
    /// Property: selected_protocol が available_protocols に含まれる場合は valid、
    /// 含まれない場合は invalid (RFC Section 3.3)
    #[test]
    fn prop_connect_response_protocol_selection(
        protocols in prop::collection::vec(safe_protocol_name(), 1..5),
        selected_idx in 0usize..10,
        extra_protocol in safe_protocol_name(),
    ) {
        let req = ConnectRequest::new("https", "example.com", "/")
            .available_protocols(protocols.clone());

        if selected_idx < protocols.len() {
            // available_protocols 内のプロトコルを選択 → valid
            let resp = ConnectResponse::new(200)
                .with_protocol(&protocols[selected_idx]);
            prop_assert!(resp.is_protocol_valid(&req));
        }

        // available_protocols に含まれないプロトコルを選択 → invalid
        if !protocols.contains(&extra_protocol) {
            let resp = ConnectResponse::new(200)
                .with_protocol(&extra_protocol);
            prop_assert!(!resp.is_protocol_valid(&req));
        }
    }
}

// =============================================================================
// Settings iter Properties (RFC Section 9.2)
// =============================================================================

proptest! {
    /// Property: builder で設定した値と iter() 出力が一致。0 の値は含まれない (RFC Section 9.2)
    #[test]
    fn prop_settings_iter_matches_builder(
        max_sessions in 0u64..100,
        max_streams_uni in 0u64..1000,
        max_streams_bidi in 0u64..1000,
        max_data in 0u64..100000,
    ) {
        let settings = Settings::new()
            .wt_enabled(max_sessions)
            .wt_initial_max_streams_uni(max_streams_uni)
            .wt_initial_max_streams_bidi(max_streams_bidi)
            .wt_initial_max_data(max_data);

        let entries: Vec<(u64, u64)> = settings.iter().collect();

        // 0 の値は含まれない
        for &(_, v) in &entries {
            prop_assert!(v > 0, "iter() should not include zero values");
        }

        // 0 でない値は全て含まれる
        if max_sessions > 0 {
            prop_assert!(entries.contains(&(SettingsId::WtEnabled as u64, max_sessions)));
        }
        if max_streams_uni > 0 {
            prop_assert!(entries.contains(&(SettingsId::WtInitialMaxStreamsUni as u64, max_streams_uni)));
        }
        if max_streams_bidi > 0 {
            prop_assert!(entries.contains(&(SettingsId::WtInitialMaxStreamsBidi as u64, max_streams_bidi)));
        }
        if max_data > 0 {
            prop_assert!(entries.contains(&(SettingsId::WtInitialMaxData as u64, max_data)));
        }

        // エントリ数は 0 でないフィールド数と一致
        let expected_count = [max_sessions, max_streams_uni, max_streams_bidi, max_data]
            .iter()
            .filter(|&&v| v > 0)
            .count();
        prop_assert_eq!(entries.len(), expected_count);
    }
}

// =============================================================================
// ApplicationErrorCode Reserved Collision Properties (RFC Section 9.5)
// =============================================================================

proptest! {
    /// Property: to_http3_code() の結果が予約コードポイント (x - 0x21) % 0x1f == 0 でない
    /// (RFC Section 9.5)
    #[test]
    fn prop_application_error_code_no_reserved_collision(app_code in any::<u32>()) {
        let http3_code = ApplicationErrorCode::to_http3_code(app_code);

        // 予約コードポイントでないことを確認
        prop_assert!(
            !(http3_code.wrapping_sub(0x21)).is_multiple_of(0x1f),
            "HTTP/3 code {} is a reserved codepoint",
            http3_code
        );
    }
}

// =============================================================================
// Session Stream Add/Remove Consistency Properties
// =============================================================================

proptest! {
    /// Property: 追加/削除を交互に実行後、stream_count() と get_stream() が整合的
    #[test]
    fn prop_session_add_remove_stream_consistency(
        add_ids in prop::collection::vec(0u64..100, 1..20),
        remove_ids in prop::collection::vec(0u64..100, 0..10),
    ) {
        let mut session = Session::new(0);

        // ストリーム ID を 4 の倍数に変換 (重複排除)
        let mut added_set = std::collections::HashSet::new();
        for &raw_id in &add_ids {
            let sid = raw_id * 4;
            if added_set.insert(sid) {
                session.add_stream(Stream::new(sid, 0, true));
            }
        }

        // 一部を削除
        for &raw_id in &remove_ids {
            let sid = raw_id * 4;
            session.remove_stream(sid);
            added_set.remove(&sid);
        }

        // stream_count() が整合的
        prop_assert_eq!(session.stream_count(), added_set.len());

        // get_stream() が整合的
        for &sid in &added_set {
            prop_assert!(session.get_stream(sid).is_some(),
                "Stream {} should exist", sid);
        }

        // 削除済みのストリームは取得不可
        for &raw_id in &remove_ids {
            let sid = raw_id * 4;
            if !added_set.contains(&sid) {
                prop_assert!(session.get_stream(sid).is_none(),
                    "Stream {} should not exist", sid);
            }
        }
    }
}

// =============================================================================
// 動的ウィンドウ更新 Properties
// =============================================================================

proptest! {
    /// advertised_max は単調増加する (ストリーム)
    ///
    /// 任意の open/close シーケンスに対して、生成される WT_MAX_STREAMS の
    /// maximum は常に前回以上の値である。
    #[test]
    fn prop_advertised_max_monotonically_increases(
        concurrent_limit in 1u64..200,
        num_streams in 1usize..500,
    ) {
        let mut session = Session::new(0);
        session.set_established();
        session.initialize_local_limits(FlowControlLimits {
            max_streams_uni: concurrent_limit,
            ..FlowControlLimits::default()
        });

        let mut last_max = concurrent_limit;

        for _ in 0..num_streams {
            session.add_received_stream(false);
        }

        for _ in 0..num_streams {
            session.on_remote_stream_closed(false);

            for capsule in session.take_pending_capsules() {
                if let Capsule::MaxStreams { bidirectional: false, maximum } = capsule {
                    prop_assert!(
                        maximum >= last_max,
                        "advertised_max decreased: {} -> {}", last_max, maximum
                    );
                    last_max = maximum;
                }
            }
        }
    }

    /// advertised_max は MAX_STREAMS_LIMIT を超えない (ストリーム)
    #[test]
    fn prop_advertised_max_within_limit(
        concurrent_limit in 1u64..=MAX_STREAMS_LIMIT,
        num_cycles in 1usize..100,
    ) {
        let mut session = Session::new(0);
        session.set_established();
        session.initialize_local_limits(FlowControlLimits {
            max_streams_uni: concurrent_limit,
            ..FlowControlLimits::default()
        });

        for _ in 0..num_cycles {
            session.add_received_stream(false);
            session.on_remote_stream_closed(false);

            for capsule in session.take_pending_capsules() {
                if let Capsule::MaxStreams { maximum, .. } = capsule {
                    prop_assert!(
                        maximum <= MAX_STREAMS_LIMIT,
                        "advertised_max exceeds limit: {}", maximum
                    );
                }
            }
        }
    }

    /// WT_STREAMS_BLOCKED は同じ maximum に対して 1 回だけ送信される
    #[test]
    fn prop_streams_blocked_dedup(
        limit in 0u64..50,
        attempts in 2usize..20,
    ) {
        let mut session = Session::new(0);
        session.set_established();
        session.remote_limits_mut().max_streams_uni = limit;

        // limit 本まで開く
        for _ in 0..limit {
            prop_assert!(session.try_open_stream(false));
        }

        // これ以降はブロック
        for _ in 0..attempts {
            prop_assert!(!session.try_open_stream(false));
        }

        let capsules = session.take_pending_capsules();
        let blocked_count = capsules
            .iter()
            .filter(|c| matches!(c, Capsule::StreamsBlocked { bidirectional: false, .. }))
            .count();
        prop_assert_eq!(
            blocked_count, 1,
            "STREAMS_BLOCKED should be sent exactly once per maximum, got {}", blocked_count
        );
    }

    /// セッション A → セッション B の動的ウィンドウ更新往復プロパティ
    ///
    /// セッション A で on_remote_stream_closed により生成された WT_MAX_STREAMS カプセルを
    /// セッション B の process_capsule に渡すと remote_limits が正しく更新される。
    #[test]
    fn prop_dynamic_max_streams_roundtrip(
        concurrent_limit in 1u64..200,
        num_streams in 1usize..200,
    ) {
        // セッション A: 受信側 (WT_MAX_STREAMS を生成する)
        let mut session_a = Session::new(0);
        session_a.set_established();
        session_a.initialize_local_limits(FlowControlLimits {
            max_streams_uni: concurrent_limit,
            ..FlowControlLimits::default()
        });

        // セッション B: 送信側 (WT_MAX_STREAMS を受信して remote_limits を更新)
        let mut session_b = Session::new(0);
        session_b.set_established();
        // 初期値はセッション A の SETTINGS から
        session_b.remote_limits_mut().max_streams_uni = concurrent_limit;

        for _ in 0..num_streams {
            session_a.add_received_stream(false);
        }

        for _ in 0..num_streams {
            session_a.on_remote_stream_closed(false);

            for capsule in session_a.take_pending_capsules() {
                if capsule.capsule_type() == 0x190B4D40 {
                    // WT_MAX_STREAMS (uni)
                    let result = session_b.process_capsule(&capsule);
                    prop_assert!(result.is_ok(), "process_capsule failed: {:?}", result);
                }
            }
        }

        // セッション B の remote_limits は セッション A の advertised_max 以上
        prop_assert!(
            session_b.remote_limits().max_streams_uni >= concurrent_limit,
            "remote_limits should be >= initial: {} < {}",
            session_b.remote_limits().max_streams_uni,
            concurrent_limit
        );
    }

    /// advertised_max は単調増加する (データ)
    #[test]
    fn prop_data_advertised_max_monotonically_increases(
        initial_window in 1u64..10000,
        num_chunks in 1usize..100,
        chunk_size in 1u64..200,
    ) {
        let mut session = Session::new(0);
        session.set_established();
        session.initialize_local_limits(FlowControlLimits {
            max_data: initial_window,
            ..FlowControlLimits::default()
        });

        let mut last_max = initial_window;

        for _ in 0..num_chunks {
            let size = chunk_size.min(initial_window); // 受信上限内に収める
            if session.check_received_data(size) {
                session.add_received_data(size);
                session.on_data_consumed(size);

                for capsule in session.take_pending_capsules() {
                    if let Capsule::MaxData { maximum } = capsule {
                        prop_assert!(
                            maximum >= last_max,
                            "data advertised_max decreased: {} -> {}", last_max, maximum
                        );
                        last_max = maximum;
                    }
                }
            }
        }
    }

    /// WT_DATA_BLOCKED は同じ maximum に対して 1 回だけ送信される
    #[test]
    fn prop_data_blocked_dedup(
        limit in 0u64..100,
        attempts in 2usize..20,
    ) {
        let mut session = Session::new(0);
        session.set_established();
        session.remote_limits_mut().max_data = limit;

        // limit まで送信
        if limit > 0 {
            prop_assert!(session.try_send_data(limit));
        }

        // これ以降はブロック
        for _ in 0..attempts {
            prop_assert!(!session.try_send_data(1));
        }

        let capsules = session.take_pending_capsules();
        let blocked_count = capsules
            .iter()
            .filter(|c| matches!(c, Capsule::DataBlocked { .. }))
            .count();
        prop_assert_eq!(
            blocked_count, 1,
            "DATA_BLOCKED should be sent exactly once per maximum, got {}", blocked_count
        );
    }
}
