//! Property-Based Testing for WebTransport Capsule Protocol
//! (draft-ietf-webtrans-http3-15 Section 5.6, 6)

use proptest::prelude::*;
use shiguredo_http3::webtransport::{Capsule, CapsuleValidationError, MAX_STREAMS_LIMIT};

// =============================================================================
// Strategy ヘルパー
// =============================================================================

/// 有効な可変長整数の最大値
const MAX_VARINT: u64 = (1 << 62) - 1;

prop_compose! {
    /// CloseSession Capsule を生成 (メッセージは最大 1024 バイトの ASCII)
    fn close_session_capsule()(
        error_code in any::<u32>(),
        msg_len in 0usize..=1024,
    )(
        error_code in Just(error_code),
        msg in prop::collection::vec(0x20u8..0x7f, msg_len),
    ) -> Capsule {
        let message = String::from_utf8(msg).unwrap_or_default();
        Capsule::CloseSession { error_code, message }
    }
}

prop_compose! {
    /// MaxData Capsule を生成
    fn max_data_capsule()(maximum in 0u64..=MAX_VARINT) -> Capsule {
        Capsule::MaxData { maximum }
    }
}

prop_compose! {
    /// MaxStreams Capsule を生成
    fn max_streams_capsule()(
        bidirectional in any::<bool>(),
        maximum in 0u64..=MAX_VARINT,
    ) -> Capsule {
        Capsule::MaxStreams { bidirectional, maximum }
    }
}

prop_compose! {
    /// DataBlocked Capsule を生成
    fn data_blocked_capsule()(maximum in 0u64..=MAX_VARINT) -> Capsule {
        Capsule::DataBlocked { maximum }
    }
}

prop_compose! {
    /// StreamsBlocked Capsule を生成
    fn streams_blocked_capsule()(
        bidirectional in any::<bool>(),
        maximum in 0u64..=MAX_VARINT,
    ) -> Capsule {
        Capsule::StreamsBlocked { bidirectional, maximum }
    }
}

/// 全ての既知の Capsule 型を生成する Strategy
fn any_known_capsule() -> impl Strategy<Value = Capsule> {
    prop_oneof![
        close_session_capsule(),
        Just(Capsule::DrainSession),
        max_data_capsule(),
        max_streams_capsule(),
        data_blocked_capsule(),
        streams_blocked_capsule(),
    ]
}

// =============================================================================
// (a) Capsule encode/decode ラウンドトリップ
// =============================================================================

proptest! {
    /// Property: 任意の既知の Capsule を encode → decode すると元と一致する
    #[test]
    fn prop_capsule_roundtrip(capsule in any_known_capsule()) {
        let mut buf = Vec::new();
        capsule.encode(&mut buf);

        let (decoded, consumed) = Capsule::decode(&buf)
            .expect("decode should not error for valid encoded capsule")
            .expect("decode should not be incomplete for valid encoded capsule");

        prop_assert_eq!(
            &decoded, &capsule,
            "ラウンドトリップで Capsule が変化した"
        );
        prop_assert_eq!(
            consumed, buf.len(),
            "消費バイト数がバッファ長と不一致"
        );
    }
}

// =============================================================================
// (b) encode_as_data_frame ラウンドトリップ
// =============================================================================

proptest! {
    /// Property: encode_as_data_frame の出力を decode_frame で DATA として取り出し、
    /// そのペイロードを Capsule::decode すると元と一致する
    #[test]
    fn prop_capsule_data_frame_roundtrip(capsule in any_known_capsule()) {
        let mut buf = Vec::new();
        capsule.encode_as_data_frame(&mut buf);

        // DATA フレームとしてデコード
        let (frame, consumed) = shiguredo_http3::frame::decode_frame(&buf)
            .expect("decode_frame should succeed");
        prop_assert_eq!(consumed, buf.len(), "consumed != buf.len()");

        // DATA フレームからペイロードを取り出す
        match frame {
            shiguredo_http3::Frame::Data(payload) => {
                let (decoded, cap_consumed) = Capsule::decode(payload.data())
                    .expect("Capsule::decode should not error")
                    .expect("Capsule::decode should not be incomplete");
                prop_assert_eq!(
                    &decoded, &capsule,
                    "カプセルがラウンドトリップで変化した"
                );
                prop_assert_eq!(
                    cap_consumed, payload.len(),
                    "カプセル消費バイト数が不一致"
                );
            }
            other => {
                prop_assert!(false, "DATA フレームでない: {:?}", other);
            }
        }
    }
}

