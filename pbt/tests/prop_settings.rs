//! Property-Based Testing for HTTP/3 Settings (RFC 9114 §7.2.4)
//!
//! 値域や bool 値検査は `Setting::from_wire` 側で構築時に検査されるため、
//! 不正値の単体テストは `src/settings.rs` の `#[cfg(test)]` に移してある。
//! 本ファイルは noprop で構築した正常値が SettingsPayload とのラウンドトリップ /
//! `Setting::from_wire` ↔ `Setting::as_wire` の対称性を満たすことを検査する。

use pbt::strategies::{sample_varint_raw_in, valid_varint};
use shiguredo_http3::frame::SettingsPayload;
use shiguredo_http3::{Setting, SettingError, Settings, VarInt, webtransport};

fn vi(value: u64) -> VarInt {
    VarInt::new(value).expect("test must succeed")
}

// `arbitrary_settings` / `arbitrary_wt_settings` で生成する範囲は中規模 (`1 << 30`)
// に絞り、サンプルノイズを抑える。`prop_setting_wire_roundtrip` 等で VarInt
// 全域を叩く場合は別関数 (`arbitrary_varint_full`) を用いる。
const VARINT_TEST_MAX: u64 = 1 << 30;

/// `0..VARINT_TEST_MAX` から VarInt を生成する
fn arbitrary_varint(ctx: &mut noprop::TestCaseContext) -> VarInt {
    vi(sample_varint_raw_in(ctx, 0..=VARINT_TEST_MAX - 1))
}

/// VarInt 全域 (`0..=VarInt::MAX`) を生成する
fn arbitrary_varint_full(ctx: &mut noprop::TestCaseContext) -> VarInt {
    valid_varint(ctx)
}

/// `Setting::from_wire` で受理される ID のみを生成する
///
/// HTTP/2 専用 ID (0x02..=0x05) と予約 ID (0x00) を生成段階で除外する。
/// 受理率はほぼ 1 のため rejection の試行回数は少なくて済む。
fn settable_id(ctx: &mut noprop::TestCaseContext) -> VarInt {
    noprop::sample_with_rejection(ctx, 64, |ctx| {
        let v = arbitrary_varint_full(ctx);
        let raw = v.get();
        (raw != 0 && !matches!(raw, 0x02..=0x05)).then_some(v)
    })
}

/// Option を 50% の確率で生成するヘルパー
fn sample_option<T>(
    ctx: &mut noprop::TestCaseContext,
    sample: impl Fn(&mut noprop::TestCaseContext) -> T,
) -> Option<T> {
    if noprop::sample_bool(ctx) {
        Some(sample(ctx))
    } else {
        None
    }
}

/// 任意の webtransport::Settings を生成
fn arbitrary_wt_settings(ctx: &mut noprop::TestCaseContext) -> webtransport::Settings {
    let wt_enabled = arbitrary_varint(ctx);
    let wt_initial_max_streams_uni = arbitrary_varint(ctx);
    let wt_initial_max_streams_bidi = arbitrary_varint(ctx);
    let wt_initial_max_data = arbitrary_varint(ctx);
    let enable_webtransport_draft02 = sample_option(ctx, noprop::sample_bool);
    let webtransport_max_sessions_draft07 = sample_option(ctx, arbitrary_varint);

    let mut wt = webtransport::Settings::new()
        .wt_enabled(wt_enabled)
        .wt_initial_max_streams_uni(wt_initial_max_streams_uni)
        .wt_initial_max_streams_bidi(wt_initial_max_streams_bidi)
        .wt_initial_max_data(wt_initial_max_data);
    if let Some(v) = enable_webtransport_draft02 {
        wt = wt.enable_webtransport_draft02(v);
    }
    if let Some(v) = webtransport_max_sessions_draft07 {
        wt = wt.webtransport_max_sessions_draft07(v);
    }
    wt
}

