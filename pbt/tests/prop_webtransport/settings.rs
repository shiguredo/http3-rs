//! WebTransport Settings のプロパティ
//! (draft-ietf-webtrans-http3-15 Section 5.1, 9.2)

use proptest::prelude::*;
use shiguredo_http3::webtransport::Settings;
use shiguredo_http3::{Setting, VarInt};

// =============================================================================
// フロー制御有効化判定 (draft-ietf-webtrans-http3-15 Section 5.1)
// =============================================================================

proptest! {
    /// Property: wt_enabled のみではフロー制御無効 (draft-15)
    #[test]
    fn prop_settings_flow_control_wt_enabled_only(wt_enabled in 1u64..100) {
        let settings = Settings::new().wt_enabled(VarInt::new(wt_enabled).expect("value in VarInt range"));
        prop_assert!(!settings.declares_flow_control());
    }

    /// Property: INITIAL_MAX_* が 0 以外ならフロー制御有効 (draft-15)
    #[test]
    fn prop_settings_flow_control_enabled_by_initial_values(
        max_streams_uni in 1u64..100,
        max_streams_bidi in 1u64..100,
        max_data in 1u64..100000,
    ) {
        let v = |x: u64| VarInt::new(x).expect("value in VarInt range");
        let settings = Settings::new()
            .wt_enabled(VarInt::from_static(1))
            .wt_initial_max_streams_uni(v(max_streams_uni));
        prop_assert!(settings.declares_flow_control());

        let settings = Settings::new()
            .wt_enabled(VarInt::from_static(1))
            .wt_initial_max_streams_bidi(v(max_streams_bidi));
        prop_assert!(settings.declares_flow_control());

        let settings = Settings::new()
            .wt_enabled(VarInt::from_static(1))
            .wt_initial_max_data(v(max_data));
        prop_assert!(settings.declares_flow_control());
    }

    /// Property: 全て 0 または wt_enabled = 0 なら無効
    #[test]
    fn prop_settings_disabled_when_zero(_dummy in Just(())) {
        let settings = Settings::new();
        prop_assert!(!settings.is_enabled());
        prop_assert!(!settings.declares_flow_control());
    }
}

// =============================================================================
// Settings iter (draft-ietf-webtrans-http3-15 Section 9.2)
// =============================================================================

proptest! {
    /// Property: builder で設定した値と iter() 出力が一致。0 の値は含まれない
    #[test]
    fn prop_settings_iter_matches_builder(
        max_sessions in 0u64..100,
        max_streams_uni in 0u64..1000,
        max_streams_bidi in 0u64..1000,
        max_data in 0u64..100000,
    ) {
        let v = |x: u64| VarInt::new(x).expect("value in VarInt range");
        let settings = Settings::new()
            .wt_enabled(v(max_sessions))
            .wt_initial_max_streams_uni(v(max_streams_uni))
            .wt_initial_max_streams_bidi(v(max_streams_bidi))
            .wt_initial_max_data(v(max_data));

        let entries: Vec<Setting> = settings.iter().collect();

        for e in &entries {
            let (_, val) = e.as_wire();
            prop_assert!(val.get() > 0);
        }

        if max_sessions > 0 {
            prop_assert!(entries.contains(&Setting::WtEnabled(v(max_sessions))));
        }
        if max_streams_uni > 0 {
            prop_assert!(entries.contains(&Setting::WtInitialMaxStreamsUni(v(max_streams_uni))));
        }
        if max_streams_bidi > 0 {
            prop_assert!(entries.contains(&Setting::WtInitialMaxStreamsBidi(v(max_streams_bidi))));
        }
        if max_data > 0 {
            prop_assert!(entries.contains(&Setting::WtInitialMaxData(v(max_data))));
        }

        let expected_count = [max_sessions, max_streams_uni, max_streams_bidi, max_data]
            .iter()
            .filter(|&&v| v > 0)
            .count();
        prop_assert_eq!(entries.len(), expected_count);
    }
}
