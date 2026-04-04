//! WebTransport SETTINGS (draft-ietf-webtrans-http3-15 Section 9.2)
//!
//! HTTP/3 SETTINGS パラメータの WebTransport 拡張を定義。

use super::connect::DraftVersion;

/// WebTransport SETTINGS パラメータ ID
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum SettingsId {
    /// WebTransport 有効化 (SETTINGS_WT_ENABLED)
    /// draft-ietf-webtrans-http3-15 Section 3.1, Section 9.2
    /// 将来のドラフトで変更される可能性がある
    WtEnabled = 0x2c7cf000,
    /// 初期単方向ストリーム上限 (SETTINGS_WT_INITIAL_MAX_STREAMS_UNI)
    WtInitialMaxStreamsUni = 0x2b64,
    /// 初期双方向ストリーム上限 (SETTINGS_WT_INITIAL_MAX_STREAMS_BIDI)
    WtInitialMaxStreamsBidi = 0x2b65,
    /// 初期データ上限 (SETTINGS_WT_INITIAL_MAX_DATA)
    WtInitialMaxData = 0x2b61,
    /// WebTransport 有効化 (draft-02)
    EnableWebTransportDraft02 = 0x2b603742,
    /// WebTransport 最大セッション数 (draft-07)
    WebTransportMaxSessionsDraft07 = 0xc671706a,
    /// WebTransport 最大セッション数 (draft-14)
    /// draft-14 Section 9.2
    WtMaxSessionsDraft14 = 0x14e9cd29,
}

impl SettingsId {
    /// ID から `SettingsId` を作成
    pub fn from_id(id: u64) -> Option<Self> {
        match id {
            0x2c7cf000 => Some(Self::WtEnabled),
            0x2b64 => Some(Self::WtInitialMaxStreamsUni),
            0x2b65 => Some(Self::WtInitialMaxStreamsBidi),
            0x2b61 => Some(Self::WtInitialMaxData),
            0x2b603742 => Some(Self::EnableWebTransportDraft02),
            0xc671706a => Some(Self::WebTransportMaxSessionsDraft07),
            0x14e9cd29 => Some(Self::WtMaxSessionsDraft14),
            _ => None,
        }
    }

    /// WebTransport 関連の設定 ID かどうか
    pub fn is_webtransport(id: u64) -> bool {
        matches!(
            id,
            0x2c7cf000 | 0x2b64 | 0x2b65 | 0x2b61 | 0x2b603742 | 0xc671706a | 0x14e9cd29
        )
    }
}

/// WebTransport 設定
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    /// WebTransport 有効化 (デフォルト: 0 = WebTransport 無効, 0 より大きければ有効)
    /// draft-ietf-webtrans-http3-15 Section 3.1, Section 9.2
    /// 将来のドラフトで変更される可能性がある
    pub wt_enabled: u64,
    /// 初期単方向ストリーム上限 (デフォルト: 0)
    pub wt_initial_max_streams_uni: u64,
    /// 初期双方向ストリーム上限 (デフォルト: 0)
    pub wt_initial_max_streams_bidi: u64,
    /// 初期データ上限 (デフォルト: 0)
    pub wt_initial_max_data: u64,
    /// WebTransport 有効化 (draft-02)
    pub enable_webtransport_draft02: Option<bool>,
    /// WebTransport 最大セッション数 (draft-07)
    pub webtransport_max_sessions_draft07: Option<u64>,
    /// WebTransport 最大セッション数 (draft-14)
    /// draft-14 Section 9.2
    pub wt_max_sessions_draft14: Option<u64>,
}

impl Default for Settings {
    fn default() -> Self {
        Self::new()
    }
}

impl Settings {
    /// 新しい Settings を作成 (すべてデフォルト値)
    pub const fn new() -> Self {
        Self {
            wt_enabled: 0,
            wt_initial_max_streams_uni: 0,
            wt_initial_max_streams_bidi: 0,
            wt_initial_max_data: 0,
            enable_webtransport_draft02: None,
            webtransport_max_sessions_draft07: None,
            wt_max_sessions_draft14: None,
        }
    }

    /// WebTransport 有効化を設定
    /// draft-ietf-webtrans-http3-15 Section 3.1, Section 9.2
    /// 将来のドラフトで変更される可能性がある
    pub fn wt_enabled(mut self, value: u64) -> Self {
        self.wt_enabled = value;
        self
    }

