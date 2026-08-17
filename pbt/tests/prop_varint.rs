//! Property-Based Testing for QUIC Variable-Length Integer (RFC 9000 Section 16)

use pbt::strategies::{invalid_varint_u64, valid_varint};
use shiguredo_http3::VarInt;
use shiguredo_http3::varint;

/// 1 バイトエンコード範囲の値を生成 (0-63)
fn one_byte_value(ctx: &mut noprop::TestCaseContext) -> VarInt {
    VarInt::new(noprop::sample_u64_in(ctx, 0..64)).expect("test must succeed")
}

/// 2 バイトエンコード範囲の値を生成 (64-16383)
fn two_byte_value(ctx: &mut noprop::TestCaseContext) -> VarInt {
    VarInt::new(noprop::sample_u64_in(ctx, 64..16384)).expect("test must succeed")
}

/// 4 バイトエンコード範囲の値を生成 (16384-1073741823)
fn four_byte_value(ctx: &mut noprop::TestCaseContext) -> VarInt {
    VarInt::new(noprop::sample_u64_in(ctx, 16384..1073741824)).expect("test must succeed")
}

/// 8 バイトエンコード範囲の値を生成 (1073741824-MAX)
fn eight_byte_value(ctx: &mut noprop::TestCaseContext) -> VarInt {
    VarInt::new(noprop::sample_u64_in(ctx, 1073741824..=VarInt::MAX.get()))
        .expect("test must succeed")
}

/// Property: エンコード -> デコードのラウンドトリップで値が保存される
#[test]
fn prop_roundtrip_preserves_value() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_VARINT_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let value = valid_varint(ctx);
        let mut buf = [0u8; 8];
        let encoded_len = varint::encode(&mut buf, value).expect("test must succeed");
        let (decoded, decoded_len) = varint::decode(&buf).expect("test must succeed");

        assert_eq!(value, decoded, "Roundtrip failed for value {}", value);
        assert_eq!(
            encoded_len, decoded_len,
            "Length mismatch for value {}",
            value
        );
        Ok(())
    })?;
    Ok(())
}

/// Property: VarInt::encoded_len() が実際のエンコード長と一致する
#[test]
fn prop_encoded_len_matches_actual() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_VARINT_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let value = valid_varint(ctx);
        let expected_len = value.encoded_len();
        let mut buf = [0u8; 8];
        let actual_len = varint::encode(&mut buf, value).expect("test must succeed");

        assert_eq!(
            expected_len, actual_len,
            "encoded_len mismatch for {}",
            value
        );
        Ok(())
    })?;
    Ok(())
}

/// Property: 任意 VarInt は `encoded_len()` 分のバッファで必ずエンコードできる
/// (`EncodeError::ValueTooLarge` を削除した正当性: 値域は型レベルで保証される)
#[test]
fn prop_encode_succeeds_for_any_varint() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_VARINT_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let value = valid_varint(ctx);
        let mut buf = vec![0u8; value.encoded_len()];
        let result = varint::encode(&mut buf, value);
        assert!(result.is_ok(), "encode should succeed for any VarInt");
        Ok(())
    })?;
    Ok(())
}

/// Property: バッファ長が `encoded_len()` 未満なら必ず `BufferTooShort` を返す
#[test]
fn prop_short_buffer_returns_buffer_too_short() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_VARINT_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let value = valid_varint(ctx);
        let shortfall = noprop::sample_usize_in(ctx, 1..=8);
        let need = value.encoded_len();
        // shortfall 分だけ短いバッファ (need == 1 の場合は 0 バイトバッファ)
        let len = need.saturating_sub(shortfall);
        let mut buf = vec![0u8; len];
        let result = varint::encode(&mut buf, value);
        assert_eq!(result, Err(varint::EncodeError::BufferTooShort));
        Ok(())
    })?;
    Ok(())
}

/// Property: 1 バイト値は 1 バイトでエンコードされる
#[test]
fn prop_one_byte_encoding() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_VARINT_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let value = one_byte_value(ctx);
        let len = value.encoded_len();
        assert_eq!(len, 1, "Value {} should encode to 1 byte", value);

        let mut buf = [0u8; 8];
        varint::encode(&mut buf, value).expect("test must succeed");
        // 上位 2 ビットは 00
        assert_eq!(buf[0] >> 6, 0, "1-byte prefix should be 00");
        Ok(())
    })?;
    Ok(())
}

/// Property: 2 バイト値は 2 バイトでエンコードされる
#[test]
fn prop_two_byte_encoding() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_VARINT_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let value = two_byte_value(ctx);
        let len = value.encoded_len();
        assert_eq!(len, 2, "Value {} should encode to 2 bytes", value);

        let mut buf = [0u8; 8];
        varint::encode(&mut buf, value).expect("test must succeed");
        // 上位 2 ビットは 01
        assert_eq!(buf[0] >> 6, 1, "2-byte prefix should be 01");
        Ok(())
    })?;
    Ok(())
}

