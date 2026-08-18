//! Session の状態遷移・フロー制御・バッファリング・GOAWAY・終了処理プロパティ
//! (draft-ietf-webtrans-http3-15 Section 3, 4.5, 4.6, 4.7, 5.6, 6)

use pbt::strategies::{sample_len, sample_varint_raw_in};
use shiguredo_http3::webtransport::{
    Capsule, FlowControlLimits, MAX_STREAMS_LIMIT, Session, Stream,
};

/// 有効なエラーコード (32-bit)
fn valid_error_code(ctx: &mut noprop::TestCaseContext) -> u32 {
    noprop::sample_u32(ctx)
}

/// 有効なエラーメッセージ (最大 1024 バイト)
fn valid_error_message(ctx: &mut noprop::TestCaseContext) -> String {
    let len = sample_len(ctx, 0..=1024);
    let mut msg = Vec::new();
    for _ in 0..len {
        msg.push(0x20 + noprop::sample_usize_in(ctx, 0..=0x5f) as u8);
    }
    String::from_utf8(msg).expect("ASCII range bytes are always valid UTF-8")
}

// =============================================================================
// フロー制御の単調増加制約 (draft-ietf-webtrans-http3-16 Section 5.6.2, 5.6.4)
// =============================================================================

/// Property: MaxData が減少した場合にエラー
#[test]
fn prop_session_max_data_non_monotonic_error() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_SESSION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let initial = noprop::sample_u64_in(ctx, 100..10000);
        let decrease = noprop::sample_u64_in(ctx, 1..100);
        let mut session = Session::new(0);

        let result = session.process_capsule(&Capsule::MaxData { maximum: initial });
        assert!(result.is_ok());
        assert_eq!(session.remote_limits().max_data, initial);

        let smaller = initial.saturating_sub(decrease);
        let result = session.process_capsule(&Capsule::MaxData { maximum: smaller });
        assert!(result.is_err());

        assert_eq!(session.remote_limits().max_data, initial);
        Ok(())
    })?;
    Ok(())
}

/// Property: MaxData が厳密に増加すれば成功
///
/// draft-16 Section 5.6.4: "does not increase" は WT_FLOW_CONTROL_ERROR。
/// セッション初期値は 0 のため、最初の値も 1 以上が必要。
#[test]
fn prop_session_max_data_monotonic_ok() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_SESSION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let initial = noprop::sample_u64_in(ctx, 1..10000);
        let increase = noprop::sample_u64_in(ctx, 1..10000);
        let mut session = Session::new(0);

        let result = session.process_capsule(&Capsule::MaxData { maximum: initial });
        assert!(result.is_ok());

        let larger = initial.saturating_add(increase);
        let result = session.process_capsule(&Capsule::MaxData { maximum: larger });
        assert!(result.is_ok());
        assert_eq!(session.remote_limits().max_data, larger);
        Ok(())
    })?;
    Ok(())
}

/// Property: MaxStreams (双方向) が減少した場合にエラー
#[test]
fn prop_session_max_streams_bidi_non_monotonic_error() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_SESSION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let initial = noprop::sample_u64_in(ctx, 100..10000);
        let decrease = noprop::sample_u64_in(ctx, 1..100);
        let mut session = Session::new(0);

        let result = session.process_capsule(&Capsule::MaxStreams {
            bidirectional: true,
            maximum: initial,
        });
        assert!(result.is_ok());

        let smaller = initial.saturating_sub(decrease);
        let result = session.process_capsule(&Capsule::MaxStreams {
            bidirectional: true,
            maximum: smaller,
        });
        assert!(result.is_err());
        Ok(())
    })?;
    Ok(())
}

/// Property: MaxStreams (単方向) が減少した場合にエラー
#[test]
fn prop_session_max_streams_uni_non_monotonic_error() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_SESSION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let initial = noprop::sample_u64_in(ctx, 100..10000);
        let decrease = noprop::sample_u64_in(ctx, 1..100);
        let mut session = Session::new(0);

        let result = session.process_capsule(&Capsule::MaxStreams {
            bidirectional: false,
            maximum: initial,
        });
        assert!(result.is_ok());

        let smaller = initial.saturating_sub(decrease);
        let result = session.process_capsule(&Capsule::MaxStreams {
            bidirectional: false,
            maximum: smaller,
        });
        assert!(result.is_err());
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// フロー制御リミット境界テスト
// =============================================================================