    /// 初期単方向ストリーム上限を設定
    pub fn wt_initial_max_streams_uni(mut self, max_streams: u64) -> Self {
        self.wt_initial_max_streams_uni = max_streams;
        self
    }

    /// 初期双方向ストリーム上限を設定
    pub fn wt_initial_max_streams_bidi(mut self, max_streams: u64) -> Self {
        self.wt_initial_max_streams_bidi = max_streams;
        self
    }

    /// 初期データ上限を設定
    pub fn wt_initial_max_data(mut self, max_data: u64) -> Self {
        self.wt_initial_max_data = max_data;
        self
    }

    /// WebTransport 有効化 (draft-02) を設定
    pub fn enable_webtransport_draft02(mut self, enable: bool) -> Self {
        self.enable_webtransport_draft02 = Some(enable);
        self
    }

    /// WebTransport 最大セッション数 (draft-07) を設定
    pub fn webtransport_max_sessions_draft07(mut self, max_sessions: u64) -> Self {
        self.webtransport_max_sessions_draft07 = Some(max_sessions);
        self
    }

    /// WebTransport 最大セッション数 (draft-14) を設定
    /// draft-14 Section 9.2
    pub fn wt_max_sessions_draft14(mut self, max_sessions: u64) -> Self {
        self.wt_max_sessions_draft14 = Some(max_sessions);
        self
    }

    /// SETTINGS からドラフトバージョンを検出する
    ///
    /// WebTransport が無効の場合は None を返す。
    ///
    /// 4 パターン (draft-02 / draft-07 / draft-14 / draft-15) を固有の SETTINGS ID で
    /// 判別する。判定は新しいドラフトから順に行う。
    ///
    /// **Safari (Network.framework) の挙動**: draft-07 の ID
    /// `SETTINGS_WEBTRANSPORT_MAX_SESSIONS` と draft-14 の ID `SETTINGS_WT_MAX_SESSIONS`
    /// を **同時** に送ってくるが、サーバーが応答 SETTINGS に draft-14 固有の
    /// `WT_INITIAL_MAX_*` を含めると `H3_REQUEST_CANCELLED` (0x10C) で拒否する。
    /// したがって SETTINGS ネゴシエーションとしては draft-07 を優先し、
    /// draft-14 固有のカプセルベースフロー制御はセッション確立後に別途扱う。
    ///
    /// 判定キー（上から順）:
    /// - draft-15: SETTINGS_WT_ENABLED (0x2c7cf000)
    /// - draft-07: SETTINGS_WEBTRANSPORT_MAX_SESSIONS (0xc671706a)
    /// - draft-14: SETTINGS_WT_MAX_SESSIONS (0x14e9cd29)
    /// - draft-02: SETTINGS_ENABLE_WEBTRANSPORT (0x2b603742)
    ///
    /// draft-ietf-webtrans-http3-02 / -07 / -14 / -15
    /// 将来のドラフトで変更される可能性がある
    pub fn detect_draft_pattern(&self) -> Option<DraftVersion> {
        if self.wt_enabled > 0 {
            return Some(DraftVersion::Draft15);
        }
        if self
            .webtransport_max_sessions_draft07
            .is_some_and(|v| v > 0)
        {
            return Some(DraftVersion::Draft07);
        }
        if self.wt_max_sessions_draft14.is_some_and(|v| v > 0) {
            return Some(DraftVersion::Draft14);
        }
        if self.enable_webtransport_draft02 == Some(true) {
            return Some(DraftVersion::Draft02);
        }
        None
    }

    /// WebTransport が有効かどうか
    ///
    /// draft-02/07/14/15 のいずれかで有効になっていれば true。
    /// 将来のドラフトで変更される可能性がある
    pub fn is_enabled(&self) -> bool {
        self.wt_enabled > 0
            || self.enable_webtransport_draft02 == Some(true)
            || self
                .webtransport_max_sessions_draft07
                .is_some_and(|v| v > 0)
            || self.wt_max_sessions_draft14.is_some_and(|v| v > 0)
    }

    /// フロー制御が有効かどうか
    ///
    /// 以下のいずれかを満たす場合に「フロー制御を使う意図を宣言」する:
    /// - SETTINGS_WT_MAX_SESSIONS > 1 (draft-14 Section 5.1)
    /// - SETTINGS_WT_INITIAL_MAX_STREAMS_UNI != 0
    /// - SETTINGS_WT_INITIAL_MAX_STREAMS_BIDI != 0
    /// - SETTINGS_WT_INITIAL_MAX_DATA != 0
    ///
    /// draft-ietf-webtrans-http3-14 Section 5.1, draft-ietf-webtrans-http3-15 Section 5.1
    /// 将来のドラフトで変更される可能性がある
    pub fn declares_flow_control(&self) -> bool {
        self.wt_max_sessions_draft14.is_some_and(|v| v > 1)
            || self.wt_initial_max_streams_uni != 0
            || self.wt_initial_max_streams_bidi != 0
            || self.wt_initial_max_data != 0
    }

