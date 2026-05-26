//! Session の状態遷移・フロー制御・バッファリング・GOAWAY・終了処理プロパティ
//! (draft-ietf-webtrans-http3-15 Section 3, 4.5, 4.6, 4.7, 5.6, 6)

use proptest::prelude::*;
use shiguredo_http3::webtransport::{
    Capsule, FlowControlLimits, MAX_STREAMS_LIMIT, Session, SessionState, Stream,
};

prop_compose! {
    /// 有効なエラーコード (32-bit)
    fn valid_error_code()(code in any::<u32>()) -> u32 {
        code
    }
}

prop_compose! {
    /// 有効なエラーメッセージ (最大 1024 バイト)
    fn valid_error_message()(
        len in 0usize..=1024,
    )(
        msg in prop::collection::vec(0x20u8..0x7f, len)
    ) -> String {
        String::from_utf8(msg).expect("ASCII range bytes are always valid UTF-8")
    }
}

// =============================================================================
// 状態遷移 (draft-ietf-webtrans-http3-15 Section 3, 6)
// =============================================================================

proptest! {
    /// Property: 有効な状態遷移パス (Pending → Connecting → Established → Draining → Closed)
    #[test]
    fn prop_session_valid_transitions(_dummy in Just(())) {
        let mut session = Session::new(0);
        prop_assert_eq!(session.state(), SessionState::Pending);

        session.set_connecting();
        prop_assert_eq!(session.state(), SessionState::Connecting);

        session.set_established();
        prop_assert_eq!(session.state(), SessionState::Established);

        session.set_draining();
        prop_assert_eq!(session.state(), SessionState::Draining);

        session.close(None);
        prop_assert_eq!(session.state(), SessionState::Closed);
    }

    /// Property: Pending → Established の直接遷移も有効
    #[test]
    fn prop_session_direct_establish(_dummy in Just(())) {
        let mut session = Session::new(0);
        prop_assert_eq!(session.state(), SessionState::Pending);

        session.set_established();
        prop_assert_eq!(session.state(), SessionState::Established);

        session.close(None);
        prop_assert_eq!(session.state(), SessionState::Closed);
    }

    /// Property: 各状態でのストリーム作成可否
    #[test]
    fn prop_session_state_can_create_stream(_dummy in Just(())) {
        prop_assert!(!SessionState::Pending.can_create_stream());
        prop_assert!(!SessionState::Connecting.can_create_stream());
        prop_assert!(SessionState::Established.can_create_stream());
        prop_assert!(SessionState::Draining.can_create_stream());
        prop_assert!(!SessionState::Closed.can_create_stream());
    }

    /// Property: 各状態での送信可否
    #[test]
    fn prop_session_state_can_send(_dummy in Just(())) {
        prop_assert!(!SessionState::Pending.can_send());
        prop_assert!(!SessionState::Connecting.can_send());
        prop_assert!(SessionState::Established.can_send());
        prop_assert!(SessionState::Draining.can_send());
        prop_assert!(!SessionState::Closed.can_send());
    }
}

// =============================================================================
// フロー制御の単調増加制約 (draft-ietf-webtrans-http3-15 Section 5.6.2, 5.6.4)
// =============================================================================

