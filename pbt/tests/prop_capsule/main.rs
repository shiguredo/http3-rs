//! Property-Based Testing for WebTransport Capsule Protocol
//! (draft-ietf-webtrans-http3-15 Section 5.6, 6)

use shiguredo_http3::webtransport::{Capsule, CapsuleValidationError, MAX_STREAMS_LIMIT};

// =============================================================================
// 生成ヘルパー
// =============================================================================

/// 有効な可変長整数の最大値
const MAX_VARINT: u64 = (1 << 62) - 1;

/// CloseSession Capsule を生成 (メッセージは最大 1024 バイトの ASCII)
fn close_session_capsule(ctx: &mut noprop::TestCaseContext) -> Capsule {
    let error_code = noprop::sample_u32(ctx);
    let msg_len = noprop::sample_usize_in(ctx, 0..=1024);
    let mut msg = Vec::new();
    for _ in 0..msg_len {
        msg.push(0x20 + noprop::sample_usize_in(ctx, 0..=0x5f) as u8);
    }
    let message = String::from_utf8(msg).unwrap_or_default();
    Capsule::CloseSession {
        error_code,
        message,
    }
}

/// MaxData Capsule を生成
fn max_data_capsule(ctx: &mut noprop::TestCaseContext) -> Capsule {
    let maximum = noprop::sample_u64_in(ctx, 0..=MAX_VARINT);
    Capsule::MaxData { maximum }
}

/// MaxStreams Capsule を生成
fn max_streams_capsule(ctx: &mut noprop::TestCaseContext) -> Capsule {
    let bidirectional = noprop::sample_bool(ctx);
    let maximum = noprop::sample_u64_in(ctx, 0..=MAX_VARINT);
    Capsule::MaxStreams {
        bidirectional,
        maximum,
    }
}

/// DataBlocked Capsule を生成
fn data_blocked_capsule(ctx: &mut noprop::TestCaseContext) -> Capsule {
    let maximum = noprop::sample_u64_in(ctx, 0..=MAX_VARINT);
    Capsule::DataBlocked { maximum }
}

/// StreamsBlocked Capsule を生成
fn streams_blocked_capsule(ctx: &mut noprop::TestCaseContext) -> Capsule {
    let bidirectional = noprop::sample_bool(ctx);
    let maximum = noprop::sample_u64_in(ctx, 0..=MAX_VARINT);
    Capsule::StreamsBlocked {
        bidirectional,
        maximum,
    }
}

/// 全ての既知の Capsule 型を生成する
fn any_known_capsule(ctx: &mut noprop::TestCaseContext) -> Capsule {
    match noprop::sample_weighted_index(ctx, &[1u32; 6]) {
        0 => close_session_capsule(ctx),
        1 => Capsule::DrainSession,
        2 => max_data_capsule(ctx),
        3 => max_streams_capsule(ctx),
        4 => data_blocked_capsule(ctx),
        _ => streams_blocked_capsule(ctx),
    }
}

// =============================================================================
// (a) Capsule encode/decode ラウンドトリップ
// =============================================================================

/// Property: 任意の既知の Capsule を encode → decode すると元と一致する
#[test]
fn prop_capsule_roundtrip() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_CAPSULE_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let capsule = any_known_capsule(ctx);
        let mut buf = Vec::new();
        capsule.encode(&mut buf);

        let (decoded, consumed) = Capsule::decode(&buf)
            .expect("decode should not error for valid encoded capsule")
            .expect("decode should not be incomplete for valid encoded capsule");

        assert_eq!(&decoded, &capsule, "ラウンドトリップで Capsule が変化した");
        assert_eq!(consumed, buf.len(), "消費バイト数がバッファ長と不一致");
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// (b) encode_as_data_frame ラウンドトリップ
// =============================================================================

/// Property: encode_as_data_frame の出力を decode_frame で DATA として取り出し、
/// そのペイロードを Capsule::decode すると元と一致する
#[test]
fn prop_capsule_data_frame_roundtrip() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_CAPSULE_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let capsule = any_known_capsule(ctx);
        let mut buf = Vec::new();
        capsule.encode_as_data_frame(&mut buf);

        // DATA フレームとしてデコード
        let (frame, consumed) =
            shiguredo_http3::frame::decode_frame(&buf).expect("decode_frame should succeed");
        assert_eq!(consumed, buf.len(), "consumed != buf.len()");

        // DATA フレームからペイロードを取り出す
        match frame {
            shiguredo_http3::Frame::Data(payload) => {
                let (decoded, cap_consumed) = Capsule::decode(payload.data())
                    .expect("Capsule::decode should not error")
                    .expect("Capsule::decode should not be incomplete");
                assert_eq!(&decoded, &capsule, "カプセルがラウンドトリップで変化した");
                assert_eq!(cap_consumed, payload.len(), "カプセル消費バイト数が不一致");
            }
            other => {
                panic!("DATA フレームでない: {:?}", other);
            }
        }
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// (c) エンコードバイト列の消費量が encode の出力長と一致
// =============================================================================

