//! Property-Based Testing for HTTP/3 Settings (RFC 9114 Section 7.2.4)

use proptest::prelude::*;
use shiguredo_http3::Settings;
use shiguredo_http3::error::{Error, ErrorCode};
use shiguredo_http3::frame::SettingsPayload;
use shiguredo_http3::webtransport;

// =============================================================================
// Strategy ヘルパー
// =============================================================================

prop_compose! {
    /// 任意の webtransport::Settings を生成
    fn arbitrary_wt_settings()(
        wt_enabled in 0u64..256,
        wt_initial_max_streams_uni in 0u64..256,
        wt_initial_max_streams_bidi in 0u64..256,
        wt_initial_max_data in 0u64..1_000_000,
        enable_webtransport_draft02 in proptest::option::of(any::<bool>()),
        webtransport_max_sessions_draft07 in proptest::option::of(0u64..256),
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
        qpack_max_table_capacity in proptest::option::of(0u64..65536),
        max_field_section_size in proptest::option::of(0u64..1_000_000),
        qpack_blocked_streams in proptest::option::of(0u64..256),
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

// =============================================================================
// (a) Settings → SettingsPayload → Settings ラウンドトリップ
// =============================================================================

proptest! {
    /// Property: Settings を SettingsPayload に変換して from_payload で復元すると元と一致する
    #[test]
    fn prop_settings_roundtrip(settings in arbitrary_settings()) {
        // H3 + WT の全エントリを SettingsPayload に入れる
        let payload = SettingsPayload::from_settings(&settings);

        // ビルダーで構築した Settings は必ず有効な値のみ含むため unwrap 可
        let restored = Settings::from_payload(&payload).unwrap();

        // H3 フィールド比較
        prop_assert_eq!(
            settings.qpack_max_table_capacity, restored.qpack_max_table_capacity,
            "qpack_max_table_capacity が不一致"
        );
        prop_assert_eq!(
            settings.max_field_section_size, restored.max_field_section_size,
            "max_field_section_size が不一致"
        );
        prop_assert_eq!(
            settings.qpack_blocked_streams, restored.qpack_blocked_streams,
            "qpack_blocked_streams が不一致"
        );
        prop_assert_eq!(
            settings.enable_connect_protocol, restored.enable_connect_protocol,
            "enable_connect_protocol が不一致"
        );
        prop_assert_eq!(
            settings.h3_datagram, restored.h3_datagram,
            "h3_datagram が不一致"
        );

        // enable_webtransport() で wt_settings を設定した場合、
        // enable_connect_protocol と h3_datagram も自動設定されるため、
        // from_payload 側では wt_settings の有無が元と一致することを確認する。
        // ただし iter() は 0 の値を出力しないため、全フィールドが 0 の wt_settings は
        // from_payload で None になる点に注意。
        match (&settings.wt_settings, &restored.wt_settings) {
            (Some(orig), Some(rest)) => {
                prop_assert_eq!(orig, rest, "wt_settings が不一致");
            }
            (Some(orig), None) => {
                // 全フィールドがデフォルト (0/None) なら iter() が空になるので None は正常
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

// =============================================================================
// (b) iter() の要素数と len() が一致
// =============================================================================

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
        let len = settings.len();
        prop_assert_eq!(
            total, len,
            "iter 合計 ({}) と len() ({}) が不一致", total, len
        );
    }
}

// =============================================================================
// (c) enable_webtransport で必要な設定が全て有効
// =============================================================================

proptest! {
    /// Property: enable_webtransport(wt) 後は is_webtransport_enabled() == true かつ
    ///           enable_connect_protocol と h3_datagram が Some(true)
    #[test]
    fn prop_enable_webtransport_sets_required_fields(
        wt_settings in arbitrary_wt_settings().prop_filter(
            "wt_enabled が 0 だと is_enabled() が false になるためスキップ",
            |wt| wt.is_enabled()
        )
    ) {
        let settings = Settings::new().enable_webtransport_server(wt_settings);

        prop_assert!(
            settings.is_webtransport_enabled(),
            "enable_webtransport 後に is_webtransport_enabled() が false"
        );
        prop_assert_eq!(
            settings.enable_connect_protocol,
            Some(true),
            "enable_connect_protocol が Some(true) でない"
        );
        prop_assert_eq!(
            settings.h3_datagram,
            Some(true),
            "h3_datagram が Some(true) でない"
        );
    }
}

// =============================================================================
// (d) webtransport_draft_pattern は wt_settings.detect_draft_pattern() と一致
// =============================================================================

proptest! {
    /// Property: Settings::webtransport_draft_pattern() は
    /// wt_settings.as_ref().and_then(|wt| wt.detect_draft_pattern()) と一致する
    #[test]
    fn prop_webtransport_draft_pattern_consistent(settings in arbitrary_settings()) {
        let expected = settings.wt_settings
            .as_ref()
            .and_then(|wt| wt.detect_draft_pattern());
        let actual = settings.webtransport_draft_pattern();
        prop_assert_eq!(
            actual, expected,
            "webtransport_draft_pattern() が wt_settings.detect_draft_pattern() と不一致"
        );
    }
}

// =============================================================================
// (e) ブール SETTINGS の不正値で H3_SETTINGS_ERROR
// =============================================================================

proptest! {
    /// Property: SETTINGS_ENABLE_CONNECT_PROTOCOL (0x08) に 2 以上の値を設定すると
    /// H3_SETTINGS_ERROR が返る (RFC 8441 Section 3)
    #[test]
    fn prop_enable_connect_protocol_invalid_value(value in 2u64..=u64::MAX) {
        let mut payload = SettingsPayload::new();
        payload.add(0x08, value);
        let result = Settings::from_payload(&payload);
        match result {
            Err(Error::ConnectionError(ErrorCode::SettingsError)) => {}
            other => prop_assert!(
                false,
                "0x08 に値 {} を設定したとき H3_SETTINGS_ERROR を期待したが {:?} が返った",
                value, other
            ),
        }
    }

    /// Property: SETTINGS_H3_DATAGRAM (0x33) に 2 以上の値を設定すると
    /// H3_SETTINGS_ERROR が返る (RFC 9297 Section 2.1.1)
    #[test]
    fn prop_h3_datagram_invalid_value(value in 2u64..=u64::MAX) {
        let mut payload = SettingsPayload::new();
        payload.add(0x33, value);
        let result = Settings::from_payload(&payload);
        match result {
            Err(Error::ConnectionError(ErrorCode::SettingsError)) => {}
            other => prop_assert!(
                false,
                "0x33 に値 {} を設定したとき H3_SETTINGS_ERROR を期待したが {:?} が返った",
                value, other
            ),
        }
    }

    /// Property: SETTINGS_ENABLE_WEBTRANSPORT_DRAFT02 (0x2b603742) に 2 以上の値を設定すると
    /// H3_SETTINGS_ERROR が返る
    /// (draft-ietf-webtrans-http3-02 由来、将来変更される可能性がある)
    #[test]
    fn prop_enable_webtransport_draft02_invalid_value(value in 2u64..=u64::MAX) {
        let mut payload = SettingsPayload::new();
        payload.add(0x2b603742, value);
        let result = Settings::from_payload(&payload);
        match result {
            Err(Error::ConnectionError(ErrorCode::SettingsError)) => {}
            other => prop_assert!(
                false,
                "0x2b603742 に値 {} を設定したとき H3_SETTINGS_ERROR を期待したが {:?} が返った",
                value, other
            ),
        }
    }

}
