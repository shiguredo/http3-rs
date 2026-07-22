//! QPACK 整数コーデックの境界値テスト (RFC 7541 Section 5.1)
//!
//! PBT で到達しにくいケースを単体テストでカバーする。
//! PBT (ラウンドトリップ等) は pbt/tests/prop_qpack/integer.rs を参照。

use shiguredo_http3::qpack::integer;

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
