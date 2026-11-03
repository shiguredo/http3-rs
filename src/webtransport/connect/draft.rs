//! WebTransport ドラフトバージョン管理 connect/mod.rs からの分離
//!
//! ドラフトバージョンごとの SETTINGS 構築とプロトコルネゴシエーションを担う。
//! (draft-ietf-webtrans-http3-02/07/14/15/16/17 Section 3.1, 7.1)

use crate::varint::VarInt;

use super::ProtocolToken;

/// WebTransport over HTTP/3 のドラフト版の識別子
///
/// `:protocol` 疑似ヘッダーの値がドラフトバージョンによって異なる:
/// - draft-02, draft-07, draft-14: `webtransport`
/// - draft-15 / 16 / 17: `webtransport-h3`
///
/// SETTINGS のコードポイントと `:protocol` トークンで版を識別するための値であり、
/// 各ドラフト仕様全体への準拠を表すものではない。たとえば [`DraftVersion::Draft17`]
/// は draft-17 の SETTINGS コードポイントに対応する版を表し、draft-17 の全規定
/// (WT_STREAM のエラーコードなど) への準拠を意味しない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftVersion {
    /// draft-ietf-webtrans-http3-02
    /// `:protocol` = `"webtransport"`, SETTINGS_ENABLE_WEBTRANSPORT (0x2b603742)
    Draft02,
    /// draft-ietf-webtrans-http3-07
    /// `:protocol` = `"webtransport"`, SETTINGS_WEBTRANSPORT_MAX_SESSIONS (0xc671706a)
    Draft07,
    /// draft-ietf-webtrans-http3-14
    /// `:protocol` = `"webtransport"`, SETTINGS_WT_MAX_SESSIONS (0x14e9cd29)
    /// draft-07 のセッションネゴシエーションに加え、
    /// WT_INITIAL_MAX_STREAMS_UNI/BIDI (0x2b64/0x2b65) によるフロー制御を使用する。
    /// Safari 26.4 がこのパターンを使用する。
    Draft14,
    /// draft-ietf-webtrans-http3-15 / 16
    /// `:protocol` = `"webtransport-h3"`, SETTINGS_WT_ENABLED (0x2c7cf000)
    ///
    /// draft-15 と draft-16 は同じコードポイントを共有するため、本 variant に
    /// まとめる。
    Draft15,
    /// draft-ietf-webtrans-http3-17 (latest)
    /// `:protocol` = `"webtransport-h3"`, SETTINGS_WT_ENABLED (0x20b8933b)
    ///
    /// draft-15 / 16 とは `SETTINGS_WT_ENABLED` のコードポイントが異なり、
    /// `:protocol` は共通である。両者を区別できるのは SETTINGS の
    /// コードポイントだけなので、`SETTINGS` の受信内容で判別する
    /// (draft-ietf-webtrans-http3-17 Section 3.1 / 7.1 / 9.2)。
    Draft17,
}

