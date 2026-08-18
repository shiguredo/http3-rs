//! Property-Based Testing for HTTP/3 ストリーム (RFC 9114 Section 6)

use pbt::strategies::sample_varint_raw_in;
use shiguredo_http3::stream::{StreamKind, StreamState, UniStreamType};

// =============================================================================
// (b) Reset は全ての状態から遷移可能で、Reset 後は send/receive 不可
// =============================================================================

/// Property: 任意の状態遷移列の後に reset() を呼ぶと、can_send()==false かつ can_receive()==false
#[test]
fn prop_reset_disables_send_and_receive() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_STREAM_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let op_count = noprop::sample_usize_in(ctx, 0..=10);
        let mut state = StreamState::Open;

        // 任意の操作列を適用
        for _ in 0..op_count {
            match noprop::sample_choice(ctx, &[0u8, 1, 2]) {
                0 => state.close_local(),
                1 => state.close_remote(),
                _ => state.reset(),
            }
        }

        // reset() を適用
        state.reset();

        assert_eq!(state, StreamState::Reset);
        assert!(!state.can_send(), "Reset 後に can_send() が true");
        assert!(!state.can_receive(), "Reset 後に can_receive() が true");
        assert!(state.is_reset(), "Reset 後に is_reset() が false");
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// (c) StreamKind のストリーム ID 分類が QUIC 仕様に準拠
// =============================================================================

/// Property: 任意の stream_id に対して下位 2 ビットに基づく StreamKind 分類が正しい
#[test]
fn prop_stream_kind_classification() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_STREAM_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let stream_id = sample_varint_raw_in(ctx, 0..=999_999);
        let kind = StreamKind::from_stream_id(stream_id);
        let low_bits = stream_id & 0x03;

        // 下位 2 ビットに基づく分類 (RFC 9000 Table 1)
        match low_bits {
            0x00 => {
                assert_eq!(kind, StreamKind::ClientBidi);
                assert!(kind.is_bidirectional());
                assert!(kind.is_client_initiated());
                assert!(!kind.is_unidirectional());
                assert!(!kind.is_server_initiated());
            }
            0x01 => {
                assert_eq!(kind, StreamKind::ServerBidi);
                assert!(kind.is_bidirectional());
                assert!(kind.is_server_initiated());
            }
            0x02 => {
                assert_eq!(kind, StreamKind::ClientUni);
                assert!(kind.is_unidirectional());
                assert!(kind.is_client_initiated());
            }
            0x03 => {
                assert_eq!(kind, StreamKind::ServerUni);
                assert!(kind.is_unidirectional());
                assert!(kind.is_server_initiated());
            }
            _ => unreachable!(),
        }
        Ok(())
    })?;
    Ok(())
}

/// Property: bidirectional と unidirectional は排他的
#[test]
fn prop_stream_kind_bidi_uni_exclusive() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_STREAM_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let stream_id = sample_varint_raw_in(ctx, 0..=999_999);
        let kind = StreamKind::from_stream_id(stream_id);
        assert_ne!(
            kind.is_bidirectional(),
            kind.is_unidirectional(),
            "bidirectional と unidirectional が同じ値"
        );
        Ok(())
    })?;
    Ok(())
}

/// Property: client_initiated と server_initiated は排他的
#[test]
fn prop_stream_kind_initiator_exclusive() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_STREAM_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let stream_id = sample_varint_raw_in(ctx, 0..=999_999);
        let kind = StreamKind::from_stream_id(stream_id);
        assert_ne!(
            kind.is_client_initiated(),
            kind.is_server_initiated(),
            "client_initiated と server_initiated が同じ値"
        );
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// (d) UniStreamType::is_reserved の GREASE パターン
// =============================================================================

/// Property: 0x1f * N + 0x21 形式の値は is_reserved() == true
#[test]
fn prop_reserved_stream_type_formula() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_STREAM_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let n = sample_varint_raw_in(ctx, 0..=9_999);
        let t = 0x1f * n + 0x21;
        assert!(
            UniStreamType::is_reserved(t),
            "0x1f * {} + 0x21 = {:#x} が is_reserved() == false",
            n,
            t
        );
        Ok(())
    })?;
    Ok(())
}

/// Property: 0x1f * N + 0x21 形式でない値は is_reserved() == false (0x21 未満 + その他)
#[test]
fn prop_non_reserved_stream_type() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_STREAM_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let t = noprop::sample_u64_in(ctx, 0..0x21);
        assert!(
            !UniStreamType::is_reserved(t),
            "{:#x} が is_reserved() == true (0x21 未満なのに予約済み)",
            t
        );
        Ok(())
    })?;
    Ok(())
}

/// Property: 0x21 以上で 0x1f * N + 0x21 形式でない値は is_reserved() == false
#[test]
fn prop_non_grease_above_threshold() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_STREAM_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let n = sample_varint_raw_in(ctx, 0..=9_999);
        let offset = noprop::sample_u64_in(ctx, 1..0x1f);
        let t = 0x1f * n + 0x21 + offset;
        // offset が 0x1f の倍数でない限り is_reserved() == false のはず
        // ただし offset + 0x21 が次の GREASE 値に一致する場合がある。
        // 正確には (t - 0x21) % 0x1f != 0 なら false
        if !(t - 0x21).is_multiple_of(0x1f) {
            assert!(
                !UniStreamType::is_reserved(t),
                "{:#x} が is_reserved() == true (GREASE パターンに合致しないのに)",
                t
            );
        }
        Ok(())
    })?;
    Ok(())
}
