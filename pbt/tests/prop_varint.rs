//! Property-Based Testing for QUIC Variable-Length Integer (RFC 9000 Section 16)

use proptest::prelude::*;
use shiguredo_http3::varint;

/// 可変長整数の最大値
const MAX_VARINT: u64 = (1 << 62) - 1;

prop_compose! {
    /// 有効な可変長整数値を生成
    fn valid_varint()(value in 0u64..=MAX_VARINT) -> u64 {
        value
    }
}

prop_compose! {
    /// 1 バイトエンコード範囲の値を生成 (0-63)
    fn one_byte_value()(value in 0u64..64) -> u64 {
        value
    }
}

prop_compose! {
    /// 2 バイトエンコード範囲の値を生成 (64-16383)
    fn two_byte_value()(value in 64u64..16384) -> u64 {
        value
    }
}

prop_compose! {
    /// 4 バイトエンコード範囲の値を生成 (16384-1073741823)
    fn four_byte_value()(value in 16384u64..1073741824) -> u64 {
        value
    }
}

prop_compose! {
    /// 8 バイトエンコード範囲の値を生成 (1073741824-MAX)
    fn eight_byte_value()(value in 1073741824u64..=MAX_VARINT) -> u64 {
        value
    }
}

proptest! {
    /// Property: エンコード -> デコードのラウンドトリップで値が保存される
    #[test]
    fn prop_roundtrip_preserves_value(value in valid_varint()) {
        let mut buf = [0u8; 8];
        let encoded_len = varint::encode(&mut buf, value).unwrap();
        let (decoded, decoded_len) = varint::decode(&buf).unwrap();

        prop_assert_eq!(value, decoded, "Roundtrip failed for value {}", value);
        prop_assert_eq!(encoded_len, decoded_len, "Length mismatch for value {}", value);
    }

    /// Property: encoded_len() が実際のエンコード長と一致する
    #[test]
    fn prop_encoded_len_matches_actual(value in valid_varint()) {
        let expected_len = varint::encoded_len(value);
        let mut buf = [0u8; 8];
        let actual_len = varint::encode(&mut buf, value).unwrap();

        prop_assert_eq!(expected_len, actual_len, "encoded_len mismatch for {}", value);
    }

    /// Property: 1 バイト値は 1 バイトでエンコードされる
    #[test]
    fn prop_one_byte_encoding(value in one_byte_value()) {
        let len = varint::encoded_len(value);
        prop_assert_eq!(len, 1, "Value {} should encode to 1 byte", value);

        let mut buf = [0u8; 8];
        varint::encode(&mut buf, value).unwrap();
        // 上位 2 ビットは 00
        prop_assert_eq!(buf[0] >> 6, 0, "1-byte prefix should be 00");
    }

    /// Property: 2 バイト値は 2 バイトでエンコードされる
    #[test]
    fn prop_two_byte_encoding(value in two_byte_value()) {
        let len = varint::encoded_len(value);
        prop_assert_eq!(len, 2, "Value {} should encode to 2 bytes", value);

        let mut buf = [0u8; 8];
        varint::encode(&mut buf, value).unwrap();
        // 上位 2 ビットは 01
        prop_assert_eq!(buf[0] >> 6, 1, "2-byte prefix should be 01");
    }

    /// Property: 4 バイト値は 4 バイトでエンコードされる
    #[test]
    fn prop_four_byte_encoding(value in four_byte_value()) {
        let len = varint::encoded_len(value);
        prop_assert_eq!(len, 4, "Value {} should encode to 4 bytes", value);

        let mut buf = [0u8; 8];
        varint::encode(&mut buf, value).unwrap();
        // 上位 2 ビットは 10
        prop_assert_eq!(buf[0] >> 6, 2, "4-byte prefix should be 10");
    }

    /// Property: 8 バイト値は 8 バイトでエンコードされる
    #[test]
    fn prop_eight_byte_encoding(value in eight_byte_value()) {
        let len = varint::encoded_len(value);
        prop_assert_eq!(len, 8, "Value {} should encode to 8 bytes", value);

        let mut buf = [0u8; 8];
        varint::encode(&mut buf, value).unwrap();
        // 上位 2 ビットは 11
        prop_assert_eq!(buf[0] >> 6, 3, "8-byte prefix should be 11");
    }

    /// Property: peek_len() がデコード前に正しい長さを返す
    #[test]
    fn prop_peek_len_matches_decode_len(value in valid_varint()) {
        let mut buf = [0u8; 8];
        let encoded_len = varint::encode(&mut buf, value).unwrap();
        let peeked_len = varint::peek_len(&buf).unwrap();

        prop_assert_eq!(encoded_len, peeked_len, "peek_len mismatch for {}", value);
    }

    /// Property: バッファサイズが不足している場合はエラー
    #[test]
    fn prop_insufficient_buffer_returns_error(value in two_byte_value()) {
        let mut buf = [0u8; 1]; // 2 バイト値には不十分
        let result = varint::encode(&mut buf, value);
        prop_assert!(result.is_err(), "Should fail for insufficient buffer");
    }

    /// Property: エンコード結果はビッグエンディアン順
    #[test]
    fn prop_encoding_is_big_endian(value in two_byte_value()) {
        let mut buf = [0u8; 8];
        varint::encode(&mut buf, value).unwrap();

        // 2 バイトエンコードの場合、値は 0x4000 | value として格納
        let expected_high = (0x40 | ((value >> 8) & 0x3f)) as u8;
        let expected_low = (value & 0xff) as u8;

        prop_assert_eq!(buf[0], expected_high, "High byte mismatch");
        prop_assert_eq!(buf[1], expected_low, "Low byte mismatch");
    }
}

/// Property: MAX_VALUE を超える値はエンコードできない
#[test]
fn prop_value_exceeding_max_fails() {
    let mut buf = [0u8; 8];
    let result = varint::encode(&mut buf, MAX_VARINT + 1);
    assert!(result.is_err());
}

/// Property: 空バッファのデコードはエラー
#[test]
fn prop_empty_buffer_decode_fails() {
    let result = varint::decode(&[]);
    assert!(result.is_err());
}
