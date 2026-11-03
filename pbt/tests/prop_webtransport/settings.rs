//! WebTransport Settings のプロパティ
//! (draft-ietf-webtrans-http3-15 Section 5.1, 9.2,
//!  draft-ietf-webtrans-http3-17 Section 5.1, 9.2)

use shiguredo_http3::webtransport::DraftVersion;
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
        let draft17_enabled = noprop::sample_u64_in(ctx, 0..100);
        let max_streams_uni = noprop::sample_u64_in(ctx, 0..1000);
        let max_streams_bidi = noprop::sample_u64_in(ctx, 0..1000);
        let max_data = noprop::sample_u64_in(ctx, 0..100000);
        let v = |x: u64| VarInt::new(x).expect("value in VarInt range");
        let settings = Settings::new()
            .wt_enabled(v(max_sessions))
            .wt_enabled_draft17(v(draft17_enabled))
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
        if draft17_enabled > 0 {
            assert!(entries.contains(&Setting::WtEnabledDraft17(v(draft17_enabled))));
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

        let expected_count = [
            max_sessions,
            draft17_enabled,
            max_streams_uni,
            max_streams_bidi,
            max_data,
        ]
        .iter()
        .filter(|&&v| v > 0)
        .count();
        assert_eq!(entries.len(), expected_count);
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// draft-17 の SETTINGS_WT_ENABLED コードポイント
// (draft-ietf-webtrans-http3-17 Section 3.1, 5.1, 7.1, 9.2)
// =============================================================================

/// Property: SETTINGS_WT_MAX_SESSIONS による draft-14 のフロー制御宣言条件は、
/// SETTINGS_WT_ENABLED (新旧どちらのコードポイントでも) を送る端点には適用しない
///
/// draft-15 以降の宣言条件は SETTINGS_WT_INITIAL_MAX_* のみで、
/// SETTINGS_WT_MAX_SESSIONS は廃止されている
/// (draft-ietf-webtrans-http3-17 Section 5.1)。
/// 新旧どちらのコードポイントを送るか、WT_MAX_SESSIONS が 1 か 2 以上かで
/// 結果が変わる境界をまたぐ入力を生成する。
#[test]
fn prop_settings_flow_control_draft17_ignores_max_sessions() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_SETTINGS_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        // 新旧の SETTINGS_WT_ENABLED は 0 / 1 / 2 の境界をまたぐ
        let draft17_enabled = noprop::sample_u64_in(ctx, 0..3);
        let old_enabled = noprop::sample_u64_in(ctx, 0..3);
        // WT_MAX_SESSIONS は draft-14 の宣言境界 (1 と 2) をまたぐ
        let max_sessions = noprop::sample_u64_in(ctx, 0..3);
        let v = |x: u64| VarInt::new(x).expect("value in VarInt range");
        let settings = Settings::new()
            .wt_enabled_draft17(v(draft17_enabled))
            .wt_enabled(v(old_enabled))
            .wt_max_sessions_draft14(v(max_sessions));

        // SETTINGS_WT_ENABLED を送らない端点にだけ draft-14 の宣言条件を適用する
        let sends_wt_enabled = draft17_enabled != 0 || old_enabled != 0;
        let expected = !sends_wt_enabled && max_sessions > 1;
        assert_eq!(
            settings.declares_flow_control(),
            expected,
            "新旧コードポイント ({draft17_enabled}/{old_enabled}) と \
             WT_MAX_SESSIONS {max_sessions} の組み合わせで宣言条件が一致すること"
        );

        // SETTINGS_WT_INITIAL_MAX_* が非ゼロなら、どの組み合わせでも宣言になる
        let initial_max_data = noprop::sample_u64_in(ctx, 1..100000);
        let settings = settings.wt_initial_max_data(v(initial_max_data));
        assert!(
            settings.declares_flow_control(),
            "SETTINGS_WT_INITIAL_MAX_DATA が非ゼロならフロー制御宣言になること"
        );
        Ok(())
    })?;
    Ok(())
}