    /// フロー制御を有効として扱うかどうか
    ///
    /// 互換性のためにローカル宣言判定を返す。
    pub fn flow_control_enabled(&self) -> bool {
        self.declares_flow_control()
    }

    /// ピアとのネゴシエーション結果としてフロー制御が有効かどうか
    ///
    /// draft-ietf-webtrans-http3-15 Section 5.1:
    /// 両端点がフロー制御を使う意図を宣言した場合のみ有効。
    /// 将来のドラフトで変更される可能性がある
    pub fn flow_control_enabled_with_peer(&self, peer: &Self) -> bool {
        self.declares_flow_control() && peer.declares_flow_control()
    }

    /// 互換性のためにセッション確立直後の初期フロー制御カプセルが必要かどうか
    ///
    /// Safari 26.4 は draft-07 の `SETTINGS_WEBTRANSPORT_MAX_SESSIONS` で
    /// WebTransport をネゴシエートしつつ、`SETTINGS_WT_INITIAL_MAX_*` で
    /// フロー制御の意図を宣言し、セッション確立直後の
    /// `WT_MAX_STREAMS` / `WT_MAX_DATA` カプセルを要求する。
    ///
    /// そのため、ドラフト判定自体は draft-07 のまま維持しつつ、
    /// 初期カプセル要否だけは別判定にする。
    ///
    /// draft-ietf-webtrans-http3-14 Section 5,
    /// Safari 26.4 (Network.framework) 実装互換
    pub fn requires_initial_capsule_flow_control_compat(&self) -> bool {
        match self.detect_draft_pattern() {
            Some(DraftVersion::Draft14) => true,
            Some(DraftVersion::Draft07) => {
                self.wt_initial_max_streams_uni != 0
                    || self.wt_initial_max_streams_bidi != 0
                    || self.wt_initial_max_data != 0
            }
            _ => false,
        }
    }

    /// 複数セッションの同時使用が許可されるかどうか
    ///
    /// フロー制御が有効でない場合、エンドポイントは同時に 1 セッションのみ使用可能 (MUST NOT)。
    /// draft-ietf-webtrans-http3-15 Section 5.2
    /// 将来のドラフトで変更される可能性がある
    pub fn allows_multiple_sessions_with_peer(&self, peer: &Self) -> bool {
        self.flow_control_enabled_with_peer(peer)
    }

    /// SettingsPayload から WebTransport Settings を作成
    ///
    /// WebTransport 関連の設定 ID のみをパースする。
    /// 1 つも WebTransport 関連 ID が含まれない場合は None を返す。
    ///
    /// `SETTINGS_ENABLE_WEBTRANSPORT_DRAFT02` (0x2b603742) はブール設定であり、
    /// 値が 0 または 1 以外の場合は `H3_SETTINGS_ERROR` を返す。
    /// draft-ietf-webtrans-http3-02 由来。将来変更される可能性がある。
    pub fn from_payload(
        payload: &crate::frame::SettingsPayload,
    ) -> Result<Option<Self>, crate::error::Error> {
        let mut settings = Self::new();
        let mut found = false;
        for (id, value) in &payload.entries {
            match *id {
                0x2c7cf000 => {
                    settings.wt_enabled = *value;
                    found = true;
                }
                0x2b64 => {
                    settings.wt_initial_max_streams_uni = *value;
                    found = true;
                }
                0x2b65 => {
                    settings.wt_initial_max_streams_bidi = *value;
                    found = true;
                }
                0x2b61 => {
                    settings.wt_initial_max_data = *value;
                    found = true;
                }
                // SETTINGS_ENABLE_WEBTRANSPORT_DRAFT02: ブール設定
                // 値は 0 または 1 のみ
                // (draft-ietf-webtrans-http3-02 由来、将来変更される可能性がある)
                0x2b603742 => {
                    if *value > 1 {
                        return Err(crate::error::Error::ConnectionError(
                            crate::error::ErrorCode::SettingsError,
                        ));
                    }
                    settings.enable_webtransport_draft02 = Some(*value == 1);
                    found = true;
                }
                0xc671706a => {
                    settings.webtransport_max_sessions_draft07 = Some(*value);
                    found = true;
                }
                0x14e9cd29 => {
                    settings.wt_max_sessions_draft14 = Some(*value);
                    found = true;
                }
                _ => {} // H3 設定や不明な ID は無視
            }
        }
        Ok(found.then_some(settings))
    }

