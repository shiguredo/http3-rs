//! Property-Based Testing for QUIC Variable-Length Integer (RFC 9000 Section 16)

use proptest::prelude::*;
use shiguredo_http3::VarInt;
use shiguredo_http3::varint;

prop_compose! {
    /// 有効な VarInt を生成
    fn valid_varint()(value in 0u64..=VarInt::MAX.get()) -> VarInt {
        VarInt::new(value).unwrap()
    }
}

prop_compose! {
    /// 1 バイトエンコード範囲の値を生成 (0-63)
    fn one_byte_value()(value in 0u64..64) -> VarInt {
        VarInt::new(value).unwrap()
    }
}

prop_compose! {
    /// 2 バイトエンコード範囲の値を生成 (64-16383)
    fn two_byte_value()(value in 64u64..16384) -> VarInt {
        VarInt::new(value).unwrap()
    }
}

prop_compose! {
    /// 4 バイトエンコード範囲の値を生成 (16384-1073741823)
    fn four_byte_value()(value in 16384u64..1073741824) -> VarInt {
        VarInt::new(value).unwrap()
    }
}

prop_compose! {
    /// 8 バイトエンコード範囲の値を生成 (1073741824-MAX)
    fn eight_byte_value()(value in 1073741824u64..=VarInt::MAX.get()) -> VarInt {
        VarInt::new(value).unwrap()
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

    /// Property: VarInt::encoded_len() が実際のエンコード長と一致する
    #[test]
    fn prop_encoded_len_matches_actual(value in valid_varint()) {
        let expected_len = value.encoded_len();
        let mut buf = [0u8; 8];
        let actual_len = varint::encode(&mut buf, value).unwrap();

        prop_assert_eq!(expected_len, actual_len, "encoded_len mismatch for {}", value);
    }

    /// Property: 任意 VarInt は `encoded_len()` 分のバッファで必ずエンコードできる
    /// (`EncodeError::ValueTooLarge` を削除した正当性: 値域は型レベルで保証される)
    #[test]
    fn prop_encode_succeeds_for_any_varint(value in valid_varint()) {
        let mut buf = vec![0u8; value.encoded_len()];
        let result = varint::encode(&mut buf, value);
        prop_assert!(result.is_ok(), "encode should succeed for any VarInt");
    }

    /// Property: バッファ長が `encoded_len()` 未満なら必ず `BufferTooShort` を返す
    #[test]
    fn prop_short_buffer_returns_buffer_too_short(
        value in valid_varint(),
        shortfall in 1usize..=8,
    ) {
        let need = value.encoded_len();
        // shortfall 分だけ短いバッファ (need == 1 の場合は 0 バイトバッファ)
        let len = need.saturating_sub(shortfall);
        let mut buf = vec![0u8; len];
        let result = varint::encode(&mut buf, value);
        prop_assert_eq!(result, Err(varint::EncodeError::BufferTooShort));
    }

    /// Property: 1 バイト値は 1 バイトでエンコードされる
    #[test]
    fn prop_one_byte_encoding(value in one_byte_value()) {
        let len = value.encoded_len();
        prop_assert_eq!(len, 1, "Value {} should encode to 1 byte", value);

        let mut buf = [0u8; 8];
        varint::encode(&mut buf, value).unwrap();
        // 上位 2 ビットは 00
        prop_assert_eq!(buf[0] >> 6, 0, "1-byte prefix should be 00");
    }

    /// Property: 2 バイト値は 2 バイトでエンコードされる
    #[test]
    fn prop_two_byte_encoding(value in two_byte_value()) {
        let len = value.encoded_len();
        prop_assert_eq!(len, 2, "Value {} should encode to 2 bytes", value);

        let mut buf = [0u8; 8];
        varint::encode(&mut buf, value).unwrap();
        // 上位 2 ビットは 01
        prop_assert_eq!(buf[0] >> 6, 1, "2-byte prefix should be 01");
    }

    /// Property: 4 バイト値は 4 バイトでエンコードされる
    #[test]
    fn prop_four_byte_encoding(value in four_byte_value()) {
        let len = value.encoded_len();
        prop_assert_eq!(len, 4, "Value {} should encode to 4 bytes", value);

        let mut buf = [0u8; 8];
        varint::encode(&mut buf, value).unwrap();
        // 上位 2 ビットは 10
        prop_assert_eq!(buf[0] >> 6, 2, "4-byte prefix should be 10");
    }

    /// Property: 8 バイト値は 8 バイトでエンコードされる
    #[test]
    fn prop_eight_byte_encoding(value in eight_byte_value()) {
        let len = value.encoded_len();
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

    /// Property: エンコード結果はビッグエンディアン順
    #[test]
    fn prop_encoding_is_big_endian(value in two_byte_value()) {
        let mut buf = [0u8; 8];
        varint::encode(&mut buf, value).unwrap();

        // 2 バイトエンコードの場合、値は 0x4000 | value として格納
        let raw = value.get();
        let expected_high = (0x40 | ((raw >> 8) & 0x3f)) as u8;
        let expected_low = (raw & 0xff) as u8;

        prop_assert_eq!(buf[0], expected_high, "High byte mismatch");
        prop_assert_eq!(buf[1], expected_low, "Low byte mismatch");
    }

    /// Property: `From<u8>` は任意 u8 で値が一致する
    #[test]
    fn prop_from_u8_roundtrip(value in any::<u8>()) {
        let v = VarInt::from(value);
        prop_assert_eq!(v.get(), u64::from(value));
    }

    /// Property: `From<u16>` は任意 u16 で値が一致する
    #[test]
    fn prop_from_u16_roundtrip(value in any::<u16>()) {
        let v = VarInt::from(value);
        prop_assert_eq!(v.get(), u64::from(value));
    }

    /// Property: `From<u32>` は任意 u32 で値が一致する
    #[test]
    fn prop_from_u32_roundtrip(value in any::<u32>()) {
        let v = VarInt::from(value);
        prop_assert_eq!(v.get(), u64::from(value));
    }

    /// Property: `TryFrom<u64>` は値域内で必ず Ok、値域外で必ず Err
    #[test]
    fn prop_try_from_u64_value_domain(value in any::<u64>()) {
        let result = VarInt::try_from(value);
        if value <= VarInt::MAX.get() {
            prop_assert!(result.is_ok());
            prop_assert_eq!(result.unwrap().get(), value);
        } else {
            prop_assert!(result.is_err());
        }
    }

    /// Property: `VarInt::Display` 出力は内部値の 10 進表現と一致する
    #[test]
    fn prop_display_matches_u64(value in valid_varint()) {
        prop_assert_eq!(format!("{value}"), format!("{}", value.get()));
    }
}