/// Property: ストリーム作成可否の境界テスト (単方向)
#[test]
fn prop_session_can_create_stream_boundary_uni() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_SESSION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let limit = noprop::sample_u64_in(ctx, 1..100);
        let mut session = Session::new(0);
        session.set_established();
        session.remote_limits_mut().max_streams_uni = limit;

        session.flow_state_mut().streams_uni_opened = limit - 1;
        assert!(session.can_create_unidirectional_stream());

        session.flow_state_mut().streams_uni_opened = limit;
        assert!(!session.can_create_unidirectional_stream());

        session.flow_state_mut().streams_uni_opened = limit + 1;
        assert!(!session.can_create_unidirectional_stream());
        Ok(())
    })?;
    Ok(())
}

/// Property: ストリーム作成可否の境界テスト (双方向)
#[test]
fn prop_session_can_create_stream_boundary_bidi() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_SESSION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let limit = noprop::sample_u64_in(ctx, 1..100);
        let mut session = Session::new(0);
        session.set_established();
        session.remote_limits_mut().max_streams_bidi = limit;

        session.flow_state_mut().streams_bidi_opened = limit - 1;
        assert!(session.can_create_bidirectional_stream());

        session.flow_state_mut().streams_bidi_opened = limit;
        assert!(!session.can_create_bidirectional_stream());
        Ok(())
    })?;
    Ok(())
}

/// Property: データ送信可否の境界テスト
#[test]
fn prop_session_can_send_data_boundary() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_SESSION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let limit = noprop::sample_u64_in(ctx, 100..10000);
        let sent = noprop::sample_u64_in(ctx, 0..100);
        let mut session = Session::new(0);
        session.set_established();
        session.remote_limits_mut().max_data = limit;
        session.flow_state_mut().data_sent = sent;

        let remaining = limit - sent;

        assert!(session.can_send_data(remaining));
        assert!(!session.can_send_data(remaining + 1));
        Ok(())
    })?;
    Ok(())
}

/// Property: Established/Draining 以外ではストリーム作成不可
#[test]
fn prop_session_cannot_create_stream_in_wrong_state() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_SESSION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let limit = noprop::sample_u64_in(ctx, 1..100);
        let mut session = Session::new(0);
        session.remote_limits_mut().max_streams_uni = limit;
        session.remote_limits_mut().max_streams_bidi = limit;

        // Pending 状態
        assert!(!session.can_create_unidirectional_stream());
        assert!(!session.can_create_bidirectional_stream());

        // Connecting 状態
        session.set_connecting();
        assert!(!session.can_create_unidirectional_stream());
        assert!(!session.can_create_bidirectional_stream());

        // Closed 状態
        session.close(None);
        assert!(!session.can_create_unidirectional_stream());
        assert!(!session.can_create_bidirectional_stream());
        Ok(())
    })?;
    Ok(())
}

/// Property: Established/Draining 以外ではデータ送信不可
#[test]
fn prop_session_cannot_send_in_wrong_state() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_SESSION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let limit = noprop::sample_u64_in(ctx, 100..10000);
        let mut session = Session::new(0);
        session.remote_limits_mut().max_data = limit;

        // Pending 状態
        assert!(!session.can_send_data(1));

        // Connecting 状態
        session.set_connecting();
        assert!(!session.can_send_data(1));

        // Closed 状態
        session.close(None);
        assert!(!session.can_send_data(1));
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// バッファリング (draft-ietf-webtrans-http3-15 Section 4.6)
// =============================================================================

/// Property: MAX_BUFFERED_STREAMS (100) までバッファリング成功
#[test]
fn prop_session_buffer_streams_up_to_limit() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_SESSION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let count = noprop::sample_usize_in(ctx, 1..=100);
        let mut session = Session::new(0);

        for i in 0..count {
            assert!(session.buffer_incoming_stream(i as u64 * 4, false));
        }

        let buffered = session.take_buffered_streams();
        assert_eq!(buffered.len(), count);
        Ok(())
    })?;
    Ok(())
}