proptest! {
    /// Property: MaxData が減少した場合にエラー
    #[test]
    fn prop_session_max_data_non_monotonic_error(
        initial in 100u64..10000,
        decrease in 1u64..100,
    ) {
        let mut session = Session::new(0);

        let result = session.process_capsule(&Capsule::MaxData { maximum: initial });
        prop_assert!(result.is_ok());
        prop_assert_eq!(session.remote_limits().max_data, initial);

        let smaller = initial.saturating_sub(decrease);
        let result = session.process_capsule(&Capsule::MaxData { maximum: smaller });
        prop_assert!(result.is_err());

        prop_assert_eq!(session.remote_limits().max_data, initial);
    }

    /// Property: MaxData が増加または同値なら成功
    #[test]
    fn prop_session_max_data_monotonic_ok(
        initial in 0u64..10000,
        increase in 0u64..10000,
    ) {
        let mut session = Session::new(0);

        let result = session.process_capsule(&Capsule::MaxData { maximum: initial });
        prop_assert!(result.is_ok());

        let larger = initial.saturating_add(increase);
        let result = session.process_capsule(&Capsule::MaxData { maximum: larger });
        prop_assert!(result.is_ok());
        prop_assert_eq!(session.remote_limits().max_data, larger);
    }

    /// Property: MaxStreams (双方向) が減少した場合にエラー
    #[test]
    fn prop_session_max_streams_bidi_non_monotonic_error(
        initial in 100u64..10000,
        decrease in 1u64..100,
    ) {
        let mut session = Session::new(0);

        let result = session.process_capsule(&Capsule::MaxStreams {
            bidirectional: true,
            maximum: initial,
        });
        prop_assert!(result.is_ok());

        let smaller = initial.saturating_sub(decrease);
        let result = session.process_capsule(&Capsule::MaxStreams {
            bidirectional: true,
            maximum: smaller,
        });
        prop_assert!(result.is_err());
    }

    /// Property: MaxStreams (単方向) が減少した場合にエラー
    #[test]
    fn prop_session_max_streams_uni_non_monotonic_error(
        initial in 100u64..10000,
        decrease in 1u64..100,
    ) {
        let mut session = Session::new(0);

        let result = session.process_capsule(&Capsule::MaxStreams {
            bidirectional: false,
            maximum: initial,
        });
        prop_assert!(result.is_ok());

        let smaller = initial.saturating_sub(decrease);
        let result = session.process_capsule(&Capsule::MaxStreams {
            bidirectional: false,
            maximum: smaller,
        });
        prop_assert!(result.is_err());
    }
}

// =============================================================================
// フロー制御リミット境界テスト
// =============================================================================

proptest! {
    /// Property: ストリーム作成可否の境界テスト (単方向)
    #[test]
    fn prop_session_can_create_stream_boundary_uni(
        limit in 1u64..100,
    ) {
        let mut session = Session::new(0);
        session.set_established();
        session.remote_limits_mut().max_streams_uni = limit;

        session.flow_state_mut().streams_uni_opened = limit - 1;
        prop_assert!(session.can_create_unidirectional_stream());

        session.flow_state_mut().streams_uni_opened = limit;
        prop_assert!(!session.can_create_unidirectional_stream());

        session.flow_state_mut().streams_uni_opened = limit + 1;
        prop_assert!(!session.can_create_unidirectional_stream());
    }

    /// Property: ストリーム作成可否の境界テスト (双方向)
    #[test]
    fn prop_session_can_create_stream_boundary_bidi(
        limit in 1u64..100,
    ) {
        let mut session = Session::new(0);
        session.set_established();
        session.remote_limits_mut().max_streams_bidi = limit;

        session.flow_state_mut().streams_bidi_opened = limit - 1;
        prop_assert!(session.can_create_bidirectional_stream());

        session.flow_state_mut().streams_bidi_opened = limit;
        prop_assert!(!session.can_create_bidirectional_stream());
    }

    /// Property: データ送信可否の境界テスト
    #[test]
    fn prop_session_can_send_data_boundary(
        limit in 100u64..10000,
        sent in 0u64..100,
    ) {
        let mut session = Session::new(0);
        session.set_established();
        session.remote_limits_mut().max_data = limit;
        session.flow_state_mut().data_sent = sent;

        let remaining = limit - sent;

        prop_assert!(session.can_send_data(remaining));
        prop_assert!(!session.can_send_data(remaining + 1));
    }

    /// Property: Established/Draining 以外ではストリーム作成不可
    #[test]
    fn prop_session_cannot_create_stream_in_wrong_state(
        limit in 1u64..100,
    ) {
        let mut session = Session::new(0);
        session.remote_limits_mut().max_streams_uni = limit;
        session.remote_limits_mut().max_streams_bidi = limit;

        // Pending 状態
        prop_assert!(!session.can_create_unidirectional_stream());
        prop_assert!(!session.can_create_bidirectional_stream());

        // Connecting 状態
        session.set_connecting();
        prop_assert!(!session.can_create_unidirectional_stream());
        prop_assert!(!session.can_create_bidirectional_stream());

        // Closed 状態
        session.close(None);
        prop_assert!(!session.can_create_unidirectional_stream());
        prop_assert!(!session.can_create_bidirectional_stream());
    }

    /// Property: Established/Draining 以外ではデータ送信不可
    #[test]
    fn prop_session_cannot_send_in_wrong_state(
        limit in 100u64..10000,
    ) {
        let mut session = Session::new(0);
        session.remote_limits_mut().max_data = limit;

        // Pending 状態
        prop_assert!(!session.can_send_data(1));

        // Connecting 状態
        session.set_connecting();
        prop_assert!(!session.can_send_data(1));

        // Closed 状態
        session.close(None);
        prop_assert!(!session.can_send_data(1));
    }
}

