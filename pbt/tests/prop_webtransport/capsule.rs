//! Capsule の capsule_type 検証・Unknown ラウンドトリップ・不完全バッファの Sans I/O 挙動
//! (既知バリアントのラウンドトリップは pbt/tests/prop_capsule.rs に集約)

use proptest::prelude::*;
use shiguredo_http3::webtransport::Capsule;

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
        String::from_utf8(msg).expect("ASCII range bytes are always valid UTF-8")
    }
}

prop_compose! {
    /// 有効な最大値 (可変長整数範囲)
    fn valid_maximum()(max in 0u64..=(1 << 62) - 1) -> u64 {
        max
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
        Capsule::Unknown { capsule_type, payload }
    }
}

proptest! {
    /// Property: Unknown Capsule のラウンドトリップ
    #[test]
    fn prop_unknown_capsule_roundtrip(capsule in valid_unknown_capsule()) {
        let mut buf = Vec::new();
        capsule.encode(&mut buf);

        let (decoded, consumed) = Capsule::decode(&buf)
            .expect("decode should not error")
            .expect("decode should not be incomplete");
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
// 不完全バッファの Sans I/O 挙動
// =============================================================================

proptest! {
    /// Property: 不完全なバッファでは None を返す (CloseSession)
    #[test]
    fn prop_capsule_incomplete_buffer_close_session(
        error_code in any::<u32>(),
        message in valid_error_message(),
        cut_ratio in 0.1f64..0.9,
    ) {
        let capsule = Capsule::CloseSession {
            error_code,
            message,
        };

        let mut buf = Vec::new();
        capsule.encode(&mut buf);

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