/// 任意の Settings を生成 (各フィールドが Some または None)
fn arbitrary_settings(ctx: &mut noprop::TestCaseContext) -> Settings {
    let qpack_max_table_capacity = sample_option(ctx, arbitrary_varint);
    let max_field_section_size = sample_option(ctx, arbitrary_varint);
    let qpack_blocked_streams = sample_option(ctx, arbitrary_varint);
    let enable_connect_protocol = sample_option(ctx, noprop::sample_bool);
    let h3_datagram = sample_option(ctx, noprop::sample_bool);
    let wt_settings = sample_option(ctx, arbitrary_wt_settings);

    let mut s = Settings::new();
    if let Some(v) = qpack_max_table_capacity {
        s = s.qpack_max_table_capacity(v);
    }
    if let Some(v) = max_field_section_size {
        s = s.max_field_section_size(v);
    }
    if let Some(v) = qpack_blocked_streams {
        s = s.qpack_blocked_streams(v);
    }
    if let Some(v) = enable_connect_protocol {
        s = s.enable_connect_protocol(v);
    }
    if let Some(v) = h3_datagram {
        s = s.h3_datagram(v);
    }
    if let Some(wt) = wt_settings {
        s = s.enable_webtransport_server(wt);
    }
    s
}

/// Property: Settings を SettingsPayload に変換して from_payload で復元すると元と一致する
#[test]
fn prop_settings_roundtrip() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_SETTINGS_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let settings = arbitrary_settings(ctx);
        let payload = SettingsPayload::from_settings(&settings);
        let restored = Settings::from_payload(&payload);

        assert_eq!(
            settings.qpack_max_table_capacity,
            restored.qpack_max_table_capacity
        );
        assert_eq!(
            settings.max_field_section_size,
            restored.max_field_section_size
        );
        assert_eq!(
            settings.qpack_blocked_streams,
            restored.qpack_blocked_streams
        );
        assert_eq!(
            settings.enable_connect_protocol,
            restored.enable_connect_protocol
        );
        assert_eq!(settings.h3_datagram, restored.h3_datagram);

        // enable_webtransport_server() で wt_settings を設定した場合、
        // enable_connect_protocol と h3_datagram も自動設定されるため、
        // from_payload 側では wt_settings の有無が元と一致することを確認する。
        // ただし iter() は 0 の値を出力しないため、全フィールドが 0 の wt_settings は
        // from_payload で None になる。
        match (&settings.wt_settings, &restored.wt_settings) {
            (Some(orig), Some(rest)) => {
                assert_eq!(orig, rest, "wt_settings が不一致");
            }
            (Some(orig), None) => {
                assert!(
                    orig.iter().count() == 0,
                    "元の wt_settings にエントリがあるのに復元が None"
                );
            }
            (None, None) => {}
            (None, Some(_)) => {
                panic!("元が None なのに復元に wt_settings がある");
            }
        }
        Ok(())
    })?;
    Ok(())
}

/// Property: Settings::len() は H3 + WT のエントリ合計数と一致する
#[test]
fn prop_iter_count_equals_len() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_SETTINGS_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let settings = arbitrary_settings(ctx);
        let h3_count = settings.iter().count();
        let wt_count = settings
            .wt_settings
            .as_ref()
            .map(|wt| wt.iter().count())
            .unwrap_or(0);
        let total = h3_count + wt_count;
        assert_eq!(total, settings.len());
        Ok(())
    })?;
    Ok(())
}

/// Property: enable_webtransport_server(wt) 後は is_webtransport_enabled() == true かつ
///           enable_connect_protocol と h3_datagram が Some(true)
#[test]
fn prop_enable_webtransport_sets_required_fields() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_SETTINGS_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        // wt_enabled が 0 だと is_enabled() が false になるためスキップ
        let wt_settings = noprop::sample_with_rejection(ctx, 64, |ctx| {
            let wt = arbitrary_wt_settings(ctx);
            wt.is_enabled().then_some(wt)
        });
        let settings = Settings::new().enable_webtransport_server(wt_settings);
        assert!(settings.is_webtransport_enabled());
        assert_eq!(settings.enable_connect_protocol, Some(true));
        assert_eq!(settings.h3_datagram, Some(true));
        Ok(())
    })?;
    Ok(())
}

