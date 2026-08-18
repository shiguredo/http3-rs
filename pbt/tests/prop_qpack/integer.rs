//! Property-Based Testing for QPACK 整数コーデック (RFC 7541 Section 5.1)
//!
//! 境界値テストは tests/test_qpack_integer.rs を参照。

use pbt::strategies::sample_varint_raw_in;
use shiguredo_http3::qpack::integer;

// RFC 9204 Section 4.1.1: 62 ビットまでデコード可能 (MUST)
const MAX_DECODABLE_VALUE: u64 = (1u64 << 62) - 1;

/// prefix_bits (1..=8) を生成する
fn sample_prefix_bits(ctx: &mut noprop::TestCaseContext) -> u8 {
    noprop::sample_usize_in(ctx, 1..=8) as u8
}

/// prefix 長に応じた 1 バイト境界と VarInt 符号化境界を厚くした整数値
fn sample_integer_value(ctx: &mut noprop::TestCaseContext, prefix_bits: u8) -> u64 {
    let max_prefix = (1u64 << prefix_bits) - 1;
    let mut boundaries = vec![0, 1, MAX_DECODABLE_VALUE];
    if max_prefix > 1 {
        boundaries.push(max_prefix - 1);
    }
    boundaries.push(max_prefix);
    boundaries.sort_unstable();
    boundaries.dedup();
    noprop::sample_with_boundaries(ctx, &boundaries, noprop::Ratio::one_nth(5), |ctx| {
        sample_varint_raw_in(ctx, 0..=MAX_DECODABLE_VALUE)
    })
}

/// Property: encode -> decode のラウンドトリップで値が一致する (スライス版)
#[test]
fn prop_integer_roundtrip_slice() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_QPACK_INTEGER_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let prefix_bits = sample_prefix_bits(ctx);
        let value = sample_integer_value(ctx, prefix_bits);
        let raw_prefix = noprop::sample_u8(ctx);

        let prefix_mask = 0xFFu8.checked_shl(prefix_bits as u32).unwrap_or(0);
        let prefix = raw_prefix & prefix_mask;

        let mut buf = vec![0u8; 16];
        // 16 バイトバッファは 62 ビット値の最大エンコード長 (9 バイト) に対して十分
        let encoded_len = integer::encode_integer(&mut buf, value, prefix_bits, prefix)
            .expect("16 バイトバッファはエンコードに十分");

        let (decoded_value, decoded_len) =
            integer::decode_integer(&buf[..encoded_len], prefix_bits)
                .map_err(|e| format!("デコード失敗: {:?}", e))?;

        assert_eq!(decoded_value, value, "値が一致しない");
        assert_eq!(decoded_len, encoded_len, "長さが一致しない");

        // prefix ビット外のビットが保存されていること
        let first_byte_prefix = buf[0] & prefix_mask;
        assert_eq!(
            first_byte_prefix,
            prefix & prefix_mask,
            "prefix ビットが壊れている"
        );
        Ok(())
    })?;
    Ok(())
}

/// Property: encode -> decode のラウンドトリップで値が一致する (Vec 版)
#[test]
fn prop_integer_roundtrip_vec() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_QPACK_INTEGER_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let prefix_bits = sample_prefix_bits(ctx);
        let value = sample_integer_value(ctx, prefix_bits);
        let raw_prefix = noprop::sample_u8(ctx);

        let prefix_mask = 0xFFu8.checked_shl(prefix_bits as u32).unwrap_or(0);
        let prefix = raw_prefix & prefix_mask;

        let mut buf = Vec::new();
        integer::encode_integer_to_vec(&mut buf, value, prefix_bits, prefix);

        let (decoded_value, decoded_len) = integer::decode_integer(&buf, prefix_bits)
            .map_err(|e| format!("デコード失敗: {:?}", e))?;

        assert_eq!(decoded_value, value, "値が一致しない");
        assert_eq!(decoded_len, buf.len(), "長さが一致しない");
        Ok(())
    })?;
    Ok(())
}

/// Property: スライス版と Vec 版のエンコード結果が一致する
#[test]
fn prop_slice_and_vec_encode_match() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_QPACK_INTEGER_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let prefix_bits = sample_prefix_bits(ctx);
        let value = sample_integer_value(ctx, prefix_bits);
        let raw_prefix = noprop::sample_u8(ctx);

        let prefix_mask = 0xFFu8.checked_shl(prefix_bits as u32).unwrap_or(0);
        let prefix = raw_prefix & prefix_mask;

        let mut slice_buf = vec![0u8; 16];
        let encoded_len = integer::encode_integer(&mut slice_buf, value, prefix_bits, prefix)
            .expect("16 バイトバッファはエンコードに十分");

        let mut vec_buf = Vec::new();
        integer::encode_integer_to_vec(&mut vec_buf, value, prefix_bits, prefix);

        assert_eq!(
            &slice_buf[..encoded_len],
            &vec_buf[..],
            "エンコード結果が一致しない"
        );
        Ok(())
    })?;
    Ok(())
}

/// Property: prefix_bits の上限値 (2^N - 1) 未満の値は 1 バイトでエンコードされる
#[test]
fn prop_small_value_single_byte() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_QPACK_INTEGER_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let prefix_bits = sample_prefix_bits(ctx);
        let max_prefix = (1u64 << prefix_bits) - 1;
        let value = max_prefix - 1;
        let mut buf = vec![0u8; 16];
        let encoded_len = integer::encode_integer(&mut buf, value, prefix_bits, 0x00)
            .expect("エンコードは成功するはず");

        assert_eq!(
            encoded_len, 1,
            "max_prefix 未満の値は 1 バイトでエンコードされるべき"
        );
        Ok(())
    })?;
    Ok(())
}