/// Property: MAX_BUFFERED_DATAGRAMS (100) までバッファリング成功
#[test]
fn prop_session_buffer_datagrams_up_to_limit() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_SESSION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let count = noprop::sample_usize_in(ctx, 1..=100);
        let mut session = Session::new(0);

        for i in 0..count {
            assert!(session.buffer_datagram(vec![i as u8]));
        }

        let buffered = session.take_buffered_datagrams();
        assert_eq!(buffered.len(), count);
        Ok(())
    })?;
    Ok(())
}

/// Property: take 後のバッファは空になる
#[test]
fn prop_session_take_buffered_empties_buffer() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_SESSION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let stream_count = noprop::sample_usize_in(ctx, 1..=10);
        let datagram_count = noprop::sample_usize_in(ctx, 1..=10);
        let mut session = Session::new(0);

        for i in 0..stream_count {
            session.buffer_incoming_stream(i as u64 * 4, false);
        }
        for _ in 0..datagram_count {
            session.buffer_datagram(vec![0]);
        }

        let _streams = session.take_buffered_streams();
        let _datagrams = session.take_buffered_datagrams();

        assert!(session.take_buffered_streams().is_empty());
        assert!(session.take_buffered_datagrams().is_empty());
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// GOAWAY (draft-ietf-webtrans-http3-15 Section 4.7)
// =============================================================================

/// Property: handle_goaway 後も既存ストリームは保持され can_send() == true
#[test]
fn prop_session_handle_goaway_preserves_streams() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_SESSION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let stream_count = noprop::sample_usize_in(ctx, 1..10);
        let mut session = Session::new(0);
        session.set_established();
        session.remote_limits_mut().max_data = 100000;

        for i in 0..stream_count {
            session.add_stream(Stream::new(i as u64 * 4, 0, true));
        }

        session.handle_goaway();

        assert_eq!(session.stream_count(), stream_count);
        for i in 0..stream_count {
            assert!(session.get_stream(i as u64 * 4).is_some());
        }
        assert!(session.state().can_send());
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// セッション終了処理 (draft-ietf-webtrans-http3-15 Section 6)
// =============================================================================

/// Property: 任意の初期状態 (Established/Draining) で on_connect_stream_closed() →
/// is_close_session_received() == true かつ is_closed() == true
#[test]
fn prop_session_on_connect_stream_closed_sets_flags() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_SESSION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let start_draining = noprop::sample_bool(ctx);
        let mut session = Session::new(0);
        session.set_established();

        if start_draining {
            session.set_draining();
        }

        session.on_connect_stream_closed();

        assert!(session.is_close_session_received());
        assert!(session.is_closed());
        Ok(())
    })?;
    Ok(())
}

/// Property: CloseSession Capsule 受信後に on_connect_stream_closed() →
/// 最初のエラー情報が保持される
#[test]
fn prop_session_on_connect_stream_closed_preserves_first_error() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_SESSION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let error_code = valid_error_code(ctx);
        let message = valid_error_message(ctx);
        let mut session = Session::new(0);
        session.set_established();

        session
            .process_capsule(&Capsule::CloseSession {
                error_code,
                message: message.clone(),
            })
            .expect("CloseSession capsule should be accepted");

        let first_error = session.close_error().cloned();

        session.on_connect_stream_closed();

        assert_eq!(session.close_error(), first_error.as_ref());
        assert!(session.is_close_session_received());
        Ok(())
    })?;
    Ok(())
}

/// Property: 任意のストリーム追加/削除後、stream_ids_to_reset() が
/// 残存ストリーム ID と一致
#[test]
fn prop_session_stream_ids_to_reset_matches_streams() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_SESSION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let add_count = noprop::sample_usize_in(ctx, 1..20);
        let remove_count = noprop::sample_usize_in(ctx, 0..10);
        let mut remove_indices = Vec::new();
        for _ in 0..remove_count {
            remove_indices.push(noprop::sample_usize_in(ctx, 0..20));
        }

        let mut session = Session::new(0);
        session.set_established();

        let stream_ids: Vec<u64> = (0..add_count).map(|i| i as u64 * 4).collect();
        for &sid in &stream_ids {
            session.add_stream(Stream::new(sid, 0, true));
        }

        for &idx in &remove_indices {
            if idx < stream_ids.len() {
                session.remove_stream(stream_ids[idx]);
            }
        }

        let mut reset_ids = session.stream_ids_to_reset();
        reset_ids.sort();

        let mut expected: Vec<u64> = session.streams().map(|s| s.stream_id()).collect();
        expected.sort();

        let reset_count = reset_ids.len();
        assert_eq!(reset_ids, expected);
        assert_eq!(reset_count, session.stream_count());
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// カプセルインターリーブ処理 (draft-ietf-webtrans-http3-15 Section 5.6, 6)
// =============================================================================