/// Property: decode の consumed がバッファ長と一致する
#[test]
fn prop_capsule_consumed_equals_buffer_length() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_CAPSULE_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let capsule = any_known_capsule(ctx);
        let mut buf = Vec::new();
        capsule.encode(&mut buf);

        let (_, consumed) = Capsule::decode(&buf)
            .expect("decode should not error")
            .expect("decode should not be incomplete");

        assert_eq!(consumed, buf.len(), "consumed != buf.len()");
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// (c) validate_max_streams の単調増加制約
// =============================================================================

/// Property: maximum > current_max かつ maximum <= MAX_STREAMS_LIMIT なら Ok
#[test]
fn prop_validate_max_streams_ok() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_CAPSULE_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let (current_max, maximum) = noprop::sample_with_rejection(ctx, 64, |ctx| {
            let current_max = noprop::sample_u64_in(ctx, 0..=MAX_STREAMS_LIMIT);
            let delta = noprop::sample_u64_in(ctx, 1..1000);
            let maximum = current_max.saturating_add(delta).min(MAX_STREAMS_LIMIT);
            (maximum > current_max).then_some((current_max, maximum))
        });
        let result = Capsule::validate_max_streams(maximum, current_max);
        assert!(
            result.is_ok(),
            "maximum > current_max かつ上限以下なのに Err: maximum={}, current_max={}",
            maximum,
            current_max
        );
        Ok(())
    })?;
    Ok(())
}

/// Property: maximum <= current_max なら MaxStreamsDecreased
/// (draft-16: "does not increase")
#[test]
fn prop_validate_max_streams_not_increased() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_CAPSULE_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let current_max = noprop::sample_u64_in(ctx, 1..=MAX_STREAMS_LIMIT);
        let delta = noprop::sample_u64_in(ctx, 0..1000);
        let maximum = current_max.saturating_sub(delta);
        // saturating_sub により maximum <= current_max は常に成り立つ

        let result = Capsule::validate_max_streams(maximum, current_max);
        assert_eq!(
            result,
            Err(CapsuleValidationError::MaxStreamsDecreased),
            "maximum <= current_max なのに MaxStreamsDecreased でない: maximum={}, current_max={}",
            maximum,
            current_max
        );
        Ok(())
    })?;
    Ok(())
}

/// Property: maximum > MAX_STREAMS_LIMIT なら MaxStreamsExceedsLimit
#[test]
fn prop_validate_max_streams_exceeds_limit() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_CAPSULE_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let excess = noprop::sample_u64_in(ctx, 1..1000);
        let maximum = MAX_STREAMS_LIMIT + excess;
        let result = Capsule::validate_max_streams(maximum, 0);
        assert_eq!(
            result,
            Err(CapsuleValidationError::MaxStreamsExceedsLimit),
            "maximum > MAX_STREAMS_LIMIT なのに MaxStreamsExceedsLimit でない: maximum={}",
            maximum
        );
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// (d) validate_max_data の単調増加制約
// =============================================================================

/// Property: maximum > current_max なら Ok
#[test]
fn prop_validate_max_data_ok() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_CAPSULE_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let current_max = noprop::sample_u64_in(ctx, 0..=MAX_VARINT / 2);
        let delta = noprop::sample_u64_in(ctx, 1..1000);
        let maximum = current_max.saturating_add(delta);
        // current_max は MAX_VARINT / 2 以下で delta は 1000 未満のため
        // saturating_add は飽和せず maximum > current_max が常に成り立つ
        let result = Capsule::validate_max_data(maximum, current_max);
        assert!(
            result.is_ok(),
            "maximum > current_max なのに Err: maximum={}, current_max={}",
            maximum,
            current_max
        );
        Ok(())
    })?;
    Ok(())
}

/// Property: maximum <= current_max なら MaxDataDecreased
/// (draft-16: "does not increase")
#[test]
fn prop_validate_max_data_not_increased() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_CAPSULE_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let current_max = noprop::sample_u64_in(ctx, 1..=MAX_VARINT);
        let delta = noprop::sample_u64_in(ctx, 0..1000);
        let maximum = current_max.saturating_sub(delta);
        // saturating_sub により maximum <= current_max は常に成り立つ

        let result = Capsule::validate_max_data(maximum, current_max);
        assert_eq!(
            result,
            Err(CapsuleValidationError::MaxDataDecreased),
            "maximum <= current_max なのに MaxDataDecreased でない: maximum={}, current_max={}",
            maximum,
            current_max
        );
        Ok(())
    })?;
    Ok(())
}

/// Property: maximum > MAX_VARINT なら MaxDataExceedsLimit
#[test]
fn prop_validate_max_data_exceeds_limit() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_CAPSULE_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let excess = noprop::sample_u64_in(ctx, 1..1000);
        let maximum = MAX_VARINT + excess;
        let result = Capsule::validate_max_data(maximum, 0);
        assert_eq!(
            result,
            Err(CapsuleValidationError::MaxDataExceedsLimit),
            "maximum > MAX_VARINT なのに MaxDataExceedsLimit でない: maximum={}",
            maximum
        );
        Ok(())
    })?;
    Ok(())
}
