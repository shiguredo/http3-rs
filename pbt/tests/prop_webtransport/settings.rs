//! WebTransport Settings のプロパティ
//! (draft-ietf-webtrans-http3-15 Section 5.1, 9.2)

use shiguredo_http3::webtransport::Settings;
use shiguredo_http3::{Setting, VarInt};

// =============================================================================
// フロー制御有効化判定 (draft-ietf-webtrans-http3-15 Section 5.1)
// =============================================================================

/// Property: wt_enabled のみではフロー制御無効 (draft-15)
#[test]
fn prop_settings_flow_control_wt_enabled_only() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_SETTINGS_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let wt_enabled = noprop::sample_u64_in(ctx, 1..100);
        let settings =
            Settings::new().wt_enabled(VarInt::new(wt_enabled).expect("value in VarInt range"));
        assert!(!settings.declares_flow_control());
        Ok(())
    })?;
    Ok(())
}

/// Property: INITIAL_MAX_* が 0 以外ならフロー制御有効 (draft-15)
#[test]
fn prop_settings_flow_control_enabled_by_initial_values() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_SETTINGS_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let max_streams_uni = noprop::sample_u64_in(ctx, 1..100);
        let max_streams_bidi = noprop::sample_u64_in(ctx, 1..100);
        let max_data = noprop::sample_u64_in(ctx, 1..100000);
        let v = |x: u64| VarInt::new(x).expect("value in VarInt range");
        let settings = Settings::new()
            .wt_enabled(VarInt::from_static(1))
            .wt_initial_max_streams_uni(v(max_streams_uni));
        assert!(settings.declares_flow_control());

        let settings = Settings::new()
            .wt_enabled(VarInt::from_static(1))
            .wt_initial_max_streams_bidi(v(max_streams_bidi));
        assert!(settings.declares_flow_control());

        let settings = Settings::new()
            .wt_enabled(VarInt::from_static(1))
            .wt_initial_max_data(v(max_data));
        assert!(settings.declares_flow_control());
        Ok(())
    })?;
    Ok(())
}

/// Property: builder で設定した値と iter() 出力が一致。0 の値は含まれない
#[test]
fn prop_settings_iter_matches_builder() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_SETTINGS_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let max_sessions = noprop::sample_u64_in(ctx, 0..100);
        let max_streams_uni = noprop::sample_u64_in(ctx, 0..1000);
        let max_streams_bidi = noprop::sample_u64_in(ctx, 0..1000);
        let max_data = noprop::sample_u64_in(ctx, 0..100000);
        let v = |x: u64| VarInt::new(x).expect("value in VarInt range");
        let settings = Settings::new()
            .wt_enabled(v(max_sessions))
            .wt_initial_max_streams_uni(v(max_streams_uni))
            .wt_initial_max_streams_bidi(v(max_streams_bidi))
            .wt_initial_max_data(v(max_data));

        let entries: Vec<Setting> = settings.iter().collect();

        for e in &entries {
            let (_, val) = e.as_wire();
            assert!(val.get() > 0);
        }

        if max_sessions > 0 {
            assert!(entries.contains(&Setting::WtEnabled(v(max_sessions))));
        }
        if max_streams_uni > 0 {
            assert!(entries.contains(&Setting::WtInitialMaxStreamsUni(v(max_streams_uni))));
        }
        if max_streams_bidi > 0 {
            assert!(entries.contains(&Setting::WtInitialMaxStreamsBidi(v(max_streams_bidi))));
        }
        if max_data > 0 {
            assert!(entries.contains(&Setting::WtInitialMaxData(v(max_data))));
        }

        let expected_count = [max_sessions, max_streams_uni, max_streams_bidi, max_data]
            .iter()
            .filter(|&&v| v > 0)
            .count();
        assert_eq!(entries.len(), expected_count);
        Ok(())
    })?;
    Ok(())
}
