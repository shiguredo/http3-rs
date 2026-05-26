//! Property-Based Testing for QPACK 整数コーデック (RFC 7541 Section 5.1)

use proptest::prelude::*;
use shiguredo_http3::qpack::integer;

// RFC 9204 Section 4.1.1: 62 ビットまでデコード可能 (MUST)
const MAX_DECODABLE_VALUE: u64 = (1u64 << 62) - 1;

proptest! {
    /// Property: encode -> decode のラウンドトリップで値が一致する (スライス版)
    #[test]
    fn prop_integer_roundtrip_slice(
        prefix_bits in 1u8..=8,
        value in 0u64..=MAX_DECODABLE_VALUE,
        raw_prefix in any::<u8>(),
    ) {
        let prefix_mask = 0xFFu8.checked_shl(prefix_bits as u32).unwrap_or(0);
        let prefix = raw_prefix & prefix_mask;

        let mut buf = vec![0u8; 16];
        let Some(encoded_len) = integer::encode_integer(&mut buf, value, prefix_bits, prefix) else {
            return Err(TestCaseError::reject("バッファが小さすぎる"));
        };

        let (decoded_value, decoded_len) = integer::decode_integer(&buf[..encoded_len], prefix_bits)
            .map_err(|e| TestCaseError::fail(format!("デコード失敗: {:?}", e)))?;

        prop_assert_eq!(decoded_value, value, "値が一致しない");
        prop_assert_eq!(decoded_len, encoded_len, "長さが一致しない");

        // prefix ビット外のビットが保存されていること
        let first_byte_prefix = buf[0] & prefix_mask;
        prop_assert_eq!(first_byte_prefix, prefix & prefix_mask, "prefix ビットが壊れている");
    }

    /// Property: encode -> decode のラウンドトリップで値が一致する (Vec 版)
    #[test]
    fn prop_integer_roundtrip_vec(
        prefix_bits in 1u8..=8,
        value in 0u64..=MAX_DECODABLE_VALUE,
        raw_prefix in any::<u8>(),
    ) {
        let prefix_mask = 0xFFu8.checked_shl(prefix_bits as u32).unwrap_or(0);
        let prefix = raw_prefix & prefix_mask;

        let mut buf = Vec::new();
        integer::encode_integer_to_vec(&mut buf, value, prefix_bits, prefix);

        let (decoded_value, decoded_len) = integer::decode_integer(&buf, prefix_bits)
            .map_err(|e| TestCaseError::fail(format!("デコード失敗: {:?}", e)))?;

        prop_assert_eq!(decoded_value, value, "値が一致しない");
        prop_assert_eq!(decoded_len, buf.len(), "長さが一致しない");
    }

    /// Property: スライス版と Vec 版のエンコード結果が一致する
    #[test]
    fn prop_slice_and_vec_encode_match(
        prefix_bits in 1u8..=8,
        value in 0u64..=MAX_DECODABLE_VALUE,
        raw_prefix in any::<u8>(),
    ) {
        let prefix_mask = 0xFFu8.checked_shl(prefix_bits as u32).unwrap_or(0);
        let prefix = raw_prefix & prefix_mask;

        let mut slice_buf = vec![0u8; 16];
        let Some(encoded_len) = integer::encode_integer(&mut slice_buf, value, prefix_bits, prefix) else {
            return Err(TestCaseError::reject("バッファが小さすぎる"));
        };

        let mut vec_buf = Vec::new();
        integer::encode_integer_to_vec(&mut vec_buf, value, prefix_bits, prefix);

        prop_assert_eq!(&slice_buf[..encoded_len], &vec_buf[..], "エンコード結果が一致しない");
    }

    /// Property: prefix_bits の上限値 (2^N - 1) 未満の値は 1 バイトでエンコードされる
    #[test]
    fn prop_small_value_single_byte(
        prefix_bits in 1u8..=8,
    ) {
        let max_prefix = (1u64 << prefix_bits) - 1;
        let value = max_prefix - 1;
        let mut buf = vec![0u8; 16];
        let encoded_len = integer::encode_integer(&mut buf, value, prefix_bits, 0x00)
            .expect("エンコードは成功するはず");

        prop_assert_eq!(encoded_len, 1, "max_prefix 未満の値は 1 バイトでエンコードされるべき");
    }
}