/// Property: draft 検出は新コードポイント優先で、送られている ID の組み合わせから
/// 一意に決まる
///
/// draft-17 のコードポイントと旧 draft のコードポイントを併送した場合、
/// 最も新しい版である draft-17 を優先する
/// (draft-ietf-webtrans-http3-17 Section 7.1)。
/// 各コードポイントを 0 / 1 / 2 の境界で生成し、期待値を優先順位のオラクルとして
/// 組み立てる。draft-07 と draft-14 の ID が併送された場合は、Safari
/// (Network.framework) 互換のため draft-07 を優先する点もオラクルに含める。
#[test]
fn prop_settings_detect_draft17_priority() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_SETTINGS_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let draft17_enabled = noprop::sample_u64_in(ctx, 0..3);
        let old_enabled = noprop::sample_u64_in(ctx, 0..3);
        let max_sessions_draft14 = noprop::sample_u64_in(ctx, 0..3);
        let max_sessions_draft07 = noprop::sample_u64_in(ctx, 0..3);
        let draft02_enabled = noprop::sample_bool(ctx);
        let v = |x: u64| VarInt::new(x).expect("value in VarInt range");
        let settings = Settings::new()
            .wt_enabled_draft17(v(draft17_enabled))
            .wt_enabled(v(old_enabled))
            .wt_max_sessions_draft14(v(max_sessions_draft14))
            .webtransport_max_sessions_draft07(v(max_sessions_draft07))
            .enable_webtransport_draft02(draft02_enabled);

        // 新しい版から順に判定する (draft-15 は draft-15 / 16 の代表)。
        // draft-07 と draft-14 の ID 併送時は Safari 互換のため draft-07 を優先する
        // (src/webtransport/settings.rs の detect_draft_pattern を参照)。
        let expected = if draft17_enabled > 0 {
            Some(DraftVersion::Draft17)
        } else if old_enabled > 0 {
            Some(DraftVersion::Draft15)
        } else if max_sessions_draft07 > 0 {
            Some(DraftVersion::Draft07)
        } else if max_sessions_draft14 > 0 {
            Some(DraftVersion::Draft14)
        } else if draft02_enabled {
            Some(DraftVersion::Draft02)
        } else {
            None
        };
        assert_eq!(
            settings.detect_draft_pattern(),
            expected,
            "送信済みコードポイントから最も新しい版を検出すること"
        );
        assert_eq!(
            settings.is_enabled(),
            expected.is_some(),
            "WebTransport の有効判定が版検出と一致すること"
        );
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// 接続単位のドラフト版選択
// (draft-ietf-webtrans-http3-17 Section 3.1, 7.1)
// =============================================================================

/// SETTINGS の (ID, 値) の並びを制御ストリームのバイト列に変換する
///
/// エントリの順序と分割位置は版選択に影響しないため、PBT では順序を
/// シャッフルしたうえで任意の位置で分割して流し込む。
fn encode_control_stream(entries: &[(VarInt, VarInt)]) -> Vec<u8> {
    let mut payload = Vec::new();
    for (id, value) in entries {
        shiguredo_http3::varint::encode_into_vec(&mut payload, *id);
        shiguredo_http3::varint::encode_into_vec(&mut payload, *value);
    }

    let mut control = Vec::new();
    // 制御ストリームタイプ (0x00)
    shiguredo_http3::varint::encode_into_vec(&mut control, VarInt::from_static(0x00));
    // SETTINGS フレームタイプ (0x04) とペイロード長
    shiguredo_http3::varint::encode_into_vec(&mut control, VarInt::from_static(0x04));
    shiguredo_http3::varint::encode_into_vec(
        &mut control,
        VarInt::new(payload.len() as u64)
            .expect("テスト用の SETTINGS ペイロード長が VarInt 範囲に収まること"),
    );
    control.extend_from_slice(&payload);
    control
}

/// 広告するドラフト版の組み合わせを生成する
///
/// draft-02 / 07 / 14 / 15 / 17 のそれぞれについて広告するかどうかを選ぶ。
fn sample_draft_set(ctx: &mut noprop::TestCaseContext) -> Vec<DraftVersion> {
    let mut versions = Vec::new();
    for version in [
        DraftVersion::Draft02,
        DraftVersion::Draft07,
        DraftVersion::Draft14,
        DraftVersion::Draft15,
        DraftVersion::Draft17,
    ] {
        if noprop::sample_bool(ctx) {
            versions.push(version);
        }
    }
    versions
}