// =============================================================================
// (c) エンコードバイト列の消費量が encode の出力長と一致
// =============================================================================

proptest! {
    /// Property: decode の consumed がバッファ長と一致する
    #[test]
    fn prop_capsule_consumed_equals_buffer_length(capsule in any_known_capsule()) {
        let mut buf = Vec::new();
        capsule.encode(&mut buf);

        let (_, consumed) = Capsule::decode(&buf)
            .expect("decode should not error")
            .expect("decode should not be incomplete");

        prop_assert_eq!(
            consumed,
            buf.len(),
            "consumed != buf.len()"
        );
    }
}

// =============================================================================
// (c) validate_max_streams の単調増加制約
// =============================================================================

proptest! {
    /// Property: maximum > current_max かつ maximum <= MAX_STREAMS_LIMIT なら Ok
    #[test]
    fn prop_validate_max_streams_ok(
        current_max in 0u64..=MAX_STREAMS_LIMIT,
        delta in 1u64..1000,
    ) {
        let maximum = current_max.saturating_add(delta).min(MAX_STREAMS_LIMIT);
        prop_assume!(maximum > current_max);
        let result = Capsule::validate_max_streams(maximum, current_max);
        prop_assert!(
            result.is_ok(),
            "maximum > current_max かつ上限以下なのに Err: maximum={}, current_max={}", maximum, current_max
        );
    }

    /// Property: maximum <= current_max なら MaxStreamsDecreased
    /// (draft-16: "does not increase")
    #[test]
    fn prop_validate_max_streams_not_increased(
        current_max in 1u64..=MAX_STREAMS_LIMIT,
        delta in 0u64..1000,
    ) {
        let maximum = current_max.saturating_sub(delta);
        prop_assume!(maximum <= current_max);

        let result = Capsule::validate_max_streams(maximum, current_max);
        prop_assert_eq!(
            result,
            Err(CapsuleValidationError::MaxStreamsDecreased),
            "maximum <= current_max なのに MaxStreamsDecreased でない: maximum={}, current_max={}", maximum, current_max
        );
    }

    /// Property: maximum > MAX_STREAMS_LIMIT なら MaxStreamsExceedsLimit
    #[test]
    fn prop_validate_max_streams_exceeds_limit(
        excess in 1u64..1000,
    ) {
        let maximum = MAX_STREAMS_LIMIT + excess;
        let result = Capsule::validate_max_streams(maximum, 0);
        prop_assert_eq!(
            result,
            Err(CapsuleValidationError::MaxStreamsExceedsLimit),
            "maximum > MAX_STREAMS_LIMIT なのに MaxStreamsExceedsLimit でない: maximum={}", maximum
        );
    }
}

// =============================================================================
// (d) validate_max_data の単調増加制約
// =============================================================================

proptest! {
    /// Property: maximum > current_max なら Ok
    #[test]
    fn prop_validate_max_data_ok(
        current_max in 0u64..=MAX_VARINT / 2,
        delta in 1u64..1000,
    ) {
        let maximum = current_max.saturating_add(delta);
        let result = Capsule::validate_max_data(maximum, current_max);
        prop_assert!(
            result.is_ok(),
            "maximum > current_max なのに Err: maximum={}, current_max={}", maximum, current_max
        );
    }

    /// Property: maximum <= current_max なら MaxDataDecreased
    /// (draft-16: "does not increase")
    #[test]
    fn prop_validate_max_data_not_increased(
        current_max in 1u64..=MAX_VARINT,
        delta in 0u64..1000,
    ) {
        let maximum = current_max.saturating_sub(delta);
        prop_assume!(maximum <= current_max);

        let result = Capsule::validate_max_data(maximum, current_max);
        prop_assert_eq!(
            result,
            Err(CapsuleValidationError::MaxDataDecreased),
            "maximum <= current_max なのに MaxDataDecreased でない: maximum={}, current_max={}", maximum, current_max
        );
    }

    /// Property: maximum > MAX_VARINT なら MaxDataExceedsLimit
    #[test]
    fn prop_validate_max_data_exceeds_limit(
        excess in 1u64..1000,
    ) {
        let maximum = MAX_VARINT + excess;
        let result = Capsule::validate_max_data(maximum, 0);
        prop_assert_eq!(
            result,
            Err(CapsuleValidationError::MaxDataExceedsLimit),
            "maximum > MAX_VARINT なのに MaxDataExceedsLimit でない: maximum={}", maximum
        );
    }
}