/// Property: MaxData/MaxStreams/DataBlocked/StreamsBlocked をランダム順で処理しても
/// 最終リミットが整合的
/// (draft-16: 単調増加制約のため値は 1 以上から生成)
#[test]
fn prop_session_interleaved_capsule_processing() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_SESSION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let data_count = noprop::sample_usize_in(ctx, 1..5);
        let mut max_data_values = Vec::new();
        for _ in 0..data_count {
            max_data_values.push(noprop::sample_u64_in(ctx, 1..10000));
        }
        let bidi_count = noprop::sample_usize_in(ctx, 1..5);
        let mut max_streams_bidi_values = Vec::new();
        for _ in 0..bidi_count {
            max_streams_bidi_values.push(noprop::sample_u64_in(ctx, 1..1000));
        }
        let uni_count = noprop::sample_usize_in(ctx, 1..5);
        let mut max_streams_uni_values = Vec::new();
        for _ in 0..uni_count {
            max_streams_uni_values.push(noprop::sample_u64_in(ctx, 1..1000));
        }

        let mut session = Session::new(0);

        let mut sorted_data = max_data_values.clone();
        sorted_data.sort();
        sorted_data.dedup();
        let mut sorted_bidi = max_streams_bidi_values.clone();
        sorted_bidi.sort();
        sorted_bidi.dedup();
        let mut sorted_uni = max_streams_uni_values.clone();
        sorted_uni.sort();
        sorted_uni.dedup();

        for &v in &sorted_data {
            let result = session.process_capsule(&Capsule::MaxData { maximum: v });
            assert!(result.is_ok());
        }
        for &v in &sorted_bidi {
            let result = session.process_capsule(&Capsule::MaxStreams {
                bidirectional: true,
                maximum: v,
            });
            assert!(result.is_ok());
        }
        for &v in &sorted_uni {
            let result = session.process_capsule(&Capsule::MaxStreams {
                bidirectional: false,
                maximum: v,
            });
            assert!(result.is_ok());
        }

        let _ = session.process_capsule(&Capsule::DataBlocked { maximum: 999 });
        let _ = session.process_capsule(&Capsule::StreamsBlocked {
            bidirectional: true,
            maximum: 999,
        });

        if let Some(&max) = sorted_data.last() {
            assert_eq!(session.remote_limits().max_data, max);
        }
        if let Some(&max) = sorted_bidi.last() {
            assert_eq!(session.remote_limits().max_streams_bidi, max);
        }
        if let Some(&max) = sorted_uni.last() {
            assert_eq!(session.remote_limits().max_streams_uni, max);
        }
        Ok(())
    })?;
    Ok(())
}

/// Property: 単調増加列の後に減少値を送ると FlowControlError
#[test]
fn prop_session_flow_control_violation_after_increase() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_SESSION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let first = noprop::sample_u64_in(ctx, 100..10000);
        let increase = noprop::sample_u64_in(ctx, 1..10000);
        let decrease = noprop::sample_u64_in(ctx, 1..100);
        let mut session = Session::new(0);
        let second = first.saturating_add(increase);

        session
            .process_capsule(&Capsule::MaxData { maximum: first })
            .expect("monotonic increase");
        session
            .process_capsule(&Capsule::MaxData { maximum: second })
            .expect("monotonic increase");

        let smaller = second.saturating_sub(decrease);
        if smaller < second {
            let result = session.process_capsule(&Capsule::MaxData { maximum: smaller });
            assert!(result.is_err());
        }
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// ストリーム追加/削除の整合性
// =============================================================================