// =============================================================================
// バッファリング (draft-ietf-webtrans-http3-15 Section 4.6)
// =============================================================================

proptest! {
    /// Property: MAX_BUFFERED_STREAMS (100) までバッファリング成功
    #[test]
    fn prop_session_buffer_streams_up_to_limit(
        count in 1usize..=100,
    ) {
        let mut session = Session::new(0);

        for i in 0..count {
            prop_assert!(session.buffer_incoming_stream(i as u64 * 4, false));
        }

        let buffered = session.take_buffered_streams();
        prop_assert_eq!(buffered.len(), count);
    }

    /// Property: 101 個目のバッファリングは失敗
    #[test]
    fn prop_session_buffer_streams_over_limit(_dummy in proptest::strategy::Just(())) {
        let mut session = Session::new(0);

        for i in 0..100 {
            prop_assert!(session.buffer_incoming_stream(i as u64 * 4, false));
        }

        prop_assert!(!session.buffer_incoming_stream(99999, false));
    }

    /// Property: MAX_BUFFERED_DATAGRAMS (100) までバッファリング成功
    #[test]
    fn prop_session_buffer_datagrams_up_to_limit(
        count in 1usize..=100,
    ) {
        let mut session = Session::new(0);

        for i in 0..count {
            prop_assert!(session.buffer_datagram(vec![i as u8]));
        }

        let buffered = session.take_buffered_datagrams();
        prop_assert_eq!(buffered.len(), count);
    }

    /// Property: take 後のバッファは空になる
    #[test]
    fn prop_session_take_buffered_empties_buffer(
        stream_count in 1usize..=10,
        datagram_count in 1usize..=10,
    ) {
        let mut session = Session::new(0);

        for i in 0..stream_count {
            session.buffer_incoming_stream(i as u64 * 4, false);
        }
        for _ in 0..datagram_count {
            session.buffer_datagram(vec![0]);
        }

        let _streams = session.take_buffered_streams();
        let _datagrams = session.take_buffered_datagrams();

        prop_assert!(session.take_buffered_streams().is_empty());
        prop_assert!(session.take_buffered_datagrams().is_empty());
    }
}

// =============================================================================
// GOAWAY (draft-ietf-webtrans-http3-15 Section 4.7)
// =============================================================================

proptest! {
    /// Property: handle_goaway 後は goaway_received が true でドレイン状態
    #[test]
    fn prop_session_goaway_sets_draining(_dummy in proptest::strategy::Just(())) {
        let mut session = Session::new(0);
        session.set_established();

        prop_assert!(!session.is_goaway_received());
        prop_assert_eq!(session.state(), SessionState::Established);

        session.handle_goaway();

        prop_assert!(session.is_goaway_received());
        prop_assert_eq!(session.state(), SessionState::Draining);
    }

    /// Property: handle_goaway 後も既存ストリームは保持され can_send() == true
    #[test]
    fn prop_session_handle_goaway_preserves_streams(
        stream_count in 1usize..10,
    ) {
        let mut session = Session::new(0);
        session.set_established();
        session.remote_limits_mut().max_data = 100000;

        for i in 0..stream_count {
            session.add_stream(Stream::new(i as u64 * 4, 0, true));
        }

        session.handle_goaway();

        prop_assert_eq!(session.stream_count(), stream_count);
        for i in 0..stream_count {
            prop_assert!(session.get_stream(i as u64 * 4).is_some());
        }
        prop_assert!(session.state().can_send());
    }
}

// =============================================================================
// セッション終了処理 (draft-ietf-webtrans-http3-15 Section 6)
// =============================================================================