    /// 設定エントリのイテレータを返す
    ///
    /// 0 以外の値を持つ設定のみを返す。
    pub fn iter(&self) -> impl Iterator<Item = (u64, u64)> + '_ {
        let entries = [
            (self.wt_enabled > 0).then_some((SettingsId::WtEnabled as u64, self.wt_enabled)),
            (self.wt_initial_max_streams_uni > 0).then_some((
                SettingsId::WtInitialMaxStreamsUni as u64,
                self.wt_initial_max_streams_uni,
            )),
            (self.wt_initial_max_streams_bidi > 0).then_some((
                SettingsId::WtInitialMaxStreamsBidi as u64,
                self.wt_initial_max_streams_bidi,
            )),
            (self.wt_initial_max_data > 0).then_some((
                SettingsId::WtInitialMaxData as u64,
                self.wt_initial_max_data,
            )),
            self.enable_webtransport_draft02
                .map(|v| (SettingsId::EnableWebTransportDraft02 as u64, u64::from(v))),
            self.webtransport_max_sessions_draft07
                .map(|v| (SettingsId::WebTransportMaxSessionsDraft07 as u64, v)),
            self.wt_max_sessions_draft14
                .map(|v| (SettingsId::WtMaxSessionsDraft14 as u64, v)),
        ];
        entries.into_iter().flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_settings_id_from_id() {
        assert_eq!(SettingsId::from_id(0x2c7cf000), Some(SettingsId::WtEnabled));
        assert_eq!(
            SettingsId::from_id(0x2b64),
            Some(SettingsId::WtInitialMaxStreamsUni)
        );
        assert_eq!(
            SettingsId::from_id(0x2b65),
            Some(SettingsId::WtInitialMaxStreamsBidi)
        );
        assert_eq!(
            SettingsId::from_id(0x2b61),
            Some(SettingsId::WtInitialMaxData)
        );
        assert_eq!(
            SettingsId::from_id(0x2b603742),
            Some(SettingsId::EnableWebTransportDraft02)
        );
        assert_eq!(
            SettingsId::from_id(0xc671706a),
            Some(SettingsId::WebTransportMaxSessionsDraft07)
        );
        assert_eq!(SettingsId::from_id(0x99), None);
    }

    #[test]
    fn test_settings_id_is_webtransport() {
        assert!(SettingsId::is_webtransport(0x2c7cf000));
        assert!(SettingsId::is_webtransport(0x2b64));
        assert!(SettingsId::is_webtransport(0x2b65));
        assert!(SettingsId::is_webtransport(0x2b61));
        assert!(SettingsId::is_webtransport(0x2b603742));
        assert!(SettingsId::is_webtransport(0xc671706a));
        assert!(!SettingsId::is_webtransport(0x01));
        assert!(!SettingsId::is_webtransport(0x99));
    }

    #[test]
    fn test_settings_default() {
        let settings = Settings::default();
        assert_eq!(settings.wt_enabled, 0);
        assert!(!settings.is_enabled());
        assert!(!settings.flow_control_enabled());
    }

    #[test]
    fn test_settings_builder() {
        let settings = Settings::new()
            .wt_enabled(1)
            .wt_initial_max_streams_uni(100)
            .wt_initial_max_streams_bidi(50)
            .wt_initial_max_data(1024 * 1024);

        assert_eq!(settings.wt_enabled, 1);
        assert_eq!(settings.wt_initial_max_streams_uni, 100);
        assert_eq!(settings.wt_initial_max_streams_bidi, 50);
        assert_eq!(settings.wt_initial_max_data, 1024 * 1024);
        assert!(settings.is_enabled());
        assert!(settings.flow_control_enabled());
    }

    #[test]
    fn test_settings_draft02_07() {
        let settings = Settings::new()
            .enable_webtransport_draft02(true)
            .webtransport_max_sessions_draft07(5);

        assert!(settings.is_enabled());
        assert_eq!(settings.enable_webtransport_draft02, Some(true));
        assert_eq!(settings.webtransport_max_sessions_draft07, Some(5));
    }