/// Property: 4 バイト値は 4 バイトでエンコードされる
#[test]
fn prop_four_byte_encoding() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_VARINT_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let value = four_byte_value(ctx);
        let len = value.encoded_len();
        assert_eq!(len, 4, "Value {} should encode to 4 bytes", value);

        let mut buf = [0u8; 8];
        varint::encode(&mut buf, value).expect("test must succeed");
        // 上位 2 ビットは 10
        assert_eq!(buf[0] >> 6, 2, "4-byte prefix should be 10");
        Ok(())
    })?;
    Ok(())
}

/// Property: 8 バイト値は 8 バイトでエンコードされる
#[test]
fn prop_eight_byte_encoding() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_VARINT_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let value = eight_byte_value(ctx);
        let len = value.encoded_len();
        assert_eq!(len, 8, "Value {} should encode to 8 bytes", value);

        let mut buf = [0u8; 8];
        varint::encode(&mut buf, value).expect("test must succeed");
        // 上位 2 ビットは 11
        assert_eq!(buf[0] >> 6, 3, "8-byte prefix should be 11");
        Ok(())
    })?;
    Ok(())
}

/// Property: peek_len() がデコード前に正しい長さを返す
#[test]
fn prop_peek_len_matches_decode_len() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_VARINT_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let value = valid_varint(ctx);
        let mut buf = [0u8; 8];
        let encoded_len = varint::encode(&mut buf, value).expect("test must succeed");
        let peeked_len = varint::peek_len(&buf).expect("test must succeed");

        assert_eq!(encoded_len, peeked_len, "peek_len mismatch for {}", value);
        Ok(())
    })?;
    Ok(())
}

/// Property: エンコード結果はビッグエンディアン順
#[test]
fn prop_encoding_is_big_endian() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_VARINT_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let value = two_byte_value(ctx);
        let mut buf = [0u8; 8];
        varint::encode(&mut buf, value).expect("test must succeed");

        // 2 バイトエンコードの場合、値は 0x4000 | value として格納
        let raw = value.get();
        let expected_high = (0x40 | ((raw >> 8) & 0x3f)) as u8;
        let expected_low = (raw & 0xff) as u8;

        assert_eq!(buf[0], expected_high, "High byte mismatch");
        assert_eq!(buf[1], expected_low, "Low byte mismatch");
        Ok(())
    })?;
    Ok(())
}

/// Property: `From<u8>` は任意 u8 で値が一致する
#[test]
fn prop_from_u8_roundtrip() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_VARINT_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let value = noprop::sample_u8(ctx);
        let v = VarInt::from(value);
        assert_eq!(v.get(), u64::from(value));
        Ok(())
    })?;
    Ok(())
}

/// Property: `From<u16>` は任意 u16 で値が一致する
#[test]
fn prop_from_u16_roundtrip() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_VARINT_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let value = noprop::sample_u16(ctx);
        let v = VarInt::from(value);
        assert_eq!(v.get(), u64::from(value));
        Ok(())
    })?;
    Ok(())
}

/// Property: `From<u32>` は任意 u32 で値が一致する
#[test]
fn prop_from_u32_roundtrip() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_VARINT_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let value = noprop::sample_u32(ctx);
        let v = VarInt::from(value);
        assert_eq!(v.get(), u64::from(value));
        Ok(())
    })?;
    Ok(())
}

/// Property: `TryFrom<u64>` は値域内で必ず Ok、値域外で必ず Err
#[test]
fn prop_try_from_u64_value_domain() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_VARINT_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let value = noprop::sample_u64(ctx);
        let result = VarInt::try_from(value);
        if value <= VarInt::MAX.get() {
            assert!(result.is_ok());
            assert_eq!(result.expect("test must succeed").get(), value);
        } else {
            assert!(result.is_err());
        }
        Ok(())
    })?;
    Ok(())
}

/// Property: `VarInt::Display` 出力は内部値の 10 進表現と一致する
#[test]
fn prop_display_matches_u64() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_VARINT_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let value = valid_varint(ctx);
        assert_eq!(format!("{value}"), format!("{}", value.get()));
        Ok(())
    })?;
    Ok(())
}

/// Property: `from_static` と `new` が同じ値を返す (`const fn` 検査と
/// ランタイム検査のロジック一致)
#[test]
fn prop_from_static_matches_new() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_VARINT_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let value = noprop::sample_u64_in(ctx, 0..=VarInt::MAX.get());
        let via_new = VarInt::new(value).expect("test must succeed");
        let via_static = VarInt::from_static(value);
        assert_eq!(via_new, via_static);
        Ok(())
    })?;
    Ok(())
}

/// Property: VarInt 範囲外の `u64` は `new` で必ず Err、`TryFrom<u64>` でも必ず Err
/// (構築 API の値域判定が一貫している)
#[test]
fn prop_invalid_varint_rejected() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_VARINT_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let value = invalid_varint_u64(ctx);
        assert!(VarInt::new(value).is_err());
        assert!(VarInt::try_from(value).is_err());
        Ok(())
    })?;
    Ok(())
}
