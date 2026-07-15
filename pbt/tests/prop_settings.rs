//! Property-Based Testing for HTTP/3 Settings (RFC 9114 §7.2.4)
//!
//! 値域や bool 値検査は `Setting::from_wire` 側で構築時に検査されるため、
//! 不正値の単体テストは `src/settings.rs` の `#[cfg(test)]` に移してある。
//! 本ファイルは Strategy で構築した正常値が SettingsPayload とのラウンドトリップ /
//! `Setting::from_wire` ↔ `Setting::as_wire` の対称性を満たすことを検査する。

use proptest::prelude::*;
use shiguredo_http3::frame::SettingsPayload;
use shiguredo_http3::{Setting, SettingError, Settings, VarInt, webtransport};

fn vi(value: u64) -> VarInt {
    VarInt::new(value).expect("test must succeed")
}

// `arbitrary_settings` / `arbitrary_wt_settings` で生成する範囲は中規模 (`1 << 30`)
// に絞り、Strategy ノイズを抑える。`prop_setting_wire_roundtrip` 等で VarInt
// 全域を叩く場合は別 Strategy (`arbitrary_varint_full`) を用いる。
const VARINT_TEST_MAX: u64 = 1 << 30;

prop_compose! {
    fn arbitrary_varint()(value in 0u64..VARINT_TEST_MAX) -> VarInt {
        vi(value)
    }
}

prop_compose! {
    /// VarInt 全域 (`0..=VarInt::MAX`) を生成する Strategy
    fn arbitrary_varint_full()(value in 0u64..=VarInt::MAX.get()) -> VarInt {
        vi(value)
    }
}

/// `Setting::from_wire` で受理される ID のみを生成する Strategy
///
/// HTTP/2 専用 ID (0x02..=0x05) と予約 ID (0x00) を Strategy 段階で除外し、
/// `prop_assume!` の reject カウンタ消費を回避する。
fn settable_id() -> impl Strategy<Value = VarInt> {
    arbitrary_varint_full().prop_filter(
        "HTTP/2 専用 ID と予約 ID は構築時拒否されるため除外",
        |v| {
            let raw = v.get();
            raw != 0 && !matches!(raw, 0x02..=0x05)
        },
    )
}

