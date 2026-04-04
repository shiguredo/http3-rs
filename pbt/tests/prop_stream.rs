//! Property-Based Testing for HTTP/3 ストリーム (RFC 9114 Section 6)

use proptest::prelude::*;
use shiguredo_http3::stream::{StreamKind, StreamState, UniStreamType};

// =============================================================================
// (a) StreamState 状態遷移の対称性
// =============================================================================

proptest! {
    /// Property: close_local → close_remote と close_remote → close_local は共に Closed に到達する
    #[test]
    fn prop_stream_state_close_symmetry(_dummy in Just(())) {
        // Open → LocalClosed → Closed
        let mut state_lr = StreamState::Open;
        state_lr.close_local();
        prop_assert_eq!(state_lr, StreamState::LocalClosed);
        state_lr.close_remote();
        prop_assert_eq!(state_lr, StreamState::Closed, "close_local → close_remote で Closed にならない");

        // Open → RemoteClosed → Closed
        let mut state_rl = StreamState::Open;
        state_rl.close_remote();
        prop_assert_eq!(state_rl, StreamState::RemoteClosed);
        state_rl.close_local();
        prop_assert_eq!(state_rl, StreamState::Closed, "close_remote → close_local で Closed にならない");
    }
}

// =============================================================================
// (b) Reset は全ての状態から遷移可能で、Reset 後は send/receive 不可
// =============================================================================

proptest! {
    /// Property: 任意の状態遷移列の後に reset() を呼ぶと、can_send()==false かつ can_receive()==false
    #[test]
    fn prop_reset_disables_send_and_receive(
        ops in prop::collection::vec(
            prop::sample::select(vec![0u8, 1, 2]),
            0..=10,
        )
    ) {
        let mut state = StreamState::Open;

        // 任意の操作列を適用
        for op in &ops {
            match op {
                0 => state.close_local(),
                1 => state.close_remote(),
                _ => state.reset(),
            }
        }

        // reset() を適用
        state.reset();

        prop_assert_eq!(state, StreamState::Reset);
        prop_assert!(!state.can_send(), "Reset 後に can_send() が true");
        prop_assert!(!state.can_receive(), "Reset 後に can_receive() が true");
        prop_assert!(state.is_reset(), "Reset 後に is_reset() が false");
    }
}

// =============================================================================
// (c) StreamKind のストリーム ID 分類が QUIC 仕様に準拠
// =============================================================================

proptest! {
    /// Property: 任意の stream_id に対して下位 2 ビットに基づく StreamKind 分類が正しい
    #[test]
    fn prop_stream_kind_classification(stream_id in 0u64..1_000_000) {
        let kind = StreamKind::from_stream_id(stream_id);
        let low_bits = stream_id & 0x03;

        // 下位 2 ビットに基づく分類 (RFC 9000 Table 1)
        match low_bits {
            0x00 => {
                prop_assert_eq!(kind, StreamKind::ClientBidi);
                prop_assert!(kind.is_bidirectional());
                prop_assert!(kind.is_client_initiated());
                prop_assert!(!kind.is_unidirectional());
                prop_assert!(!kind.is_server_initiated());
            }
            0x01 => {
                prop_assert_eq!(kind, StreamKind::ServerBidi);
                prop_assert!(kind.is_bidirectional());
                prop_assert!(kind.is_server_initiated());
            }
            0x02 => {
                prop_assert_eq!(kind, StreamKind::ClientUni);
                prop_assert!(kind.is_unidirectional());
                prop_assert!(kind.is_client_initiated());
            }
            0x03 => {
                prop_assert_eq!(kind, StreamKind::ServerUni);
                prop_assert!(kind.is_unidirectional());
                prop_assert!(kind.is_server_initiated());
            }
            _ => unreachable!(),
        }
    }

    /// Property: bidirectional と unidirectional は排他的
    #[test]
    fn prop_stream_kind_bidi_uni_exclusive(stream_id in 0u64..1_000_000) {
        let kind = StreamKind::from_stream_id(stream_id);
        prop_assert_ne!(
            kind.is_bidirectional(),
            kind.is_unidirectional(),
            "bidirectional と unidirectional が同じ値"
        );
    }

    /// Property: client_initiated と server_initiated は排他的
    #[test]
    fn prop_stream_kind_initiator_exclusive(stream_id in 0u64..1_000_000) {
        let kind = StreamKind::from_stream_id(stream_id);
        prop_assert_ne!(
            kind.is_client_initiated(),
            kind.is_server_initiated(),
            "client_initiated と server_initiated が同じ値"
        );
    }
}

// =============================================================================
// (d) UniStreamType::is_reserved の GREASE パターン
// =============================================================================

proptest! {
    /// Property: 0x1f * N + 0x21 形式の値は is_reserved() == true
    #[test]
    fn prop_reserved_stream_type_formula(n in 0u64..10000) {
        let t = 0x1f * n + 0x21;
        prop_assert!(
            UniStreamType::is_reserved(t),
            "0x1f * {} + 0x21 = {:#x} が is_reserved() == false", n, t
        );
    }

    /// Property: 0x1f * N + 0x21 形式でない値は is_reserved() == false (0x21 未満 + その他)
    #[test]
    fn prop_non_reserved_stream_type(t in 0u64..0x21) {
        prop_assert!(
            !UniStreamType::is_reserved(t),
            "{:#x} が is_reserved() == true (0x21 未満なのに予約済み)", t
        );
    }

    /// Property: 0x21 以上で 0x1f * N + 0x21 形式でない値は is_reserved() == false
    #[test]
    fn prop_non_grease_above_threshold(n in 0u64..10000, offset in 1u64..0x1f) {
        let t = 0x1f * n + 0x21 + offset;
        // offset が 0x1f の倍数でない限り is_reserved() == false のはず
        // ただし offset + 0x21 が次の GREASE 値に一致する場合がある。
        // 正確には (t - 0x21) % 0x1f != 0 なら false
        if (t - 0x21) % 0x1f != 0 {
            prop_assert!(
                !UniStreamType::is_reserved(t),
                "{:#x} が is_reserved() == true (GREASE パターンに合致しないのに)", t
            );
        }
    }
}

// =============================================================================
// (e) StreamState の can_send / can_receive の不変条件
// =============================================================================

proptest! {
    /// Property: Open 状態では can_send==true, can_receive==true
    #[test]
    fn prop_open_state_can_send_receive(_dummy in Just(())) {
        let state = StreamState::Open;
        prop_assert!(state.can_send());
        prop_assert!(state.can_receive());
    }

    /// Property: LocalClosed では can_send==false, can_receive==true
    #[test]
    fn prop_local_closed_no_send(_dummy in Just(())) {
        let mut state = StreamState::Open;
        state.close_local();
        prop_assert!(!state.can_send());
        prop_assert!(state.can_receive());
    }

    /// Property: RemoteClosed では can_send==true, can_receive==false
    #[test]
    fn prop_remote_closed_no_receive(_dummy in Just(())) {
        let mut state = StreamState::Open;
        state.close_remote();
        prop_assert!(state.can_send());
        prop_assert!(!state.can_receive());
    }

    /// Property: Closed では can_send==false, can_receive==false
    #[test]
    fn prop_closed_no_send_no_receive(_dummy in Just(())) {
        let mut state = StreamState::Open;
        state.close_local();
        state.close_remote();
        prop_assert!(!state.can_send());
        prop_assert!(!state.can_receive());
    }
}