    #[test]
    fn test_is_enabled_draft02() {
        let settings = Settings::new().enable_webtransport_draft02(true);
        assert!(settings.is_enabled());

        let settings = Settings::new().enable_webtransport_draft02(false);
        assert!(!settings.is_enabled());
    }

    #[test]
    fn test_is_enabled_draft07() {
        let settings = Settings::new().webtransport_max_sessions_draft07(1);
        assert!(settings.is_enabled());

        let settings = Settings::new().webtransport_max_sessions_draft07(0);
        assert!(!settings.is_enabled());
    }

    #[test]
    fn test_flow_control_enabled() {
        // wt_enabled のみではフロー制御無効 (draft-15: INITIAL_MAX_* が必要)
        let settings = Settings::new().wt_enabled(1);
        assert!(!settings.flow_control_enabled());

        let settings = Settings::new().wt_enabled(2);
        assert!(!settings.flow_control_enabled());

        // INITIAL_MAX_* が非ゼロならフロー制御有効
        let settings = Settings::new().wt_enabled(1).wt_initial_max_streams_uni(10);
        assert!(settings.flow_control_enabled());

        let settings = Settings::new().wt_enabled(1).wt_initial_max_streams_bidi(5);
        assert!(settings.flow_control_enabled());

        let settings = Settings::new().wt_enabled(1).wt_initial_max_data(1024);
        assert!(settings.flow_control_enabled());
    }

    #[test]
    fn test_flow_control_enabled_with_peer() {
        let local = Settings::new().wt_initial_max_streams_uni(10);
        let peer = Settings::new().wt_enabled(1);
        assert!(!local.flow_control_enabled_with_peer(&peer));

        let peer = Settings::new().wt_initial_max_data(1);
        assert!(local.flow_control_enabled_with_peer(&peer));
    }

    #[test]
    fn test_settings_iter() {
        let settings = Settings::new().wt_enabled(1).wt_initial_max_data(4096);

        let entries: Vec<_> = settings.iter().collect();
        assert_eq!(entries.len(), 2);
        assert!(entries.contains(&(0x2c7cf000, 1)));
        assert!(entries.contains(&(0x2b61, 4096)));
    }

    #[test]
    fn test_settings_iter_with_draft02_07() {
        let settings = Settings::new()
            .wt_enabled(1)
            .enable_webtransport_draft02(true)
            .webtransport_max_sessions_draft07(3);

        let entries: Vec<_> = settings.iter().collect();
        assert_eq!(entries.len(), 3);
        assert!(entries.contains(&(0x2c7cf000, 1)));
        assert!(entries.contains(&(0x2b603742, 1)));
        assert!(entries.contains(&(0xc671706a, 3)));
    }

    #[test]
    fn test_from_payload() {
        let mut payload = crate::frame::SettingsPayload::new();
        payload.add(0x2c7cf000, 1);
        payload.add(0x2b64, 100);
        payload.add(0x2b65, 50);
        payload.add(0x2b61, 1048576);
        payload.add(0x2b603742, 1);
        payload.add(0xc671706a, 3);

        let settings = Settings::from_payload(&payload).unwrap().unwrap();
        assert_eq!(settings.wt_enabled, 1);
        assert_eq!(settings.wt_initial_max_streams_uni, 100);
        assert_eq!(settings.wt_initial_max_streams_bidi, 50);
        assert_eq!(settings.wt_initial_max_data, 1048576);
        assert_eq!(settings.enable_webtransport_draft02, Some(true));
        assert_eq!(settings.webtransport_max_sessions_draft07, Some(3));
    }

    #[test]
    fn test_from_payload_none_when_no_wt_entries() {
        let mut payload = crate::frame::SettingsPayload::new();
        payload.add(0x01, 4096); // QPACK_MAX_TABLE_CAPACITY (H3 設定)

        assert!(Settings::from_payload(&payload).unwrap().is_none());
    }

    #[test]
    fn test_detect_draft_pattern_draft15() {
        let settings = Settings::new().wt_enabled(1);
        assert_eq!(settings.detect_draft_pattern(), Some(DraftVersion::Draft15));
    }

    #[test]
    fn test_detect_draft_pattern_draft14() {
        let settings = Settings::new().wt_max_sessions_draft14(1);
        assert_eq!(settings.detect_draft_pattern(), Some(DraftVersion::Draft14));
    }

    #[test]
    fn test_detect_draft_pattern_draft07() {
        let settings = Settings::new().webtransport_max_sessions_draft07(1);
        assert_eq!(settings.detect_draft_pattern(), Some(DraftVersion::Draft07));
    }