proptest! {
    /// Property: 任意の初期状態 (Established/Draining) で on_connect_stream_closed() →
    /// is_close_session_received() == true かつ is_closed() == true
    #[test]
    fn prop_session_on_connect_stream_closed_sets_flags(
        start_draining in any::<bool>(),
    ) {
        let mut session = Session::new(0);
        session.set_established();

        if start_draining {
            session.set_draining();
        }

        session.on_connect_stream_closed();

        prop_assert!(session.is_close_session_received());
        prop_assert!(session.is_closed());
    }

    /// Property: CloseSession Capsule 受信後に on_connect_stream_closed() →
    /// 最初のエラー情報が保持される
    #[test]
    fn prop_session_on_connect_stream_closed_preserves_first_error(
        error_code in valid_error_code(),
        message in valid_error_message(),
    ) {
        let mut session = Session::new(0);
        session.set_established();

        session.process_capsule(&Capsule::CloseSession {
            error_code,
            message: message.clone(),
        }).expect("CloseSession capsule should be accepted");

        let first_error = session.close_error().cloned();

        session.on_connect_stream_closed();

        prop_assert_eq!(session.close_error(), first_error.as_ref());
        prop_assert!(session.is_close_session_received());
    }

    /// Property: 任意のストリーム追加/削除後、stream_ids_to_reset() が
    /// 残存ストリーム ID と一致
    #[test]
    fn prop_session_stream_ids_to_reset_matches_streams(
        add_count in 1usize..20,
        remove_indices in prop::collection::vec(0usize..20, 0..10),
    ) {
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
        prop_assert_eq!(reset_ids, expected);
        prop_assert_eq!(reset_count, session.stream_count());
    }
}

// =============================================================================
// カプセルインターリーブ処理 (draft-ietf-webtrans-http3-15 Section 5.6, 6)
// =============================================================================

proptest! {
    /// Property: MaxData/MaxStreams/DataBlocked/StreamsBlocked をランダム順で処理しても
    /// 最終リミットが整合的
    #[test]
    fn prop_session_interleaved_capsule_processing(
        max_data_values in prop::collection::vec(0u64..10000, 1..5),
        max_streams_bidi_values in prop::collection::vec(0u64..1000, 1..5),
        max_streams_uni_values in prop::collection::vec(0u64..1000, 1..5),
    ) {
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
            prop_assert!(result.is_ok());
        }
        for &v in &sorted_bidi {
            let result = session.process_capsule(&Capsule::MaxStreams {
                bidirectional: true,
                maximum: v,
            });
            prop_assert!(result.is_ok());
        }
        for &v in &sorted_uni {
            let result = session.process_capsule(&Capsule::MaxStreams {
                bidirectional: false,
                maximum: v,
            });
            prop_assert!(result.is_ok());
        }

        let _ = session.process_capsule(&Capsule::DataBlocked { maximum: 999 });
        let _ = session.process_capsule(&Capsule::StreamsBlocked {
            bidirectional: true,
            maximum: 999,
        });

        if let Some(&max) = sorted_data.last() {
            prop_assert_eq!(session.remote_limits().max_data, max);
        }
        if let Some(&max) = sorted_bidi.last() {
            prop_assert_eq!(session.remote_limits().max_streams_bidi, max);
        }
        if let Some(&max) = sorted_uni.last() {
            prop_assert_eq!(session.remote_limits().max_streams_uni, max);
        }
    }

    /// Property: 単調増加列の後に減少値を送ると FlowControlError
    #[test]
    fn prop_session_flow_control_violation_after_increase(
        first in 100u64..10000,
        increase in 1u64..10000,
        decrease in 1u64..100,
    ) {
        let mut session = Session::new(0);
        let second = first.saturating_add(increase);

        session.process_capsule(&Capsule::MaxData { maximum: first }).expect("monotonic increase");
        session.process_capsule(&Capsule::MaxData { maximum: second }).expect("monotonic increase");

        let smaller = second.saturating_sub(decrease);
        if smaller < second {
            let result = session.process_capsule(&Capsule::MaxData { maximum: smaller });
            prop_assert!(result.is_err());
        }
    }
}

// =============================================================================
// ストリーム追加/削除の整合性
// =============================================================================

proptest! {
    /// Property: 追加/削除を交互に実行後、stream_count() と get_stream() が整合的
    #[test]
    fn prop_session_add_remove_stream_consistency(
        add_ids in prop::collection::vec(0u64..100, 1..20),
        remove_ids in prop::collection::vec(0u64..100, 0..10),
    ) {
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

        prop_assert_eq!(session.stream_count(), added_set.len());

        for &sid in &added_set {
            prop_assert!(session.get_stream(sid).is_some(),
                "Stream {} should exist", sid);
        }

        for &raw_id in &remove_ids {
            let sid = raw_id * 4;
            if !added_set.contains(&sid) {
                prop_assert!(session.get_stream(sid).is_none(),
                    "Stream {} should not exist", sid);
            }
        }
    }
}

