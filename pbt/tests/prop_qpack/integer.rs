//! Property-Based Testing for QPACK 整数コーデック (RFC 7541 Section 5.1)
//!
//! 境界値テストは tests/test_qpack_integer.rs を参照。

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