impl DraftVersion {
    /// このドラフトバージョンに対応する `:protocol` 疑似ヘッダーの値を返す
    ///
    /// トークンの対応表は [`ProtocolToken`] に集約している。
    pub fn protocol_value(self) -> &'static str {
        ProtocolToken::from_draft_version(self).as_str()
    }

    /// 検出されたクライアントのドラフトバージョンに対応するサーバー SETTINGS を構築する
    ///
    /// クライアントの版ごとに応答 SETTINGS を組み立てる旧来の互換処理である。
    /// 接続で使う版を選択したうえで SETTINGS を広告する場合は
    /// `Settings::for_draft_versions` を使う。Safari には draft-07 の
    /// コードポイントだけを返す必要があり、その判断は呼び出し側の責務になる
    /// (`docs/SAFARI_WT.md`)。
    ///
    /// ドラフトバージョンに応じて適切な SETTINGS パラメータを設定する:
    ///
    /// - **Draft17**: `SETTINGS_WT_ENABLED` (0x20b8933b) + 初期ストリーム上限
    /// - **Draft15** (draft-15 / 16): `SETTINGS_WT_ENABLED` (0x2c7cf000) + 初期ストリーム上限
    /// - **Draft14**: safari 以外の draft-14 クライアント向けに draft-07 と draft-14 の
    ///   **両方** の SETTINGS ID を返す。Safari 26.4 は draft-07 と draft-14 の ID を
    ///   併送するが、応答 SETTINGS に draft-14 の ID を含めると CONNECT を拒否するため、
    ///   Safari に応答する場合は draft-07 を使い、この関数を使わない
    ///   (`docs/SAFARI_WT.md`)。カプセルベースフロー制御用に初期ストリーム上限と
    ///   初期データ上限も設定する。
    /// - **Draft07**: `SETTINGS_WEBTRANSPORT_MAX_SESSIONS` (draft-07) のみ
    /// - **Draft02**: `SETTINGS_ENABLE_WEBTRANSPORT` のみ
    ///
    /// draft-ietf-webtrans-http3-14, draft-ietf-webtrans-http3-15,
    /// draft-ietf-webtrans-http3-16, draft-ietf-webtrans-http3-17
    /// 将来のドラフトで変更される可能性がある
    pub fn build_server_settings(
        self,
        params: &ServerSettingsParams,
    ) -> crate::webtransport::settings::Settings {
        match self {
            Self::Draft17 => crate::webtransport::settings::Settings::new()
                .wt_enabled_draft17(VarInt::from_static(1))
                .wt_initial_max_streams_uni(params.initial_max_streams_uni)
                .wt_initial_max_streams_bidi(params.initial_max_streams_bidi)
                .wt_initial_max_data(params.initial_max_data),
            Self::Draft15 => crate::webtransport::settings::Settings::new()
                .wt_enabled(VarInt::from_static(1))
                .wt_initial_max_streams_uni(params.initial_max_streams_uni)
                .wt_initial_max_streams_bidi(params.initial_max_streams_bidi)
                .wt_initial_max_data(params.initial_max_data),
            // Safari 26.4 (Network.framework) は 0xc671706a (draft-07) と 0x14e9cd29
            // (draft-14) の両方を送るが、サーバーの SETTINGS に
            // SETTINGS_WT_INITIAL_MAX_STREAMS_* / SETTINGS_WT_INITIAL_MAX_DATA を
            // 含めると H3_REQUEST_CANCELLED (0x10C) で拒否する。初期フロー制御値は
            // セッション確立直後に WT_MAX_STREAMS / WT_MAX_DATA カプセル
            // (draft-14 Section 5) で通知する。
            // サーバーも max_sessions を両方の ID で返すことで、どちらで判定しても
            // WebTransport 対応と認識させる。
            Self::Draft14 => crate::webtransport::settings::Settings::new()
                .wt_max_sessions_draft14(params.max_sessions)
                .webtransport_max_sessions_draft07(params.max_sessions),
            Self::Draft07 => crate::webtransport::settings::Settings::new()
                .webtransport_max_sessions_draft07(params.max_sessions),
            Self::Draft02 => {
                crate::webtransport::settings::Settings::new().enable_webtransport_draft02(true)
            }
        }
    }

    /// このドラフトバージョンがセッション確立直後の初期フロー制御カプセル送信を必要とするか
    ///
    /// カプセルベースのフロー制御は draft-14 Section 5 で導入された。
    /// Safari (Network.framework) のように draft-07 の ID も併送するクライアントは
    /// `Settings::detect_draft_pattern()` の時点で draft-07 として判定される。
    ///
    /// - draft-15 / 16 / 17: SETTINGS_WT_INITIAL_MAX_STREAMS で初期上限を通知するため不要
    /// - draft-14: 必要
    /// - draft-07: フロー制御の仕組みがないため不要
    /// - draft-02: フロー制御の仕組みがないため不要
    ///
    /// 厳密には draft-14 Section 5.1 の宣言条件 (双方の `declares_flow_control()`)
    /// が成立した場合にのみカプセルを送るべき。呼び出し側は
    /// `Settings::flow_control_enabled_with_peer()` と併用すること。
    ///
    /// draft-ietf-webtrans-http3-14 Section 5
    /// 将来のドラフトで変更される可能性がある
    pub fn requires_initial_capsule_flow_control(self) -> bool {
        matches!(self, Self::Draft14)
    }

    /// このドラフトバージョンで `SETTINGS_ENABLE_CONNECT_PROTOCOL=1` が
    /// **サーバーから** 送られていることをクライアントが要求するか
    ///
    /// - draft-02: 不要 (`SETTINGS_ENABLE_WEBTRANSPORT` が拡張 CONNECT を暗示)
    /// - draft-07: 必要 (Section 3.2)
    /// - draft-14: 必要 (Section 3.1)
    /// - draft-15 / 16 / 17: 必要 (Section 3.1)
    ///
    /// サーバー側はクライアントからの `ENABLE_CONNECT_PROTOCOL` を要求しない
    /// (draft-07 のみ仕様上両端が送ることになっているが、本実装では互換性を
    /// 優先して検証しない)。
    ///
    /// draft-ietf-webtrans-http3-02 Section 3.1,
    /// draft-ietf-webtrans-http3-07 Section 3.2,
    /// draft-ietf-webtrans-http3-14 Section 3.1,
    /// draft-ietf-webtrans-http3-15 Section 3.1,
    /// draft-ietf-webtrans-http3-16 Section 3.1,
    /// draft-ietf-webtrans-http3-17 Section 3.1
    /// 将来のドラフトで変更される可能性がある
    pub fn requires_enable_connect_protocol(self) -> bool {
        matches!(
            self,
            Self::Draft07 | Self::Draft14 | Self::Draft15 | Self::Draft17
        )
    }

    /// このドラフトバージョンで `reset_stream_at` transport parameter が必須か
    ///
    /// - draft-02: 不要
    /// - draft-07: 不要
    /// - draft-14: 必要 (Section 3.1)
    /// - draft-15 / 16 / 17: 必要 (Section 3.1)
    ///
    /// draft-ietf-webtrans-http3-14 Section 3.1,
    /// draft-ietf-webtrans-http3-15 Section 3.1,
    /// draft-ietf-webtrans-http3-16 Section 3.1,
    /// draft-ietf-webtrans-http3-17 Section 3.1
    /// 将来のドラフトで変更される可能性がある
    pub fn requires_reset_stream_at(self) -> bool {
        matches!(self, Self::Draft14 | Self::Draft15 | Self::Draft17)
    }

    /// 指定ドラフトバージョンのクライアントが送るべき WebTransport SETTINGS を構築する
    ///
    /// 各ドラフトの `Clients supporting WebTransport over HTTP/3 send:` 節に基づく。
    /// `max_datagram_frame_size` / `reset_stream_at` は transport parameter の
    /// 責務なので含まれない。`SETTINGS_H3_DATAGRAM=1` は `crate::Settings` 側で
    /// 設定する。
    ///
    /// - **Draft17** (Section 3.1 + 7.1): `SETTINGS_WT_ENABLED` (0x20b8933b) + 初期ストリーム/データ上限
    /// - **Draft15** (draft-15 / 16, Section 3.1 + 7.1):
    ///   `SETTINGS_WT_ENABLED` (0x2c7cf000) + 初期ストリーム/データ上限
    /// - **Draft14** (Section 3.1): `SETTINGS_WT_MAX_SESSIONS > 0` + 初期ストリーム/データ上限
    /// - **Draft07** (Section 3.2): `SETTINGS_WEBTRANSPORT_MAX_SESSIONS > 0`
    /// - **Draft02** (Section 3.1): `SETTINGS_ENABLE_WEBTRANSPORT=1`
    ///
    /// draft-ietf-webtrans-http3-02 Section 3.1,
    /// draft-ietf-webtrans-http3-07 Section 3.2,
    /// draft-ietf-webtrans-http3-14 Section 3.1,
    /// draft-ietf-webtrans-http3-15 Section 3.1,
    /// draft-ietf-webtrans-http3-16 Section 3.1,
    /// draft-ietf-webtrans-http3-17 Section 3.1
    /// 将来のドラフトで変更される可能性がある
    pub fn build_client_settings(
        self,
        params: &ServerSettingsParams,
    ) -> crate::webtransport::settings::Settings {
        match self {
            Self::Draft17 => crate::webtransport::settings::Settings::new()
                .wt_enabled_draft17(VarInt::from_static(1))
                .wt_initial_max_streams_uni(params.initial_max_streams_uni)
                .wt_initial_max_streams_bidi(params.initial_max_streams_bidi)
                .wt_initial_max_data(params.initial_max_data),
            Self::Draft15 => crate::webtransport::settings::Settings::new()
                .wt_enabled(VarInt::from_static(1))
                .wt_initial_max_streams_uni(params.initial_max_streams_uni)
                .wt_initial_max_streams_bidi(params.initial_max_streams_bidi)
                .wt_initial_max_data(params.initial_max_data),
            Self::Draft14 => crate::webtransport::settings::Settings::new()
                .wt_max_sessions_draft14(params.max_sessions)
                .wt_initial_max_streams_uni(params.initial_max_streams_uni)
                .wt_initial_max_streams_bidi(params.initial_max_streams_bidi)
                .wt_initial_max_data(params.initial_max_data),
            Self::Draft07 => crate::webtransport::settings::Settings::new()
                .webtransport_max_sessions_draft07(params.max_sessions),
            Self::Draft02 => {
                crate::webtransport::settings::Settings::new().enable_webtransport_draft02(true)
            }
        }
    }
}

/// WebTransport SETTINGS 構築用のパラメータ
///
/// [`DraftVersion::build_server_settings()`] および
/// [`DraftVersion::build_client_settings()`] で使用する。
/// ドラフトバージョンに応じて必要なパラメータのみが反映される。
/// 各フィールドは VarInt (RFC 9000 §16) に制約される。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerSettingsParams {
    /// 最大セッション数 (draft-07/14 で使用)
    pub max_sessions: VarInt,
    /// 初期単方向ストリーム上限 (draft-14/15/16/17 で使用)
    pub initial_max_streams_uni: VarInt,
    /// 初期双方向ストリーム上限 (draft-14/15/16/17 で使用)
    pub initial_max_streams_bidi: VarInt,
    /// 初期データ上限 (draft-14/15/16/17 で使用)
    pub initial_max_data: VarInt,
}

impl Default for ServerSettingsParams {
    fn default() -> Self {
        Self {
            max_sessions: VarInt::from_static(1),
            initial_max_streams_uni: VarInt::from_static(100),
            initial_max_streams_bidi: VarInt::from_static(100),
            initial_max_data: VarInt::from_static(8 * 1024 * 1024),
        }
    }
}