// =============================================================================
// 境界値テスト (PBT で到達しにくいケース)
// =============================================================================

#[test]
fn test_decode_empty_buffer() {
    let result = integer::decode_integer(&[], 5);
    assert!(result.is_err(), "空バッファではエラーを返すこと");
}

#[test]
fn test_encode_zero() {
    let mut buf = vec![0u8; 16];
    let len = integer::encode_integer(&mut buf, 0, 5, 0x00).expect("0 のエンコードは成功するはず");
    assert_eq!(len, 1);
    assert_eq!(buf[0], 0x00);

    let (val, dec_len) = integer::decode_integer(&buf[..len], 5).expect("デコードは成功するはず");
    assert_eq!(val, 0);
    assert_eq!(dec_len, 1);
}

#[test]
fn test_encode_max_prefix_boundary() {
    for prefix_bits in 1u8..=8 {
        let max_prefix = (1u64 << prefix_bits) - 1;

        let mut buf = vec![0u8; 16];
        let len = integer::encode_integer(&mut buf, max_prefix, prefix_bits, 0x00)
            .expect("max_prefix のエンコードは成功するはず");

        // max_prefix 以上は複数バイト
        assert!(
            len >= 2,
            "prefix_bits={}: max_prefix ({}) は 2 バイト以上",
            prefix_bits,
            max_prefix
        );

        let (val, dec_len) =
            integer::decode_integer(&buf[..len], prefix_bits).expect("デコードは成功するはず");
        assert_eq!(val, max_prefix);
        assert_eq!(dec_len, len);
    }
}

#[test]
fn test_encode_slice_buffer_too_small() {
    let mut buf = [0u8; 0];
    assert!(
        integer::encode_integer(&mut buf, 0, 5, 0x00).is_none(),
        "空バッファで None を返すこと"
    );

    let mut buf = [0u8; 1];
    assert!(
        integer::encode_integer(&mut buf, 31, 5, 0x00).is_none(),
        "1 バイトバッファで多バイト値は None を返すこと"
    );
}

#[test]
fn test_decode_overflow_protection() {
    // shift > 56 でオーバーフロー保護が発動する入力を構築する
    // prefix_bits=1, prefix value = 1 (= max_prefix for 1-bit prefix), then 9 continuation bytes
    let mut data = vec![0x01];
    data.extend(std::iter::repeat_n(0x80, 9));
    data.push(0x01);

    let result = integer::decode_integer(&data, 1);
    assert!(
        result.is_err(),
        "shift > 56 でオーバーフロー保護が発動すること"
    );
}

#[test]
fn test_decode_truncated_multi_byte() {
    // 多バイトエンコードの途中でデータが途切れるケース
    let data = [0x1f, 0x80];
    let result = integer::decode_integer(&data, 5);
    assert!(
        result.is_err(),
        "不完全な多バイトエンコードではエラーを返すこと"
    );
}

#[test]
fn test_max_decodable_value_roundtrip() {
    // RFC 9204 Section 4.1.1: 62 ビットまでデコード可能
    let max_value = (1u64 << 62) - 1;
    let mut buf = vec![0u8; 16];
    let len = integer::encode_integer(&mut buf, max_value, 8, 0x00)
        .expect("最大デコード可能値のエンコードは成功するはず");

    let (val, dec_len) = integer::decode_integer(&buf[..len], 8)
        .expect("最大デコード可能値のデコードは成功するはず");
    assert_eq!(val, max_value);
    assert_eq!(dec_len, len);
}
