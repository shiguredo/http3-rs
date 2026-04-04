//! HTTP/3 Settings (RFC 9114 Section 7.2.4)
//!
//! SETTINGS フレームで交換される設定パラメータを管理。

use crate::limits::Limits;
use crate::webtransport;

/// SETTINGS パラメータ ID
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum SettingsId {
    /// QPACK 最大テーブル容量
    QpackMaxTableCapacity = 0x01,
    /// 最大ヘッダーセクションサイズ
    MaxFieldSectionSize = 0x06,
    /// QPACK ブロックストリーム数
    QpackBlockedStreams = 0x07,
    /// CONNECT プロトコル有効化
    EnableConnectProtocol = 0x08,
    /// H3 Datagram 有効化
    H3Datagram = 0x33,
}

impl SettingsId {
    /// ID から `SettingsId` を作成
    pub fn from_id(id: u64) -> Option<Self> {
        match id {
            0x01 => Some(Self::QpackMaxTableCapacity),
            0x06 => Some(Self::MaxFieldSectionSize),
            0x07 => Some(Self::QpackBlockedStreams),
            0x08 => Some(Self::EnableConnectProtocol),
            0x33 => Some(Self::H3Datagram),
            _ => None,
        }
    }

    /// HTTP/2 専用の設定 ID かどうか
    pub fn is_http2_only(id: u64) -> bool {
        matches!(id, 0x02..=0x05)
    }
}

/// HTTP/3 設定
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    /// QPACK 最大テーブル容量
    pub qpack_max_table_capacity: Option<u64>,
    /// 最大ヘッダーセクションサイズ
    pub max_field_section_size: Option<u64>,
    /// QPACK ブロックストリーム数
    pub qpack_blocked_streams: Option<u64>,
    /// CONNECT プロトコル有効化
    pub enable_connect_protocol: Option<bool>,
    /// H3 Datagram 有効化
    pub h3_datagram: Option<bool>,
    /// WebTransport 設定
    pub wt_settings: Option<webtransport::Settings>,
}

impl Default for Settings {
    fn default() -> Self {
        Self::new()
    }
}

impl Settings {
    /// 新しい Settings を作成 (すべて None)
    pub const fn new() -> Self {
        Self {
            qpack_max_table_capacity: None,
            max_field_section_size: None,
            qpack_blocked_streams: None,
            enable_connect_protocol: None,
            h3_datagram: None,
            wt_settings: None,
        }
    }

    /// Limits から Settings を作成
    pub fn from_limits(limits: &Limits) -> Self {
        Self {
            qpack_max_table_capacity: Some(limits.qpack_max_table_capacity),
            max_field_section_size: Some(limits.max_field_section_size),
            qpack_blocked_streams: Some(limits.qpack_blocked_streams),
            enable_connect_protocol: None,
            h3_datagram: None,
            wt_settings: None,
        }
    }

    /// QPACK 最大テーブル容量を設定
    pub fn qpack_max_table_capacity(mut self, capacity: u64) -> Self {
        self.qpack_max_table_capacity = Some(capacity);
        self
    }

    /// 最大ヘッダーセクションサイズを設定
    pub fn max_field_section_size(mut self, size: u64) -> Self {
        self.max_field_section_size = Some(size);
        self
    }

    /// QPACK ブロックストリーム数を設定
    pub fn qpack_blocked_streams(mut self, streams: u64) -> Self {
        self.qpack_blocked_streams = Some(streams);
        self
    }

    /// CONNECT プロトコル有効化を設定
    pub fn enable_connect_protocol(mut self, enable: bool) -> Self {
        self.enable_connect_protocol = Some(enable);
        self
    }

    /// H3 Datagram 有効化を設定
    pub fn h3_datagram(mut self, enable: bool) -> Self {
        self.h3_datagram = Some(enable);
        self
    }