    #[test]
    fn test_detect_draft_pattern_draft02() {
        let settings = Settings::new().enable_webtransport_draft02(true);
        assert_eq!(settings.detect_draft_pattern(), Some(DraftVersion::Draft02));
    }

    #[test]
    fn test_detect_draft_pattern_none() {
        let settings = Settings::new();
        assert_eq!(settings.detect_draft_pattern(), None);

        // draft-02 が false の場合も None
        let settings = Settings::new().enable_webtransport_draft02(false);
        assert_eq!(settings.detect_draft_pattern(), None);

        // draft-14 が 0 の場合も None
        let settings = Settings::new().wt_max_sessions_draft14(0);
        assert_eq!(settings.detect_draft_pattern(), None);
    }

    #[test]
    fn test_detect_draft_pattern_priority() {
        // draft-07 と draft-14 を両方送る場合は SETTINGS ネゴシエーションとしては
        // draft-07 を優先する (Safari が draft-14 固有の応答 SETTINGS を拒否するため)
        let settings = Settings::new()
            .webtransport_max_sessions_draft07(1)
            .wt_max_sessions_draft14(1);
        assert_eq!(settings.detect_draft_pattern(), Some(DraftVersion::Draft07));

        // draft-15 が最優先
        let settings = Settings::new()
            .wt_enabled(1)
            .wt_max_sessions_draft14(1)
            .webtransport_max_sessions_draft07(1);
        assert_eq!(settings.detect_draft_pattern(), Some(DraftVersion::Draft15));
    }

    #[test]
    fn test_detect_draft_pattern_safari_observed() {
        // Safari 26.4 の実測パターン:
        // - draft-07 の SETTINGS_WEBTRANSPORT_MAX_SESSIONS
        // - draft-15 系 ID の SETTINGS_WT_INITIAL_MAX_*
        // draft 自体は draft-07 として扱う。
        let settings = Settings::new()
            .webtransport_max_sessions_draft07(1)
            .wt_initial_max_data(8388608)
            .wt_initial_max_streams_uni(100)
            .wt_initial_max_streams_bidi(100);
        assert_eq!(settings.detect_draft_pattern(), Some(DraftVersion::Draft07));
    }

    #[test]
    fn test_detect_draft_pattern_safari_legacy_combo() {
        // Safari が draft-07 と draft-14 の ID を併送するケース
        // (実測: Safari 26.4 は 0xc671706a と 0x14e9cd29 を同時送信)。
        // SETTINGS ネゴシエーションとしては draft-07 を優先する。
        // draft-14 固有のカプセルベースフロー制御はセッション確立後に別途扱う。
        let settings = Settings::new()
            .webtransport_max_sessions_draft07(1)
            .wt_max_sessions_draft14(1)
            .wt_initial_max_data(8388608)
            .wt_initial_max_streams_uni(100)
            .wt_initial_max_streams_bidi(100);
        assert_eq!(settings.detect_draft_pattern(), Some(DraftVersion::Draft07));
    }

    #[test]
    fn test_declares_flow_control_draft14_max_sessions() {
        // draft-14: WT_MAX_SESSIONS > 1 でフロー制御宣言
        let settings = Settings::new().wt_max_sessions_draft14(2);
        assert!(settings.declares_flow_control());

        // WT_MAX_SESSIONS = 1 ではフロー制御宣言にならない
        let settings = Settings::new().wt_max_sessions_draft14(1);
        assert!(!settings.declares_flow_control());
    }

    #[test]
    fn test_requires_initial_capsule_flow_control_compat_safari_observed() {
        let settings = Settings::new()
            .webtransport_max_sessions_draft07(1)
            .wt_initial_max_streams_uni(100)
            .wt_initial_max_streams_bidi(100)
            .wt_initial_max_data(8 * 1024 * 1024);
        assert!(settings.requires_initial_capsule_flow_control_compat());
    }

    #[test]
    fn test_requires_initial_capsule_flow_control_compat_draft07_plain() {
        let settings = Settings::new().webtransport_max_sessions_draft07(1);
        assert!(!settings.requires_initial_capsule_flow_control_compat());
    }

    #[test]
    fn test_is_enabled_draft14() {
        let settings = Settings::new().wt_max_sessions_draft14(1);
        assert!(settings.is_enabled());

        let settings = Settings::new().wt_max_sessions_draft14(0);
        assert!(!settings.is_enabled());
    }
}