/// Property: Settings::webtransport_draft_pattern() は
/// wt_settings.as_ref().and_then(|wt| wt.detect_draft_pattern()) と一致する
#[test]
fn prop_webtransport_draft_pattern_consistent() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_SETTINGS_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let settings = arbitrary_settings(ctx);
        let expected = settings
            .wt_settings
            .as_ref()
            .and_then(|wt| wt.detect_draft_pattern());
        let actual = settings.webtransport_draft_pattern();
        assert_eq!(actual, expected);
        Ok(())
    })?;
    Ok(())
}

/// Property: `Setting::from_wire` → `as_wire` → `from_wire` のラウンドトリップ
///
/// VarInt 全域 (`encoded_settings_payload_len` の合算境界含む) を叩く。
/// HTTP/2 専用 / 予約 ID は生成段階で除外する。
#[test]
fn prop_setting_wire_roundtrip() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_SETTINGS_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let id = settable_id(ctx);
        let mut value = arbitrary_varint_full(ctx);

        // bool 値 ID の場合は値域を 0/1 に正規化する
        let id_raw = id.get();
        let bool_ids = [0x08u64, 0x33, 0x2b603742];
        if bool_ids.contains(&id_raw) {
            value = vi(value.get() & 1);
        }
        let setting = Setting::from_wire(id, value).expect("test must succeed");
        let (id2, value2) = setting.as_wire();
        assert_eq!(id, id2);
        assert_eq!(value, value2);
        let restored = Setting::from_wire(id2, value2).expect("test must succeed");
        assert_eq!(setting, restored);
        Ok(())
    })?;
    Ok(())
}

/// 任意の `Setting` variant を 1 つ生成する
///
/// HTTP/2 専用 / 予約 ID は `Setting::from_wire` で構築不可のため除外。
/// 重複検出 PBT で variant 全体に検査が及ぶことを保証するために定義する。
fn arbitrary_setting(ctx: &mut noprop::TestCaseContext) -> Setting {
    match noprop::sample_weighted_index(ctx, &[1u32; 12]) {
        0 => Setting::QpackMaxTableCapacity(arbitrary_varint(ctx)),
        1 => Setting::MaxFieldSectionSize(arbitrary_varint(ctx)),
        2 => Setting::QpackBlockedStreams(arbitrary_varint(ctx)),
        3 => Setting::EnableConnectProtocol(noprop::sample_bool(ctx)),
        4 => Setting::H3Datagram(noprop::sample_bool(ctx)),
        5 => Setting::WtEnabled(arbitrary_varint(ctx)),
        6 => Setting::WtMaxSessionsDraft14(arbitrary_varint(ctx)),
        7 => Setting::EnableWebTransportDraft02(noprop::sample_bool(ctx)),
        8 => Setting::WebTransportMaxSessionsDraft07(arbitrary_varint(ctx)),
        9 => Setting::WtInitialMaxData(arbitrary_varint(ctx)),
        10 => Setting::WtInitialMaxStreamsUni(arbitrary_varint(ctx)),
        _ => Setting::WtInitialMaxStreamsBidi(arbitrary_varint(ctx)),
    }
}

/// Property: 任意の `Setting` variant を 2 回 `add` すると `DuplicateId { id }`
/// エラーが返る (variant 横断で `Setting::id()` ベースの重複検出が機能することを検証)
#[test]
fn prop_settings_payload_add_duplicate_rejected() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_SETTINGS_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        use shiguredo_http3::frame::SettingsPayload;
        let setting = arbitrary_setting(ctx);
        let mut payload = SettingsPayload::new();
        payload.add(setting).expect("test must succeed");
        let err = payload.add(setting).unwrap_err();
        assert_eq!(err, SettingError::DuplicateId { id: setting.id() });
        Ok(())
    })?;
    Ok(())
}

/// Property: `SettingsPayload::from_settings()` が GREASE 設定 (RFC 9114 §7.2.4.1)
/// を最低 1 つ含む
#[test]
fn prop_settings_payload_includes_grease() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_SETTINGS_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let settings = arbitrary_settings(ctx);
        let payload = SettingsPayload::from_settings(&settings);
        let grease_count = payload
            .settings()
            .iter()
            .filter(|s| s.id().get() == 0x21)
            .count();
        assert_eq!(
            grease_count, 1,
            "GREASE 設定 (0x21) がちょうど 1 つ含まれていること"
        );
        Ok(())
    })?;
    Ok(())
}