    /// WebTransport をサーバー側で有効にする
    ///
    /// 以下の設定を自動的に行う:
    /// - SETTINGS_ENABLE_CONNECT_PROTOCOL: 1 (サーバー送信項目)
    /// - SETTINGS_H3_DATAGRAM: 1 (サーバー送信項目)
    ///
    /// `SETTINGS_ENABLE_CONNECT_PROTOCOL` は CONNECT 拡張の受諾を広告する
    /// サーバー側の設定であり、クライアントが送る項目ではない
    /// (RFC 8441 / RFC 9220, draft-ietf-webtrans-http3-15 Section 3.1)。
    /// クライアント側では `enable_webtransport_client()` を使うこと。
    ///
    /// WebTransport 固有の設定 (ストリーム上限、データ上限、ドラフトバージョン等) は
    /// `webtransport::Settings` のビルダーメソッドで事前に構築して渡す。
    ///
    /// # バージョンネゴシエーション (draft-ietf-webtrans-http3-15 Section 7.1)
    ///
    /// - 各ドラフトバージョンは異なる SETTINGS_WT_ENABLED コードポイントを使用する
    /// - 複数バージョン対応時、各バージョンのコードポイントでそれぞれ送信する
    /// - サーバーはクライアントの SETTINGS を受信するまで WebTransport リクエストの処理を待つ
    ///
    /// 将来のドラフトで変更される可能性がある
    pub fn enable_webtransport_server(mut self, wt: webtransport::Settings) -> Self {
        self.enable_connect_protocol = Some(true);
        self.h3_datagram = Some(true);
        self.wt_settings = Some(wt);
        self
    }

    /// WebTransport をクライアント側で有効にする
    ///
    /// 以下の設定を自動的に行う:
    /// - SETTINGS_H3_DATAGRAM: 1 (クライアント送信項目)
    ///
    /// `SETTINGS_ENABLE_CONNECT_PROTOCOL` はサーバーが広告する設定であり、
    /// クライアントは送信しない
    /// (draft-ietf-webtrans-http3-15 Section 3.1 のクライアント送信項目リスト参照)。
    ///
    /// draft バージョンの場合は `SETTINGS_WT_ENABLED` もクライアントから送信する
    /// 必要がある (draft-ietf-webtrans-http3-15 Section 7.1 の MUST)。これは
    /// `wt` の中身で表現する。
    ///
    /// 将来のドラフトで変更される可能性がある
    pub fn enable_webtransport_client(mut self, wt: webtransport::Settings) -> Self {
        self.h3_datagram = Some(true);
        self.wt_settings = Some(wt);
        self
    }

    /// WebTransport が有効かどうか
    ///
    /// draft-02/07/15 のいずれかで有効になっていれば true
    pub fn is_webtransport_enabled(&self) -> bool {
        self.wt_settings.as_ref().is_some_and(|wt| wt.is_enabled())
    }

    /// WebTransport のドラフトパターンを返す
    ///
    /// `wt_settings` が `None` または WebTransport として解釈できない場合は `None`。
    pub fn webtransport_draft_pattern(&self) -> Option<webtransport::DraftVersion> {
        self.wt_settings
            .as_ref()
            .and_then(|wt| wt.detect_draft_pattern())
    }

    /// SettingsPayload から Settings を作成
    ///
    /// H3 設定と WebTransport 設定の両方をパースする。
    /// WebTransport 関連の ID は `webtransport::Settings::from_payload()` に委譲する。
    ///
    /// `SETTINGS_ENABLE_CONNECT_PROTOCOL` (0x08) と `SETTINGS_H3_DATAGRAM` (0x33) は
    /// 値が 0 または 1 でなければ `H3_SETTINGS_ERROR` を返す。
    /// (RFC 8441 Section 3, RFC 9297 Section 2.1.1)
    pub fn from_payload(
        payload: &crate::frame::SettingsPayload,
    ) -> Result<Self, crate::error::Error> {
        let mut settings = Self::new();
        for (id, value) in &payload.entries {
            match *id {
                0x01 => settings.qpack_max_table_capacity = Some(*value),
                0x06 => settings.max_field_section_size = Some(*value),
                0x07 => settings.qpack_blocked_streams = Some(*value),
                // SETTINGS_ENABLE_CONNECT_PROTOCOL: 値は 0 または 1 のみ
                // (RFC 8441 Section 3)
                0x08 => {
                    if *value > 1 {
                        return Err(crate::error::Error::ConnectionError(
                            crate::error::ErrorCode::SettingsError,
                        ));
                    }
                    settings.enable_connect_protocol = Some(*value == 1);
                }
                // SETTINGS_H3_DATAGRAM: 値は 0 または 1 のみ
                // (RFC 9297 Section 2.1.1)
                0x33 => {
                    if *value > 1 {
                        return Err(crate::error::Error::ConnectionError(
                            crate::error::ErrorCode::SettingsError,
                        ));
                    }
                    settings.h3_datagram = Some(*value == 1);
                }
                _ => {} // WebTransport 設定と不明な設定は個別処理
            }
        }
        // WebTransport 関連 ID をまとめてパース
        settings.wt_settings = webtransport::Settings::from_payload(payload)?;
        Ok(settings)
    }