/// Property: 追加/削除を交互に実行後、stream_count() と get_stream() が整合的
#[test]
fn prop_session_add_remove_stream_consistency() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_SESSION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let add_count = noprop::sample_usize_in(ctx, 1..20);
        let mut add_ids = Vec::new();
        for _ in 0..add_count {
            add_ids.push(noprop::sample_u64_in(ctx, 0..100));
        }
        let remove_count = noprop::sample_usize_in(ctx, 0..10);
        let mut remove_ids = Vec::new();
        for _ in 0..remove_count {
            remove_ids.push(noprop::sample_u64_in(ctx, 0..100));
        }

        let mut session = Session::new(0);

        let mut added_set = std::collections::HashSet::new();
        for &raw_id in &add_ids {
            let sid = raw_id * 4;
            if added_set.insert(sid) {
                session.add_stream(Stream::new(sid, 0, true));
            }
        }

        for &raw_id in &remove_ids {
            let sid = raw_id * 4;
            session.remove_stream(sid);
            added_set.remove(&sid);
        }

        assert_eq!(session.stream_count(), added_set.len());

        for &sid in &added_set {
            assert!(
                session.get_stream(sid).is_some(),
                "Stream {} should exist",
                sid
            );
        }

        for &raw_id in &remove_ids {
            let sid = raw_id * 4;
            if !added_set.contains(&sid) {
                assert!(
                    session.get_stream(sid).is_none(),
                    "Stream {} should not exist",
                    sid
                );
            }
        }
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// 動的ウィンドウ更新
// =============================================================================

/// Property: advertised_max は単調増加する (ストリーム)
///
/// 任意の open/close シーケンスに対して、生成される WT_MAX_STREAMS の
/// maximum は常に前回以上の値である。
#[test]
fn prop_advertised_max_monotonically_increases() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_SESSION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let concurrent_limit = noprop::sample_u64_in(ctx, 1..200);
        let num_streams = noprop::sample_usize_in(ctx, 1..500);
        let mut session = Session::new(0);
        session.set_established();
        session.initialize_local_limits(FlowControlLimits {
            max_streams_uni: concurrent_limit,
            ..FlowControlLimits::default()
        });

        let mut last_max = concurrent_limit;

        for _ in 0..num_streams {
            session.add_received_stream(false);
        }

        for _ in 0..num_streams {
            session.on_remote_stream_closed(false);

            for capsule in session.take_pending_capsules() {
                if let Capsule::MaxStreams {
                    bidirectional: false,
                    maximum,
                } = capsule
                {
                    assert!(
                        maximum >= last_max,
                        "advertised_max decreased: {} -> {}",
                        last_max,
                        maximum
                    );
                    last_max = maximum;
                }
            }
        }
        Ok(())
    })?;
    Ok(())
}

/// Property: advertised_max は MAX_STREAMS_LIMIT を超えない (ストリーム)
#[test]
fn prop_advertised_max_within_limit() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_SESSION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let concurrent_limit = sample_varint_raw_in(ctx, 1..=MAX_STREAMS_LIMIT);
        let num_cycles = noprop::sample_usize_in(ctx, 1..100);
        let mut session = Session::new(0);
        session.set_established();
        session.initialize_local_limits(FlowControlLimits {
            max_streams_uni: concurrent_limit,
            ..FlowControlLimits::default()
        });

        for _ in 0..num_cycles {
            session.add_received_stream(false);
            session.on_remote_stream_closed(false);

            for capsule in session.take_pending_capsules() {
                if let Capsule::MaxStreams { maximum, .. } = capsule {
                    assert!(
                        maximum <= MAX_STREAMS_LIMIT,
                        "advertised_max exceeds limit: {}",
                        maximum
                    );
                }
            }
        }
        Ok(())
    })?;
    Ok(())
}

