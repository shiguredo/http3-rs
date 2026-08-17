//! Capsule の capsule_type 検証・Unknown ラウンドトリップ・不完全バッファの Sans I/O 挙動
//! (既知バリアントのラウンドトリップは pbt/tests/prop_capsule.rs に集約)

use shiguredo_http3::webtransport::Capsule;

/// 有効なエラーコード (32-bit)
fn valid_error_code(ctx: &mut noprop::TestCaseContext) -> u32 {
    noprop::sample_u32(ctx)
}

/// 有効なエラーメッセージ (最大 1024 バイト)
fn valid_error_message(ctx: &mut noprop::TestCaseContext) -> String {
    let len = noprop::sample_usize_in(ctx, 0..=1024);
    let mut msg = Vec::new();
    for _ in 0..len {
        msg.push(0x20 + noprop::sample_usize_in(ctx, 0..=0x5f) as u8);
    }
    String::from_utf8(msg).expect("ASCII range bytes are always valid UTF-8")
}

/// 有効な最大値 (可変長整数範囲)
fn valid_maximum(ctx: &mut noprop::TestCaseContext) -> u64 {
    noprop::sample_u64_in(ctx, 0..=(1 << 62) - 1)
}

/// 有効な Unknown Capsule
fn valid_unknown_capsule(ctx: &mut noprop::TestCaseContext) -> Capsule {
    let capsule_type = noprop::sample_u64_in(ctx, 0x100000..0x200000);
    let payload_len = noprop::sample_usize_in(ctx, 0..256);
    let payload = noprop::sample_bytes_vec(ctx, payload_len);
    Capsule::Unknown {
        capsule_type,
        payload,
    }
}

/// Property: Unknown Capsule のラウンドトリップ
#[test]
fn prop_unknown_capsule_roundtrip() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_CAPSULE_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let capsule = valid_unknown_capsule(ctx);
        let mut buf = Vec::new();
        capsule.encode(&mut buf);

        let (decoded, consumed) = Capsule::decode(&buf)
            .expect("decode should not error")
            .expect("decode should not be incomplete");
        assert_eq!(consumed, buf.len());
        assert_eq!(decoded, capsule);
        Ok(())
    })?;
    Ok(())
}

/// Property: Capsule の capsule_type() が正しい値を返す
#[test]
fn prop_capsule_type_consistent() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_CAPSULE_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let error_code = valid_error_code(ctx);
        let maximum = noprop::sample_u64_in(ctx, 0..1000);
        let bidirectional = noprop::sample_bool(ctx);

        let close = Capsule::CloseSession {
            error_code,
            message: String::new(),
        };
        assert_eq!(close.capsule_type(), 0x2843);

        let drain = Capsule::DrainSession;
        assert_eq!(drain.capsule_type(), 0x78ae);

        let max_data = Capsule::MaxData { maximum };
        assert_eq!(max_data.capsule_type(), 0x190B4D3D);

        let max_streams = Capsule::MaxStreams {
            bidirectional,
            maximum,
        };
        if bidirectional {
            assert_eq!(max_streams.capsule_type(), 0x190B4D3F);
        } else {
            assert_eq!(max_streams.capsule_type(), 0x190B4D40);
        }

        let data_blocked = Capsule::DataBlocked { maximum };
        assert_eq!(data_blocked.capsule_type(), 0x190B4D41);

        let streams_blocked = Capsule::StreamsBlocked {
            bidirectional,
            maximum,
        };
        if bidirectional {
            assert_eq!(streams_blocked.capsule_type(), 0x190B4D43);
        } else {
            assert_eq!(streams_blocked.capsule_type(), 0x190B4D44);
        }
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// 不完全バッファの Sans I/O 挙動
// =============================================================================

/// Property: 不完全なバッファでは None を返す (CloseSession)
#[test]
fn prop_capsule_incomplete_buffer_close_session() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_CAPSULE_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let error_code = noprop::sample_u32(ctx);
        let message = valid_error_message(ctx);
        let cut_ratio = noprop::sample_f64_in(ctx, 0.1, 0.9);

        let capsule = Capsule::CloseSession {
            error_code,
            message,
        };

        let mut buf = Vec::new();
        capsule.encode(&mut buf);

        let cut_at = ((buf.len() as f64) * cut_ratio) as usize;
        let incomplete = &buf[..cut_at.max(1)];

        let result = Capsule::decode(incomplete);
        assert!(
            matches!(result, Ok(None)),
            "Incomplete buffer should return None"
        );
        Ok(())
    })?;
    Ok(())
}

/// Property: 不完全なバッファでは None を返す (MaxData)
#[test]
fn prop_capsule_incomplete_buffer_max_data() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_CAPSULE_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let maximum = valid_maximum(ctx);
        let cut_ratio = noprop::sample_f64_in(ctx, 0.1, 0.9);

        let capsule = Capsule::MaxData { maximum };

        let mut buf = Vec::new();
        capsule.encode(&mut buf);

        if buf.len() > 1 {
            let cut_at = ((buf.len() as f64) * cut_ratio) as usize;
            let incomplete = &buf[..cut_at.max(1)];

            let result = Capsule::decode(incomplete);
            assert!(
                matches!(result, Ok(None)),
                "Incomplete buffer should return None"
            );
        }
        Ok(())
    })?;
    Ok(())
}

/// Property: 空バッファでは None を返す
#[test]
fn prop_capsule_empty_buffer_returns_none() {
    let result = Capsule::decode(&[]);
    assert!(matches!(result, Ok(None)));
}