    /// H3 設定エントリのイテレータを返す
    ///
    /// WebTransport 設定は含まない。WebTransport 設定は
    /// `wt_settings.iter()` で別途取得する。
    pub fn iter(&self) -> impl Iterator<Item = (u64, u64)> + '_ {
        let entries = [
            self.qpack_max_table_capacity
                .map(|v| (SettingsId::QpackMaxTableCapacity as u64, v)),
            self.max_field_section_size
                .map(|v| (SettingsId::MaxFieldSectionSize as u64, v)),
            self.qpack_blocked_streams
                .map(|v| (SettingsId::QpackBlockedStreams as u64, v)),
            self.enable_connect_protocol
                .map(|v| (SettingsId::EnableConnectProtocol as u64, u64::from(v))),
            self.h3_datagram
                .map(|v| (SettingsId::H3Datagram as u64, u64::from(v))),
        ];
        entries.into_iter().flatten()
    }

    /// 設定エントリの数を返す (WebTransport 設定を含む)
    pub fn len(&self) -> usize {
        let h3_count = self.iter().count();
        let wt_count = self
            .wt_settings
            .as_ref()
            .map(|wt| wt.iter().count())
            .unwrap_or(0);
        h3_count + wt_count
    }

    /// 設定が空かどうか
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_settings_id_from_id() {
        assert_eq!(
            SettingsId::from_id(0x01),
            Some(SettingsId::QpackMaxTableCapacity)
        );
        assert_eq!(
            SettingsId::from_id(0x06),
            Some(SettingsId::MaxFieldSectionSize)
        );
        assert_eq!(SettingsId::from_id(0x99), None);
    }

    #[test]
    fn test_settings_id_is_http2_only() {
        assert!(SettingsId::is_http2_only(0x02)); // ENABLE_PUSH
        assert!(SettingsId::is_http2_only(0x03)); // MAX_CONCURRENT_STREAMS
        assert!(SettingsId::is_http2_only(0x04)); // INITIAL_WINDOW_SIZE
        assert!(SettingsId::is_http2_only(0x05)); // MAX_FRAME_SIZE
        assert!(!SettingsId::is_http2_only(0x01));
        assert!(!SettingsId::is_http2_only(0x06));
    }

    #[test]
    fn test_settings_default() {
        let settings = Settings::default();
        assert!(settings.is_empty());
    }

    #[test]
    fn test_settings_builder() {
        let settings = Settings::new()
            .qpack_max_table_capacity(4096)
            .max_field_section_size(16384)
            .qpack_blocked_streams(100)
            .enable_connect_protocol(true)
            .h3_datagram(false);

        assert_eq!(settings.qpack_max_table_capacity, Some(4096));
        assert_eq!(settings.max_field_section_size, Some(16384));
        assert_eq!(settings.qpack_blocked_streams, Some(100));
        assert_eq!(settings.enable_connect_protocol, Some(true));
        assert_eq!(settings.h3_datagram, Some(false));
        assert_eq!(settings.len(), 5);
    }

    #[test]
    fn test_settings_from_limits() {
        let limits = Limits::new()
            .qpack_max_table_capacity(4096)
            .max_field_section_size(32768)
            .qpack_blocked_streams(50);

        let settings = Settings::from_limits(&limits);
        assert_eq!(settings.qpack_max_table_capacity, Some(4096));
        assert_eq!(settings.max_field_section_size, Some(32768));
        assert_eq!(settings.qpack_blocked_streams, Some(50));
    }

    #[test]
    fn test_settings_iter() {
        let settings = Settings::new()
            .qpack_max_table_capacity(4096)
            .max_field_section_size(16384);

        let entries: Vec<_> = settings.iter().collect();
        assert_eq!(entries.len(), 2);
        assert!(entries.contains(&(0x01, 4096)));
        assert!(entries.contains(&(0x06, 16384)));
    }

    #[test]
    fn test_enable_webtransport() {
        let wt = webtransport::Settings::new()
            .wt_enabled(1)
            .enable_webtransport_draft02(true)
            .webtransport_max_sessions_draft07(1)
            .wt_initial_max_streams_bidi(100)
            .wt_initial_max_streams_uni(100)
            .wt_initial_max_data(1048576);

        let settings = Settings::new().enable_webtransport_server(wt);

        assert_eq!(settings.enable_connect_protocol, Some(true));
        assert_eq!(settings.h3_datagram, Some(true));
        assert!(settings.is_webtransport_enabled());

        let wt = settings.wt_settings.unwrap();
        assert_eq!(wt.wt_enabled, 1);
        assert_eq!(wt.enable_webtransport_draft02, Some(true));
        assert_eq!(wt.webtransport_max_sessions_draft07, Some(1));
        assert_eq!(wt.wt_initial_max_streams_bidi, 100);
        assert_eq!(wt.wt_initial_max_streams_uni, 100);
        assert_eq!(wt.wt_initial_max_data, 1048576);
    }

    #[test]
    fn test_is_webtransport_enabled() {
        let settings = Settings::new();
        assert!(!settings.is_webtransport_enabled());

        let wt = webtransport::Settings::new().wt_enabled(1);
        let settings = Settings::new().enable_webtransport_server(wt);
        assert!(settings.is_webtransport_enabled());
    }

    #[test]
    fn test_len_includes_wt_settings() {
        let wt = webtransport::Settings::new()
            .wt_enabled(1)
            .wt_initial_max_streams_bidi(100);

        let settings = Settings::new()
            .qpack_max_table_capacity(4096)
            .enable_webtransport_server(wt);

        // H3: qpack_max_table_capacity, enable_connect_protocol, h3_datagram = 3
        // WT: wt_enabled, wt_initial_max_streams_bidi = 2
        assert_eq!(settings.len(), 5);
    }

    #[test]
    fn test_from_payload_boolean_settings_valid() {
        use crate::frame::SettingsPayload;

        // 0x08 = 0 は有効
        let mut payload = SettingsPayload::new();
        payload.add(0x08, 0);
        let settings = Settings::from_payload(&payload).unwrap();
        assert_eq!(settings.enable_connect_protocol, Some(false));

        // 0x08 = 1 は有効
        let mut payload = SettingsPayload::new();
        payload.add(0x08, 1);
        let settings = Settings::from_payload(&payload).unwrap();
        assert_eq!(settings.enable_connect_protocol, Some(true));

        // 0x33 = 0 は有効
        let mut payload = SettingsPayload::new();
        payload.add(0x33, 0);
        let settings = Settings::from_payload(&payload).unwrap();
        assert_eq!(settings.h3_datagram, Some(false));

        // 0x33 = 1 は有効
        let mut payload = SettingsPayload::new();
        payload.add(0x33, 1);
        let settings = Settings::from_payload(&payload).unwrap();
        assert_eq!(settings.h3_datagram, Some(true));
    }

    #[test]
    fn test_from_payload_boolean_settings_invalid() {
        use crate::error::{Error, ErrorCode};
        use crate::frame::SettingsPayload;

        // 0x08 = 2 は不正 (RFC 8441 Section 3)
        let mut payload = SettingsPayload::new();
        payload.add(0x08, 2);
        assert!(matches!(
            Settings::from_payload(&payload),
            Err(Error::ConnectionError(ErrorCode::SettingsError))
        ));

        // 0x33 = 2 は不正 (RFC 9297 Section 2.1.1)
        let mut payload = SettingsPayload::new();
        payload.add(0x33, 2);
        assert!(matches!(
            Settings::from_payload(&payload),
            Err(Error::ConnectionError(ErrorCode::SettingsError))
        ));
    }
}