// =============================================================================
// 動的ウィンドウ更新
// =============================================================================

proptest! {
    /// advertised_max は単調増加する (ストリーム)
    ///
    /// 任意の open/close シーケンスに対して、生成される WT_MAX_STREAMS の
    /// maximum は常に前回以上の値である。
    #[test]
    fn prop_advertised_max_monotonically_increases(
        concurrent_limit in 1u64..200,
        num_streams in 1usize..500,
    ) {
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
                if let Capsule::MaxStreams { bidirectional: false, maximum } = capsule {
                    prop_assert!(
                        maximum >= last_max,
                        "advertised_max decreased: {} -> {}", last_max, maximum
                    );
                    last_max = maximum;
                }
            }
        }
    }

    /// advertised_max は MAX_STREAMS_LIMIT を超えない (ストリーム)
    #[test]
    fn prop_advertised_max_within_limit(
        concurrent_limit in 1u64..=MAX_STREAMS_LIMIT,
        num_cycles in 1usize..100,
    ) {
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
                    prop_assert!(
                        maximum <= MAX_STREAMS_LIMIT,
                        "advertised_max exceeds limit: {}", maximum
                    );
                }
            }
        }
    }

    /// WT_STREAMS_BLOCKED は同じ maximum に対して 1 回だけ送信される
    #[test]
    fn prop_streams_blocked_dedup(
        limit in 0u64..50,
        attempts in 2usize..20,
    ) {
        let mut session = Session::new(0);
        session.set_established();
        session.remote_limits_mut().max_streams_uni = limit;

        for _ in 0..limit {
            prop_assert!(session.try_open_stream(false));
        }

        for _ in 0..attempts {
            prop_assert!(!session.try_open_stream(false));
        }

        let capsules = session.take_pending_capsules();
        let blocked_count = capsules
            .iter()
            .filter(|c| matches!(c, Capsule::StreamsBlocked { bidirectional: false, .. }))
            .count();
        prop_assert_eq!(
            blocked_count, 1,
            "STREAMS_BLOCKED should be sent exactly once per maximum, got {}", blocked_count
        );
    }

    /// セッション A → セッション B の動的ウィンドウ更新往復プロパティ
    ///
    /// セッション A で on_remote_stream_closed により生成された WT_MAX_STREAMS カプセルを
    /// セッション B の process_capsule に渡すと remote_limits が正しく更新される。
    #[test]
    fn prop_dynamic_max_streams_roundtrip(
        concurrent_limit in 1u64..200,
        num_streams in 1usize..200,
    ) {
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
                    prop_assert!(result.is_ok(), "process_capsule failed: {:?}", result);
                }
            }
        }

        prop_assert!(
            session_b.remote_limits().max_streams_uni >= concurrent_limit,
            "remote_limits should be >= initial: {} < {}",
            session_b.remote_limits().max_streams_uni,
            concurrent_limit
        );
    }

    /// advertised_max は単調増加する (データ)
    #[test]
    fn prop_data_advertised_max_monotonically_increases(
        initial_window in 1u64..10000,
        num_chunks in 1usize..100,
        chunk_size in 1u64..200,
    ) {
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
                        prop_assert!(
                            maximum >= last_max,
                            "data advertised_max decreased: {} -> {}", last_max, maximum
                        );
                        last_max = maximum;
                    }
                }
            }
        }
    }

    /// WT_DATA_BLOCKED は同じ maximum に対して 1 回だけ送信される
    #[test]
    fn prop_data_blocked_dedup(
        limit in 0u64..100,
        attempts in 2usize..20,
    ) {
        let mut session = Session::new(0);
        session.set_established();
        session.remote_limits_mut().max_data = limit;

        if limit > 0 {
            prop_assert!(session.try_send_data(limit));
        }

        for _ in 0..attempts {
            prop_assert!(!session.try_send_data(1));
        }

        let capsules = session.take_pending_capsules();
        let blocked_count = capsules
            .iter()
            .filter(|c| matches!(c, Capsule::DataBlocked { .. }))
            .count();
        prop_assert_eq!(
            blocked_count, 1,
            "DATA_BLOCKED should be sent exactly once per maximum, got {}", blocked_count
        );
    }
}