/// サーバー側の版選択の期待値を求める
///
/// - クライアントが本文版のコードポイントを広告していれば本文版
/// - そうでなければ共通する旧 draft のうち最も優先順位の高い版
/// - どちらもなければ、本文版に対応していれば本文版、対応していなければなし
fn expected_server_selection(
    local: &[DraftVersion],
    peer: &[DraftVersion],
) -> Option<DraftVersion> {
    let common = |version: DraftVersion| local.contains(&version) && peer.contains(&version);
    if common(DraftVersion::Draft17) {
        return Some(DraftVersion::Draft17);
    }
    // draft-15 と draft-16 は同一コードポイント、draft-07 は Safari 互換で
    // draft-14 より優先する
    for version in [
        DraftVersion::Draft15,
        DraftVersion::Draft07,
        DraftVersion::Draft14,
        DraftVersion::Draft02,
    ] {
        if common(version) {
            return Some(version);
        }
    }
    if local.contains(&DraftVersion::Draft17) {
        return Some(DraftVersion::Draft17);
    }
    None
}

/// Property: SETTINGS のエントリ順序と分割位置を変えても接続単位の版選択は変わらない
///
/// 実際の SETTINGS バイト列を組み立て、エントリをシャッフルしたうえで
/// ランダムな位置で分割して `ServerConnection` に流し込む。選択結果が
/// 入力の表現に依存しないことを検証する
/// (draft-ietf-webtrans-http3-17 Section 3.1, 7.1)。
#[test]
fn prop_settings_order_and_split_do_not_change_negotiation() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_SETTINGS_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(128, |ctx| {
        let local_drafts = sample_draft_set(ctx);
        let peer_drafts = sample_draft_set(ctx);
        let params = shiguredo_http3::webtransport::connect::ServerSettingsParams::default();

        let local_wt = Settings::for_draft_versions(&local_drafts, params.max_sessions);
        let mut server = shiguredo_http3::ServerConnection::new(
            shiguredo_http3::Settings::new().enable_webtransport_server(local_wt),
        );
        server
            .set_control_stream_id(3)
            .expect("テスト用の制御ストリーム設定に成功すること");

        let peer_wt = Settings::for_draft_versions(&peer_drafts, params.max_sessions);
        let mut entries: Vec<(VarInt, VarInt)> =
            peer_wt.iter().map(|setting| setting.as_wire()).collect();
        // Fisher-Yates でエントリ順をシャッフルする
        let len = entries.len();
        for i in (1..len).rev() {
            let j = noprop::sample_usize_in(ctx, 0..=i);
            entries.swap(i, j);
        }

        let control = encode_control_stream(&entries);

        // 制御ストリームをランダムな位置で分割して流し込む
        let mut pos = 0;
        while pos < control.len() {
            let remaining = control.len() - pos;
            let chunk_len = noprop::sample_usize_in(ctx, 1..=remaining);
            server
                .feed_stream(2, &control[pos..pos + chunk_len], false)
                .expect("SETTINGS の分割受信に成功すること");
            pos += chunk_len;
        }

        assert_eq!(
            server.negotiated_wt_draft_version(),
            expected_server_selection(&local_drafts, &peer_drafts),
            "ローカル {local_drafts:?} とピア {peer_drafts:?} の選択結果が \
             エントリ順序・分割位置に依存しないこと"
        );
        Ok(())
    })?;
    Ok(())
}

/// Property: 複数版を広告する SETTINGS の広告集合は要求した版の集合と一致する
///
/// 入力の順序と重複を変えても、広告集合は「要求した版を優先順位の高い順に
/// 並べたもの」になる (draft-ietf-webtrans-http3-17 Section 7.1)。
#[test]
fn prop_for_draft_versions_advertisement_set() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WEBTRANSPORT_SETTINGS_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let mut requested = sample_draft_set(ctx);
        // 入力の順序をシャッフルする
        let len = requested.len();
        for i in (1..len).rev() {
            let j = noprop::sample_usize_in(ctx, 0..=i);
            requested.swap(i, j);
        }
        // 一部の版を重複させる
        if !requested.is_empty() && noprop::sample_bool(ctx) {
            let index = noprop::sample_usize_in(ctx, 0..requested.len());
            requested.push(requested[index]);
        }

        let params = shiguredo_http3::webtransport::connect::ServerSettingsParams::default();
        let settings = Settings::for_draft_versions(&requested, params.max_sessions);

        let mut expected = Vec::new();
        for version in [
            DraftVersion::Draft17,
            DraftVersion::Draft15,
            DraftVersion::Draft07,
            DraftVersion::Draft14,
            DraftVersion::Draft02,
        ] {
            if requested.contains(&version) {
                expected.push(version);
            }
        }
        assert_eq!(
            settings.advertised_draft_versions(),
            expected,
            "要求した版 {requested:?} の広告集合が優先順位順に一致すること"
        );
        Ok(())
    })?;
    Ok(())
}