/// Property: WT_STREAMS_BLOCKED は同じ maximum に対して 1 回だけ送信される
#[test]
fn prop_streams_blocked_dedup() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_SESSION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let limit = noprop::sample_u64_in(ctx, 0..50);
        let attempts = noprop::sample_usize_in(ctx, 2..20);
        let mut session = Session::new(0);
        session.set_established();
        session.remote_limits_mut().max_streams_uni = limit;

        for _ in 0..limit {
            assert!(session.try_open_stream(false));
        }

        for _ in 0..attempts {
            assert!(!session.try_open_stream(false));
        }

        let capsules = session.take_pending_capsules();
        let blocked_count = capsules
            .iter()
            .filter(|c| {
                matches!(
                    c,
                    Capsule::StreamsBlocked {
                        bidirectional: false,
                        ..
                    }
                )
            })
            .count();
        assert_eq!(
            blocked_count, 1,
            "STREAMS_BLOCKED should be sent exactly once per maximum, got {}",
            blocked_count
        );
        Ok(())
    })?;
    Ok(())
}

/// Property: セッション A → セッション B の動的ウィンドウ更新往復プロパティ
///
/// セッション A で on_remote_stream_closed により生成された WT_MAX_STREAMS カプセルを
/// セッション B の process_capsule に渡すと remote_limits が正しく更新される。
#[test]
fn prop_dynamic_max_streams_roundtrip() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_SESSION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let concurrent_limit = noprop::sample_u64_in(ctx, 1..200);
        let num_streams = noprop::sample_usize_in(ctx, 1..200);
        let mut session_a = Session::new(0);
        session_a.set_established();
        session_a.initialize_local_limits(FlowControlLimits {
            max_streams_uni: concurrent_limit,
            ..FlowControlLimits::default()
        });

        let mut session_b = Session::new(0);
        session_b.set_established();
        session_b.remote_limits_mut().max_streams_uni = concurrent_limit;

        for _ in 0..num_streams {
            session_a.add_received_stream(false);
        }

        for _ in 0..num_streams {
            session_a.on_remote_stream_closed(false);

            for capsule in session_a.take_pending_capsules() {
                if capsule.capsule_type() == 0x190B4D40 {
                    let result = session_b.process_capsule(&capsule);
                    assert!(result.is_ok(), "process_capsule failed: {:?}", result);
                }
            }
        }

        assert!(
            session_b.remote_limits().max_streams_uni >= concurrent_limit,
            "remote_limits should be >= initial: {} < {}",
            session_b.remote_limits().max_streams_uni,
            concurrent_limit
        );
        Ok(())
    })?;
    Ok(())
}

/// Property: advertised_max は単調増加する (データ)
#[test]
fn prop_data_advertised_max_monotonically_increases() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_SESSION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let initial_window = noprop::sample_u64_in(ctx, 1..10000);
        let num_chunks = noprop::sample_usize_in(ctx, 1..100);
        let chunk_size = noprop::sample_u64_in(ctx, 1..200);
        let mut session = Session::new(0);
        session.set_established();
        session.initialize_local_limits(FlowControlLimits {
            max_data: initial_window,
            ..FlowControlLimits::default()
        });

        let mut last_max = initial_window;

        for _ in 0..num_chunks {
            let size = chunk_size.min(initial_window);
            if session.check_received_data(size) {
                session.add_received_data(size);
                session.on_data_consumed(size);

                for capsule in session.take_pending_capsules() {
                    if let Capsule::MaxData { maximum } = capsule {
                        assert!(
                            maximum >= last_max,
                            "data advertised_max decreased: {} -> {}",
                            last_max,
                            maximum
                        );
                        last_max = maximum;
                    }
                }
            }
        }
        Ok(())
    })?;
    Ok(())
}

/// Property: WT_DATA_BLOCKED は同じ maximum に対して 1 回だけ送信される
#[test]
fn prop_data_blocked_dedup() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_SESSION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let limit = noprop::sample_u64_in(ctx, 0..100);
        let attempts = noprop::sample_usize_in(ctx, 2..20);
        let mut session = Session::new(0);
        session.set_established();
        session.remote_limits_mut().max_data = limit;

        if limit > 0 {
            assert!(session.try_send_data(limit));
        }

        for _ in 0..attempts {
            assert!(!session.try_send_data(1));
        }

        let capsules = session.take_pending_capsules();
        let blocked_count = capsules
            .iter()
            .filter(|c| matches!(c, Capsule::DataBlocked { .. }))
            .count();
        assert_eq!(
            blocked_count, 1,
            "DATA_BLOCKED should be sent exactly once per maximum, got {}",
            blocked_count
        );
        Ok(())
    })?;
    Ok(())
}