prop_compose! {
    /// 任意の webtransport::Settings を生成
    fn arbitrary_wt_settings()(
        wt_enabled in arbitrary_varint(),
        wt_initial_max_streams_uni in arbitrary_varint(),
        wt_initial_max_streams_bidi in arbitrary_varint(),
        wt_initial_max_data in arbitrary_varint(),
        enable_webtransport_draft02 in proptest::option::of(any::<bool>()),
        webtransport_max_sessions_draft07 in proptest::option::of(arbitrary_varint()),
    ) -> webtransport::Settings {
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
}

prop_compose! {
    /// 任意の Settings を生成 (各フィールドが Some または None)
    fn arbitrary_settings()(
        qpack_max_table_capacity in proptest::option::of(arbitrary_varint()),
        max_field_section_size in proptest::option::of(arbitrary_varint()),
        qpack_blocked_streams in proptest::option::of(arbitrary_varint()),
        enable_connect_protocol in proptest::option::of(any::<bool>()),
        h3_datagram in proptest::option::of(any::<bool>()),
        wt_settings in proptest::option::of(arbitrary_wt_settings()),
    ) -> Settings {
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
}

proptest! {
    /// Property: Settings を SettingsPayload に変換して from_payload で復元すると元と一致する
    #[test]
    fn prop_settings_roundtrip(settings in arbitrary_settings()) {
        let payload = SettingsPayload::from_settings(&settings);
        let restored = Settings::from_payload(&payload);

        prop_assert_eq!(settings.qpack_max_table_capacity, restored.qpack_max_table_capacity);
        prop_assert_eq!(settings.max_field_section_size, restored.max_field_section_size);
        prop_assert_eq!(settings.qpack_blocked_streams, restored.qpack_blocked_streams);
        prop_assert_eq!(settings.enable_connect_protocol, restored.enable_connect_protocol);
        prop_assert_eq!(settings.h3_datagram, restored.h3_datagram);

        // enable_webtransport_server() で wt_settings を設定した場合、
        // enable_connect_protocol と h3_datagram も自動設定されるため、
        // from_payload 側では wt_settings の有無が元と一致することを確認する。
        // ただし iter() は 0 の値を出力しないため、全フィールドが 0 の wt_settings は
        // from_payload で None になる。
        match (&settings.wt_settings, &restored.wt_settings) {
            (Some(orig), Some(rest)) => {
                prop_assert_eq!(orig, rest, "wt_settings が不一致");
            }
            (Some(orig), None) => {
                prop_assert!(
                    orig.iter().count() == 0,
                    "元の wt_settings にエントリがあるのに復元が None"
                );
            }
            (None, None) => {}
            (None, Some(_)) => {
                prop_assert!(false, "元が None なのに復元に wt_settings がある");
            }
        }
    }
}

proptest! {
    /// Property: Settings::len() は H3 + WT のエントリ合計数と一致する
    #[test]
    fn prop_iter_count_equals_len(settings in arbitrary_settings()) {
        let h3_count = settings.iter().count();
        let wt_count = settings.wt_settings
            .as_ref()
            .map(|wt| wt.iter().count())
            .unwrap_or(0);
        let total = h3_count + wt_count;
        prop_assert_eq!(total, settings.len());
    }
}

proptest! {
    /// Property: enable_webtransport_server(wt) 後は is_webtransport_enabled() == true かつ
    ///           enable_connect_protocol と h3_datagram が Some(true)
    #[test]
    fn prop_enable_webtransport_sets_required_fields(
        wt_settings in arbitrary_wt_settings().prop_filter(
            "wt_enabled が 0 だと is_enabled() が false になるためスキップ",
            |wt| wt.is_enabled()
        )
    ) {
        let settings = Settings::new().enable_webtransport_server(wt_settings);
        prop_assert!(settings.is_webtransport_enabled());
        prop_assert_eq!(settings.enable_connect_protocol, Some(true));
        prop_assert_eq!(settings.h3_datagram, Some(true));
    }
}

proptest! {
    /// Property: Settings::webtransport_draft_pattern() は
    /// wt_settings.as_ref().and_then(|wt| wt.detect_draft_pattern()) と一致する
    #[test]
    fn prop_webtransport_draft_pattern_consistent(settings in arbitrary_settings()) {
        let expected = settings.wt_settings
            .as_ref()
            .and_then(|wt| wt.detect_draft_pattern());
        let actual = settings.webtransport_draft_pattern();
        prop_assert_eq!(actual, expected);
    }
}

proptest! {
    /// Property: `Setting::from_wire` → `as_wire` → `from_wire` のラウンドトリップ
    ///
    /// VarInt 全域 (`encoded_settings_payload_len` の合算境界含む) を叩く。
    /// HTTP/2 専用 / 予約 ID は Strategy 段で除外して shrink ノイズを抑える。
    #[test]
    fn prop_setting_wire_roundtrip(
        id in settable_id(),
        value in arbitrary_varint_full(),
    ) {
        // bool 値 ID の場合は値域を 0/1 に正規化する
        let id_raw = id.get();
        let bool_ids = [0x08u64, 0x33, 0x2b603742];
        let value = if bool_ids.contains(&id_raw) {
            vi(value.get() & 1)
        } else {
            value
        };
        let setting = Setting::from_wire(id, value).expect("test must succeed");
        let (id2, value2) = setting.as_wire();
        prop_assert_eq!(id, id2);
        prop_assert_eq!(value, value2);
        let restored = Setting::from_wire(id2, value2).expect("test must succeed");
        prop_assert_eq!(setting, restored);
    }
}

/// 任意の `Setting` variant を 1 つ生成する Strategy
///
/// HTTP/2 専用 / 予約 ID は `Setting::from_wire` で構築不可のため除外。
/// 重複検出 PBT で variant 全体に検査が及ぶことを保証するために定義する。
fn arbitrary_setting() -> impl Strategy<Value = Setting> {
    prop_oneof![
        arbitrary_varint().prop_map(Setting::QpackMaxTableCapacity),
        arbitrary_varint().prop_map(Setting::MaxFieldSectionSize),
        arbitrary_varint().prop_map(Setting::QpackBlockedStreams),
        any::<bool>().prop_map(Setting::EnableConnectProtocol),
        any::<bool>().prop_map(Setting::H3Datagram),
        arbitrary_varint().prop_map(Setting::WtEnabled),
        arbitrary_varint().prop_map(Setting::WtMaxSessionsDraft14),
        any::<bool>().prop_map(Setting::EnableWebTransportDraft02),
        arbitrary_varint().prop_map(Setting::WebTransportMaxSessionsDraft07),
        arbitrary_varint().prop_map(Setting::WtInitialMaxData),
        arbitrary_varint().prop_map(Setting::WtInitialMaxStreamsUni),
        arbitrary_varint().prop_map(Setting::WtInitialMaxStreamsBidi),
    ]
}

proptest! {
    /// Property: 任意の `Setting` variant を 2 回 `add` すると `DuplicateId { id }`
    /// エラーが返る (variant 横断で `Setting::id()` ベースの重複検出が機能することを検証)
    #[test]
    fn prop_settings_payload_add_duplicate_rejected(setting in arbitrary_setting()) {
        use shiguredo_http3::frame::SettingsPayload;
        let mut payload = SettingsPayload::new();
        payload.add(setting).expect("test must succeed");
        let err = payload.add(setting).unwrap_err();
        prop_assert_eq!(err, SettingError::DuplicateId { id: setting.id() });
    }
}

proptest! {
    /// Property: `SettingsPayload::from_settings()` が GREASE 設定 (RFC 9114 §7.2.4.1)
    /// を最低 1 つ含む
    #[test]
    fn prop_settings_payload_includes_grease(settings in arbitrary_settings()) {
        let payload = SettingsPayload::from_settings(&settings);
        let grease_count = payload.settings().iter().filter(|s| s.id().get() == 0x21).count();
        prop_assert_eq!(grease_count, 1, "GREASE 設定 (0x21) がちょうど 1 つ含まれていること");
    }
}
