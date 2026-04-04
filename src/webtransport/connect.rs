//! WebTransport CONNECT リクエスト/レスポンス (draft-ietf-webtrans-http3-15 Section 3)
//!
//! 拡張 CONNECT (RFC 8441, RFC 9220) を使用した WebTransport セッションの
//! 確立リクエストのバリデーションとプロトコルネゴシエーションを提供。
//!
//! # 参照
//!
//! - RFC 8441: Bootstrapping WebSockets with HTTP/2 (拡張 CONNECT の定義)
//! - RFC 9220: Bootstrapping WebSockets with HTTP/3 (HTTP/3 への適用)
//! - draft-ietf-webtrans-http3-15 Section 3.2: Creating a New Session
//! - draft-ietf-webtrans-http3-15 Section 3.3: Application Protocol Negotiation

use core::fmt;

use crate::qpack::Header;

/// `:protocol` 疑似ヘッダーの値 (draft-ietf-webtrans-http3-15 Section 3.2)
///
/// draft-15 で定義された native QUIC モードのプロトコル識別子。
pub const PROTOCOL_WEBTRANSPORT_H3: &str = "webtransport-h3";

/// `:protocol` 疑似ヘッダーの値 (draft-ietf-webtrans-http3-02 Section 3.2)
///
/// draft-02 で定義されたプロトコル識別子。Chrome 等の実装が draft-02 互換で
/// この値を送信する場合がある。将来のドラフトで廃止される可能性がある。
pub const PROTOCOL_WEBTRANSPORT_DRAFT02: &str = "webtransport";

/// WebTransport ドラフトバージョン
///
/// `:protocol` 疑似ヘッダーの値がドラフトバージョンによって異なる:
/// - draft-02, draft-07, draft-14: `webtransport`
/// - draft-15 (latest): `webtransport-h3`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftVersion {
    /// draft-ietf-webtrans-http3-02
    /// `:protocol` = `"webtransport"`, SETTINGS_ENABLE_WEBTRANSPORT (0x2b603742)
    Draft02,
    /// draft-ietf-webtrans-http3-07
    /// `:protocol` = `"webtransport"`, SETTINGS_WEBTRANSPORT_MAX_SESSIONS (0xc671706a)
    Draft07,
    /// draft-ietf-webtrans-http3-14
    /// `:protocol` = `"webtransport"`, SETTINGS_WEBTRANSPORT_MAX_SESSIONS (0xc671706a)
    /// draft-07 のセッションネゴシエーションに加え、
    /// WT_INITIAL_MAX_STREAMS_UNI/BIDI (0x2b64/0x2b65) によるフロー制御を使用する。
    /// Safari 26.4 がこのパターンを使用する。
    Draft14,
    /// draft-ietf-webtrans-http3-15 (latest)
    /// `:protocol` = `"webtransport-h3"`, SETTINGS_WT_ENABLED (0x2c7cf000)
    Draft15,
}

/// WebTransport SETTINGS 構築用のパラメータ
///
/// `DraftVersion::build_server_settings()` および
/// `DraftVersion::build_client_settings()` で使用する。
/// ドラフトバージョンに応じて必要なパラメータのみが反映される。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerSettingsParams {
    /// 最大セッション数 (draft-07/14 で使用)
    pub max_sessions: u64,
    /// 初期単方向ストリーム上限 (draft-14/15 で使用)
    pub initial_max_streams_uni: u64,
    /// 初期双方向ストリーム上限 (draft-14/15 で使用)
    pub initial_max_streams_bidi: u64,
    /// 初期データ上限 (draft-14/15 で使用)
    pub initial_max_data: u64,
}

impl Default for ServerSettingsParams {
    fn default() -> Self {
        Self {
            max_sessions: 1,
            initial_max_streams_uni: 100,
            initial_max_streams_bidi: 100,
            initial_max_data: 8 * 1024 * 1024,
        }
    }
}

impl DraftVersion {
    /// このドラフトバージョンに対応する `:protocol` 疑似ヘッダーの値を返す
    pub fn protocol_value(&self) -> &'static str {
        match self {
            Self::Draft02 | Self::Draft07 | Self::Draft14 => PROTOCOL_WEBTRANSPORT_DRAFT02,
            Self::Draft15 => PROTOCOL_WEBTRANSPORT_H3,
        }
    }

    /// 検出されたクライアントのドラフトバージョンに対応するサーバー SETTINGS を構築する
    ///
    /// ドラフトバージョンに応じて適切な SETTINGS パラメータを設定する:
    ///
    /// - **Draft15**: `SETTINGS_WT_ENABLED` + 初期ストリーム上限
    /// - **Draft14**: Safari (Network.framework) 互換のため draft-07 と draft-14 の
    ///   **両方** の SETTINGS ID を返す。Safari 26.4 はどちらの ID で判定するか不定のため、
    ///   両方返すことで WebTransport 対応と認識させる。カプセルベースフロー制御用に
    ///   初期ストリーム上限と初期データ上限も設定する。
    /// - **Draft07**: `SETTINGS_WEBTRANSPORT_MAX_SESSIONS` (draft-07) のみ
    /// - **Draft02**: `SETTINGS_ENABLE_WEBTRANSPORT` のみ
    ///
    /// draft-ietf-webtrans-http3-14, draft-ietf-webtrans-http3-15
    /// 将来のドラフトで変更される可能性がある
    pub fn build_server_settings(
        &self,
        params: &ServerSettingsParams,
    ) -> super::settings::Settings {
        match self {
            Self::Draft15 => super::settings::Settings::new()
                .wt_enabled(1)
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
            Self::Draft14 => super::settings::Settings::new()
                .wt_max_sessions_draft14(params.max_sessions)
                .webtransport_max_sessions_draft07(params.max_sessions),
            Self::Draft07 => super::settings::Settings::new()
                .webtransport_max_sessions_draft07(params.max_sessions),
            Self::Draft02 => super::settings::Settings::new().enable_webtransport_draft02(true),
        }
    }

    /// このドラフトバージョンがセッション確立直後の初期フロー制御カプセル送信を必要とするか
    ///
    /// カプセルベースのフロー制御は draft-14 Section 5 で導入された。
    /// Safari (Network.framework) のように draft-07 の ID も併送するクライアントは
    /// `Settings::detect_draft_pattern()` の時点で draft-14 として判定される。
    ///
    /// - draft-15: SETTINGS_WT_INITIAL_MAX_STREAMS で初期上限を通知するため不要
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
    pub fn requires_initial_capsule_flow_control(&self) -> bool {
        matches!(self, Self::Draft14)
    }

    /// このドラフトバージョンで `SETTINGS_ENABLE_CONNECT_PROTOCOL=1` が
    /// **サーバーから** 送られていることをクライアントが要求するか
    ///
    /// - draft-02: 不要 (`SETTINGS_ENABLE_WEBTRANSPORT` が拡張 CONNECT を暗示)
    /// - draft-07: 必要 (Section 3.2)
    /// - draft-14: 必要 (Section 3.1)
    /// - draft-15: 必要 (Section 3.1)
    ///
    /// サーバー側はクライアントからの `ENABLE_CONNECT_PROTOCOL` を要求しない
    /// (draft-07 のみ仕様上両端が送ることになっているが、本実装では互換性を
    /// 優先して検証しない)。
    ///
    /// draft-ietf-webtrans-http3-02 Section 3.1,
    /// draft-ietf-webtrans-http3-07 Section 3.2,
    /// draft-ietf-webtrans-http3-14 Section 3.1,
    /// draft-ietf-webtrans-http3-15 Section 3.1
    /// 将来のドラフトで変更される可能性がある
    pub fn requires_enable_connect_protocol(&self) -> bool {
        matches!(self, Self::Draft07 | Self::Draft14 | Self::Draft15)
    }

    /// このドラフトバージョンで `reset_stream_at` transport parameter が必須か
    ///
    /// - draft-02: 不要
    /// - draft-07: 不要
    /// - draft-14: 必要 (Section 3.1)
    /// - draft-15: 必要 (Section 3.1)
    ///
    /// draft-ietf-webtrans-http3-14 Section 3.1,
    /// draft-ietf-webtrans-http3-15 Section 3.1
    /// 将来のドラフトで変更される可能性がある
    pub fn requires_reset_stream_at(&self) -> bool {
        matches!(self, Self::Draft14 | Self::Draft15)
    }

    /// 指定ドラフトバージョンのクライアントが送るべき WebTransport SETTINGS を構築する
    ///
    /// 各ドラフトの `Clients supporting WebTransport over HTTP/3 send:` 節に基づく。
    /// `max_datagram_frame_size` / `reset_stream_at` は transport parameter の
    /// 責務なので含まれない。`SETTINGS_H3_DATAGRAM=1` は `crate::Settings` 側で
    /// 設定する。
    ///
    /// - **Draft15** (Section 3.1 + 7.1): `SETTINGS_WT_ENABLED=1` + 初期ストリーム/データ上限
    /// - **Draft14** (Section 3.1): `SETTINGS_WT_MAX_SESSIONS > 0` + 初期ストリーム/データ上限
    /// - **Draft07** (Section 3.2): `SETTINGS_WEBTRANSPORT_MAX_SESSIONS > 0`
    /// - **Draft02** (Section 3.1): `SETTINGS_ENABLE_WEBTRANSPORT=1`
    ///
    /// draft-ietf-webtrans-http3-02 Section 3.1,
    /// draft-ietf-webtrans-http3-07 Section 3.2,
    /// draft-ietf-webtrans-http3-14 Section 3.1,
    /// draft-ietf-webtrans-http3-15 Section 3.1
    /// 将来のドラフトで変更される可能性がある
    pub fn build_client_settings(
        &self,
        params: &ServerSettingsParams,
    ) -> super::settings::Settings {
        match self {
            Self::Draft15 => super::settings::Settings::new()
                .wt_enabled(1)
                .wt_initial_max_streams_uni(params.initial_max_streams_uni)
                .wt_initial_max_streams_bidi(params.initial_max_streams_bidi)
                .wt_initial_max_data(params.initial_max_data),
            Self::Draft14 => super::settings::Settings::new()
                .wt_max_sessions_draft14(params.max_sessions)
                .wt_initial_max_streams_uni(params.initial_max_streams_uni)
                .wt_initial_max_streams_bidi(params.initial_max_streams_bidi)
                .wt_initial_max_data(params.initial_max_data),
            Self::Draft07 => super::settings::Settings::new()
                .webtransport_max_sessions_draft07(params.max_sessions),
            Self::Draft02 => super::settings::Settings::new().enable_webtransport_draft02(true),
        }
    }
}

/// CONNECT リクエスト検証エラー (RFC Section 3.2)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectError {
    /// `:scheme` が `https` でない
    InvalidScheme,
    /// `:authority` が欠落している
    MissingAuthority,
    /// `:path` が欠落している
    MissingPath,
    /// `:method` が `CONNECT` でない
    InvalidMethod,
    /// `:protocol` が `webtransport-h3` または `webtransport` でない
    InvalidProtocol,
    /// ヘッダー値が不正な UTF-8
    InvalidEncoding,
}

impl fmt::Display for ConnectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidScheme => write!(f, ":scheme must be \"https\""),
            Self::MissingAuthority => write!(f, ":authority is required"),
            Self::MissingPath => write!(f, ":path is required"),
            Self::InvalidMethod => write!(f, ":method must be \"CONNECT\""),
            Self::InvalidProtocol => {
                write!(
                    f,
                    ":protocol must be \"{}\" or \"{}\"",
                    PROTOCOL_WEBTRANSPORT_H3, PROTOCOL_WEBTRANSPORT_DRAFT02
                )
            }
            Self::InvalidEncoding => write!(f, "header value is not valid UTF-8"),
        }
    }
}

/// WebTransport セッション開始前提の検証エラー (draft-ietf-webtrans-http3-15 Section 3.1)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityError {
    /// SETTINGS_WT_ENABLED (> 0) が確認できない
    MissingWebTransportSetting,
    /// SETTINGS_ENABLE_CONNECT_PROTOCOL (= 1) が確認できない
    MissingEnableConnectProtocol,
    /// SETTINGS_H3_DATAGRAM (= 1) が確認できない
    MissingH3Datagram,
    /// max_datagram_frame_size (> 0) が確認できない
    MissingQuicDatagram,
    /// reset_stream_at transport parameter が確認できない
    MissingResetStreamAt,
}

impl fmt::Display for CapabilityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingWebTransportSetting => {
                write!(f, "SETTINGS_WT_ENABLED with value > 0 is required")
            }
            Self::MissingEnableConnectProtocol => {
                write!(
                    f,
                    "SETTINGS_ENABLE_CONNECT_PROTOCOL with value 1 is required"
                )
            }
            Self::MissingH3Datagram => write!(f, "SETTINGS_H3_DATAGRAM with value 1 is required"),
            Self::MissingQuicDatagram => {
                write!(
                    f,
                    "max_datagram_frame_size transport parameter > 0 is required"
                )
            }
            Self::MissingResetStreamAt => {
                write!(f, "reset_stream_at transport parameter is required")
            }
        }
    }
}

/// WebTransport 前提機能のネゴシエーション結果
///
/// 呼び出し側が SETTINGS / transport parameters から値を埋めて検証する。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TransportCapabilities {
    /// SETTINGS_WT_ENABLED が 0 より大きいか
    pub wt_enabled: bool,
    /// SETTINGS_ENABLE_CONNECT_PROTOCOL が 1 か
    pub enable_connect_protocol: bool,
    /// SETTINGS_H3_DATAGRAM が 1 か
    pub h3_datagram: bool,
    /// max_datagram_frame_size transport parameter が 0 より大きいか
    pub max_datagram_frame_size: bool,
    /// reset_stream_at transport parameter が存在するか
    pub reset_stream_at: bool,
}

impl TransportCapabilities {
    /// 新しい前提機能セットを作成
    pub const fn new() -> Self {
        Self {
            wt_enabled: false,
            enable_connect_protocol: false,
            h3_datagram: false,
            max_datagram_frame_size: false,
            reset_stream_at: false,
        }
    }

    /// WebTransport セッション開始前提を検証
    pub fn validate(&self) -> Result<(), CapabilityError> {
        if !self.wt_enabled {
            return Err(CapabilityError::MissingWebTransportSetting);
        }
        if !self.enable_connect_protocol {
            return Err(CapabilityError::MissingEnableConnectProtocol);
        }
        if !self.h3_datagram {
            return Err(CapabilityError::MissingH3Datagram);
        }
        if !self.max_datagram_frame_size {
            return Err(CapabilityError::MissingQuicDatagram);
        }
        if !self.reset_stream_at {
            return Err(CapabilityError::MissingResetStreamAt);
        }
        Ok(())
    }
}

/// WebTransport CONNECT リクエスト (RFC Section 3.2)
///
/// クライアントが新しい WebTransport セッションを確立するための拡張 CONNECT リクエスト。
/// `:protocol` ヘッダーの値はドラフトバージョンに依存する。
///
/// # 必須疑似ヘッダー
///
/// - `:method` = `CONNECT` (拡張 CONNECT)
/// - `:protocol` = ドラフトバージョンに依存 (`webtransport` or `webtransport-h3`)
/// - `:scheme` = `https`
/// - `:authority` = サーバーリソースの識別子
/// - `:path` = サーバーリソースのパス
///
/// # 任意ヘッダー
///
/// - `Origin` = クライアントオリジン (ブラウザクライアントの場合は MUST)
/// - `WT-Available-Protocols` = 利用可能なアプリケーションプロトコルリスト
#[derive(Debug, Clone)]
pub struct ConnectRequest {
    /// ドラフトバージョン (デフォルト: Draft15)
    pub draft_version: DraftVersion,
    /// `:scheme` (MUST be `https` - RFC Section 3.2)
    pub scheme: String,
    /// `:authority` (MUST be present - RFC Section 3.2)
    pub authority: String,
    /// `:path` (MUST be present - RFC Section 3.2)
    pub path: String,
    /// `Origin` ヘッダー (ブラウザクライアントの場合は MUST - RFC Section 3.2)
    pub origin: Option<String>,
    /// `WT-Available-Protocols` ヘッダーから解析したプロトコルリスト (RFC Section 3.3)
    ///
    /// 優先度順で列挙する。空の場合はプロトコルネゴシエーションなし。
    pub available_protocols: Vec<String>,
}

impl ConnectRequest {
    /// 新しい CONNECT リクエストを作成 (デフォルト: Draft15)
    pub fn new(
        scheme: impl Into<String>,
        authority: impl Into<String>,
        path: impl Into<String>,
    ) -> Self {
        Self {
            draft_version: DraftVersion::Draft15,
            scheme: scheme.into(),
            authority: authority.into(),
            path: path.into(),
            origin: None,
            available_protocols: Vec::new(),
        }
    }

    /// ドラフトバージョンを設定
    pub fn draft_version(mut self, version: DraftVersion) -> Self {
        self.draft_version = version;
        self
    }

    /// `Origin` ヘッダーを設定
    pub fn origin(mut self, origin: impl Into<String>) -> Self {
        self.origin = Some(origin.into());
        self
    }

    /// `WT-Available-Protocols` を設定
    pub fn available_protocols(mut self, protocols: Vec<String>) -> Self {
        self.available_protocols = protocols;
        self
    }

    /// ヘッダーペアから CONNECT リクエストを構築する
    ///
    /// `Event::Header` で受信したヘッダーを集めて渡す。
    /// パースのみ行い、RFC 準拠の検証は `validate()` で別途行う。
    ///
    /// # エラー
    ///
    /// - `ConnectError::InvalidMethod`: `:method` が `CONNECT` でない
    /// - `ConnectError::InvalidProtocol`: `:protocol` が `webtransport-h3` / `webtransport` でない
    /// - `ConnectError::InvalidEncoding`: ヘッダー値が不正な UTF-8
    pub fn from_headers(headers: &[(&[u8], &[u8])]) -> Result<Self, ConnectError> {
        let mut method: Option<&[u8]> = None;
        let mut protocol: Option<&[u8]> = None;
        let mut scheme: Option<String> = None;
        let mut authority: Option<String> = None;
        let mut path: Option<String> = None;
        let mut origin: Option<String> = None;
        let mut wt_available_protocols: Option<String> = None;

        for &(name, value) in headers {
            match name {
                b":method" => method = Some(value),
                b":protocol" => protocol = Some(value),
                b":scheme" => {
                    scheme = Some(
                        core::str::from_utf8(value)
                            .map_err(|_| ConnectError::InvalidEncoding)?
                            .to_string(),
                    );
                }
                b":authority" => {
                    authority = Some(
                        core::str::from_utf8(value)
                            .map_err(|_| ConnectError::InvalidEncoding)?
                            .to_string(),
                    );
                }
                b":path" => {
                    path = Some(
                        core::str::from_utf8(value)
                            .map_err(|_| ConnectError::InvalidEncoding)?
                            .to_string(),
                    );
                }
                b"origin" => {
                    origin = Some(
                        core::str::from_utf8(value)
                            .map_err(|_| ConnectError::InvalidEncoding)?
                            .to_string(),
                    );
                }
                b"wt-available-protocols" => {
                    wt_available_protocols = Some(
                        core::str::from_utf8(value)
                            .map_err(|_| ConnectError::InvalidEncoding)?
                            .to_string(),
                    );
                }
                _ => {}
            }
        }

        // :method の検証
        match method {
            Some(b"CONNECT") => {}
            Some(_) => return Err(ConnectError::InvalidMethod),
            None => return Err(ConnectError::InvalidMethod),
        }

        // :protocol の検証とドラフトバージョンの判定
        let draft_version = match protocol {
            Some(p) => {
                let p_str = core::str::from_utf8(p).map_err(|_| ConnectError::InvalidEncoding)?;
                if p_str == PROTOCOL_WEBTRANSPORT_H3 {
                    DraftVersion::Draft15
                } else if p_str == PROTOCOL_WEBTRANSPORT_DRAFT02 {
                    // draft-02 と draft-07 は同じ `:protocol` 値を使用するため
                    // ヘッダーだけでは区別できない。draft-02 として扱う。
                    DraftVersion::Draft02
                } else {
                    return Err(ConnectError::InvalidProtocol);
                }
            }
            None => return Err(ConnectError::InvalidProtocol),
        };

        let mut req = Self {
            draft_version,
            scheme: scheme.unwrap_or_default(),
            authority: authority.unwrap_or_default(),
            path: path.unwrap_or_default(),
            origin,
            available_protocols: Vec::new(),
        };

        if let Some(ref wt_ap) = wt_available_protocols {
            req.available_protocols = Self::parse_available_protocols(wt_ap);
        }

        Ok(req)
    }

    /// CONNECT リクエストのヘッダー配列を生成する
    ///
    /// `:protocol` の値はドラフトバージョンに依存する。
    pub fn to_headers(&self) -> Vec<Header> {
        let mut headers = vec![
            Header::new(b":method", b"CONNECT"),
            Header::new(b":protocol", self.draft_version.protocol_value().as_bytes()),
            Header::new(b":scheme", self.scheme.as_bytes()),
            Header::new(b":authority", self.authority.as_bytes()),
            Header::new(b":path", self.path.as_bytes()),
        ];

        if let Some(ref origin) = self.origin {
            headers.push(Header::new(b"origin", origin.as_bytes()));
        }

        if !self.available_protocols.is_empty() {
            let value = self
                .available_protocols
                .iter()
                .map(|p| format!("\"{}\"", p.replace('\\', "\\\\").replace('"', "\\\"")))
                .collect::<Vec<_>>()
                .join(", ");
            headers.push(Header::new(b"wt-available-protocols", value.as_bytes()));
        }

        headers
    }

    /// RFC Section 3.2 に従いリクエストを検証
    ///
    /// 以下を検証する:
    /// - `:scheme` が `https` であること
    /// - `:authority` が空でないこと
    /// - `:path` が空でないこと
    ///
    /// `:protocol` が `webtransport-h3` であることは呼び出し元が事前に確認する。
    pub fn validate(&self) -> Result<(), ConnectError> {
        if self.scheme != "https" {
            return Err(ConnectError::InvalidScheme);
        }
        if self.authority.is_empty() {
            return Err(ConnectError::MissingAuthority);
        }
        if self.path.is_empty() {
            return Err(ConnectError::MissingPath);
        }
        Ok(())
    }

    /// `WT-Available-Protocols` ヘッダー値を解析 (RFC Section 3.3)
    ///
    /// Structured Fields List 形式 (RFC 9651) から文字列型のアイテムのみを抽出する。
    /// 文字列型以外のアイテムはエラーとして無視する (RFC Section 3.3)。
    /// パラメータ (`;` 以降) は無視する (RFC Section 3.3)。
    pub fn parse_available_protocols(header_value: &str) -> Vec<String> {
        parse_sf_list_strings(header_value)
    }
}

/// WebTransport CONNECT レスポンス (RFC Section 3.2)
///
/// サーバーが CONNECT リクエストに対して返すレスポンス。
/// 2xx ステータスコードでセッション確立成功。
/// クライアントは 3xx リダイレクトを自動追従してはならない (MUST NOT)。
#[derive(Debug, Clone)]
pub struct ConnectResponse {
    /// HTTP ステータスコード
    pub status: u16,
    /// `WT-Protocol` ヘッダーで選択されたプロトコル (RFC Section 3.3)
    pub selected_protocol: Option<String>,
}

impl ConnectResponse {
    /// 新しい CONNECT レスポンスを作成
    pub fn new(status: u16) -> Self {
        Self {
            status,
            selected_protocol: None,
        }
    }

    /// 選択されたプロトコルを設定
    pub fn with_protocol(mut self, protocol: impl Into<String>) -> Self {
        self.selected_protocol = Some(protocol.into());
        self
    }

    /// CONNECT レスポンスのヘッダー配列を生成する
    pub fn to_headers(&self) -> Vec<Header> {
        let mut headers = vec![Header::new(b":status", self.status.to_string().as_bytes())];

        if let Some(ref proto) = self.selected_protocol {
            headers.push(Header::new(
                b"wt-protocol",
                format!("\"{}\"", proto.replace('\\', "\\\\").replace('"', "\\\"")).as_bytes(),
            ));
        }

        headers
    }

    /// セッション確立成功かどうか (2xx ステータスコード)
    pub fn is_success(&self) -> bool {
        self.status / 100 == 2
    }

    /// `WT-Protocol` の検証 (draft-ietf-webtrans-http3-15 Section 3.3)
    ///
    /// レスポンスの `WT-Protocol` がリクエストの `WT-Available-Protocols` に
    /// 含まれているかを確認する。
    ///
    /// - リクエストに `WT-Available-Protocols` があり、レスポンスに `WT-Protocol` がない場合: `false`
    ///   (クライアントは WT_ALPN_ERROR でセッションを閉鎖する MUST)
    /// - リクエストに `WT-Available-Protocols` がなく、レスポンスに `WT-Protocol` がある場合: `false`
    /// - `WT-Protocol` が `WT-Available-Protocols` に含まれていない場合: `false`
    ///   (クライアントは WT_ALPN_ERROR でセッションを閉鎖する MUST)
    /// - リクエストに `WT-Available-Protocols` がなく、レスポンスに `WT-Protocol` もない場合: `true`
    /// - `WT-Protocol` が `WT-Available-Protocols` に含まれている場合: `true`
    ///
    /// 将来のドラフトで変更される可能性がある
    pub fn is_protocol_valid(&self, request: &ConnectRequest) -> bool {
        match &self.selected_protocol {
            None => {
                // クライアントがネゴシエーションを要求している場合、
                // レスポンスに WT-Protocol が必須 (draft-15 Section 3.3)
                request.available_protocols.is_empty()
            }
            Some(proto) => {
                if request.available_protocols.is_empty() {
                    false
                } else {
                    request.available_protocols.contains(proto)
                }
            }
        }
    }

    /// `WT-Protocol` ヘッダー値を解析 (RFC Section 3.3)
    ///
    /// Structured Fields Item 形式 (RFC 9651) から文字列型のみを抽出する。
    /// 文字列型でない場合は `None` を返す (RFC Section 3.3)。
    /// パラメータ (`;` 以降) は無視する (RFC Section 3.3)。
    pub fn parse_protocol(header_value: &str) -> Option<String> {
        parse_sf_item_string(header_value)
    }
}

/// Structured Fields List から文字列型アイテムを抽出 (RFC 9651 簡易実装)
///
/// WebTransport の用途に特化した実装:
/// - カンマ区切りのリストを解析
/// - 全アイテムがクォート文字列の場合のみ結果を返す
/// - 文字列型以外 (Integer, Token, Boolean 等) を含む場合はフィールド全体を無視する
///   (draft-ietf-webtrans-http3-15 Section 3.3)
/// - パラメータ (`;` 以降) は無視
fn parse_sf_list_strings(value: &str) -> Vec<String> {
    let mut result = Vec::new();
    for item in value.split(',') {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        match parse_sf_item_string(item) {
            Some(s) => result.push(s),
            None => {
                // 非 String 要素を検出: フィールド全体を無視
                // (draft-ietf-webtrans-http3-15 Section 3.3)
                return Vec::new();
            }
        }
    }
    result
}

/// Structured Fields Item から文字列型を抽出 (RFC 9651 簡易実装)
///
/// フォーマット: `"<string>"[;<params>]`
/// - クォート文字列でない場合は `None` を返す
/// - パラメータ (`;` 以降、クォート外) は無視する
fn parse_sf_item_string(value: &str) -> Option<String> {
    let value = value.trim();

    // クォート外のパラメータを除去 (`;` 以降)
    let value = strip_sf_parameters(value);
    let value = value.trim();

    // クォート文字列の解析 (RFC 9651 Section 4.1.2)
    if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
        let inner = &value[1..value.len() - 1];
        // RFC 9651: バックスラッシュエスケープ (\\ → \, \" → ")
        // 一時的に \\ を置換して \" の誤処理を防ぐ
        let unescaped = inner
            .replace("\\\\", "\x00")
            .replace("\\\"", "\"")
            .replace('\x00', "\\");
        Some(unescaped)
    } else {
        // クォートされていない = 文字列型ではない
        None
    }
}

/// Structured Fields のパラメータを除去 (クォート外の `;` 以降を削除)
fn strip_sf_parameters(value: &str) -> &str {
    let bytes = value.as_bytes();
    let mut in_string = false;
    let mut escaped = false;

    for (i, &b) in bytes.iter().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        match b {
            b'"' => in_string = !in_string,
            b'\\' if in_string => escaped = true,
            b';' if !in_string => return &value[..i],
            _ => {}
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connect_request_validate_success() {
        let req = ConnectRequest::new("https", "example.com", "/path");
        assert!(req.validate().is_ok());
    }

    #[test]
    fn test_connect_request_validate_invalid_scheme() {
        let req = ConnectRequest::new("http", "example.com", "/");
        assert_eq!(req.validate(), Err(ConnectError::InvalidScheme));
    }

    #[test]
    fn test_connect_request_validate_missing_authority() {
        let req = ConnectRequest::new("https", "", "/");
        assert_eq!(req.validate(), Err(ConnectError::MissingAuthority));
    }

    #[test]
    fn test_connect_request_validate_missing_path() {
        let req = ConnectRequest::new("https", "example.com", "");
        assert_eq!(req.validate(), Err(ConnectError::MissingPath));
    }

    #[test]
    fn test_parse_available_protocols_basic() {
        let result = ConnectRequest::parse_available_protocols("\"foo\", \"bar\"");
        assert_eq!(result, vec!["foo".to_string(), "bar".to_string()]);
    }

    #[test]
    fn test_parse_available_protocols_ignores_non_strings() {
        // 非 String 要素を含む場合はフィールド全体を無視する
        // (draft-ietf-webtrans-http3-15 Section 3.3)
        let result = ConnectRequest::parse_available_protocols("\"foo\", token, 42");
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_available_protocols_with_parameters() {
        // パラメータは無視される
        let result = ConnectRequest::parse_available_protocols("\"foo\";q=1, \"bar\";q=0.5");
        assert_eq!(result, vec!["foo".to_string(), "bar".to_string()]);
    }

    #[test]
    fn test_parse_available_protocols_empty() {
        let result = ConnectRequest::parse_available_protocols("");
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_protocol_basic() {
        let result = ConnectResponse::parse_protocol("\"foo\"");
        assert_eq!(result, Some("foo".to_string()));
    }

    #[test]
    fn test_parse_protocol_non_string() {
        // Token 型 (クォートなし) は None
        let result = ConnectResponse::parse_protocol("token");
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_protocol_with_parameters() {
        // パラメータは無視される
        let result = ConnectResponse::parse_protocol("\"foo\";q=1");
        assert_eq!(result, Some("foo".to_string()));
    }

    #[test]
    fn test_parse_protocol_escape_quote() {
        let result = ConnectResponse::parse_protocol("\"foo\\\"bar\"");
        assert_eq!(result, Some("foo\"bar".to_string()));
    }

    #[test]
    fn test_parse_protocol_escape_backslash() {
        let result = ConnectResponse::parse_protocol("\"foo\\\\bar\"");
        assert_eq!(result, Some("foo\\bar".to_string()));
    }

    #[test]
    fn test_connect_response_is_success() {
        assert!(ConnectResponse::new(200).is_success());
        assert!(ConnectResponse::new(201).is_success());
        assert!(!ConnectResponse::new(301).is_success());
        assert!(!ConnectResponse::new(404).is_success());
        assert!(!ConnectResponse::new(500).is_success());
    }

    #[test]
    fn test_is_protocol_valid_no_available_protocols() {
        // WT-Available-Protocols なしで WT-Protocol あり = 無効
        let req = ConnectRequest::new("https", "example.com", "/");
        let resp = ConnectResponse::new(200).with_protocol("foo");
        assert!(!resp.is_protocol_valid(&req));
    }

    #[test]
    fn test_is_protocol_valid_match() {
        let req = ConnectRequest::new("https", "example.com", "/")
            .available_protocols(vec!["foo".to_string(), "bar".to_string()]);
        let resp = ConnectResponse::new(200).with_protocol("foo");
        assert!(resp.is_protocol_valid(&req));
    }

    #[test]
    fn test_is_protocol_valid_no_match() {
        let req = ConnectRequest::new("https", "example.com", "/")
            .available_protocols(vec!["foo".to_string()]);
        let resp = ConnectResponse::new(200).with_protocol("baz");
        assert!(!resp.is_protocol_valid(&req));
    }

    #[test]
    fn test_is_protocol_valid_no_protocol_in_response_no_negotiation() {
        // リクエストにも WT-Available-Protocols なし = ネゴシエーション不要なので有効
        let req = ConnectRequest::new("https", "example.com", "/");
        let resp = ConnectResponse::new(200);
        assert!(resp.is_protocol_valid(&req));
    }

    #[test]
    fn test_is_protocol_valid_no_protocol_in_response_with_negotiation() {
        // リクエストに WT-Available-Protocols あり、レスポンスに WT-Protocol なし
        // → draft-15 Section 3.3: MUST close with WT_ALPN_ERROR
        let req = ConnectRequest::new("https", "example.com", "/")
            .available_protocols(vec!["foo".to_string()]);
        let resp = ConnectResponse::new(200);
        assert!(!resp.is_protocol_valid(&req));
    }

    #[test]
    fn test_connect_error_display() {
        assert_eq!(
            format!("{}", ConnectError::InvalidScheme),
            ":scheme must be \"https\""
        );
        assert_eq!(
            format!("{}", ConnectError::MissingAuthority),
            ":authority is required"
        );
        assert_eq!(
            format!("{}", ConnectError::MissingPath),
            ":path is required"
        );
    }

    // ---------------------------------------------------------------
    // to_headers / from_headers テスト
    // ---------------------------------------------------------------

    #[test]
    fn test_connect_request_to_headers_basic() {
        let req = ConnectRequest::new("https", "example.com", "/wt");
        let headers = req.to_headers();
        assert_eq!(headers.len(), 5);
        assert_eq!(headers[0].name, b":method");
        assert_eq!(headers[0].value, b"CONNECT");
        assert_eq!(headers[1].name, b":protocol");
        assert_eq!(headers[1].value, PROTOCOL_WEBTRANSPORT_H3.as_bytes());
        assert_eq!(headers[2].name, b":scheme");
        assert_eq!(headers[2].value, b"https");
        assert_eq!(headers[3].name, b":authority");
        assert_eq!(headers[3].value, b"example.com");
        assert_eq!(headers[4].name, b":path");
        assert_eq!(headers[4].value, b"/wt");
    }

    #[test]
    fn test_connect_request_to_headers_draft02() {
        let req =
            ConnectRequest::new("https", "example.com", "/wt").draft_version(DraftVersion::Draft02);
        let headers = req.to_headers();
        assert_eq!(headers[1].name, b":protocol");
        assert_eq!(headers[1].value, PROTOCOL_WEBTRANSPORT_DRAFT02.as_bytes());
    }

    #[test]
    fn test_connect_request_to_headers_draft07() {
        let req =
            ConnectRequest::new("https", "example.com", "/wt").draft_version(DraftVersion::Draft07);
        let headers = req.to_headers();
        assert_eq!(headers[1].name, b":protocol");
        assert_eq!(headers[1].value, PROTOCOL_WEBTRANSPORT_DRAFT02.as_bytes());
    }

    #[test]
    fn test_connect_request_to_headers_with_origin() {
        let req =
            ConnectRequest::new("https", "example.com", "/wt").origin("https://client.example");
        let headers = req.to_headers();
        assert_eq!(headers.len(), 6);
        assert_eq!(headers[5].name, b"origin");
        assert_eq!(headers[5].value, b"https://client.example");
    }

    #[test]
    fn test_connect_request_to_headers_with_available_protocols() {
        let req = ConnectRequest::new("https", "example.com", "/wt")
            .available_protocols(vec!["moq".to_string(), "chat".to_string()]);
        let headers = req.to_headers();
        assert_eq!(headers.len(), 6);
        assert_eq!(headers[5].name, b"wt-available-protocols");
        assert_eq!(headers[5].value, b"\"moq\", \"chat\"");
    }

    #[test]
    fn test_connect_request_from_headers_draft15() {
        let headers: Vec<(&[u8], &[u8])> = vec![
            (b":method", b"CONNECT"),
            (b":protocol", b"webtransport-h3"),
            (b":scheme", b"https"),
            (b":authority", b"example.com"),
            (b":path", b"/wt"),
        ];
        let req = ConnectRequest::from_headers(&headers).unwrap();
        assert_eq!(req.draft_version, DraftVersion::Draft15);
        assert_eq!(req.scheme, "https");
        assert_eq!(req.authority, "example.com");
        assert_eq!(req.path, "/wt");
        assert!(req.origin.is_none());
        assert!(req.available_protocols.is_empty());
    }

    #[test]
    fn test_connect_request_from_headers_draft02_compat() {
        // draft-02 互換の :protocol 値も受け入れる
        let headers: Vec<(&[u8], &[u8])> = vec![
            (b":method", b"CONNECT"),
            (b":protocol", b"webtransport"),
            (b":scheme", b"https"),
            (b":authority", b"example.com"),
            (b":path", b"/"),
        ];
        let req = ConnectRequest::from_headers(&headers).unwrap();
        assert_eq!(req.draft_version, DraftVersion::Draft02);
        assert_eq!(req.scheme, "https");
    }

    #[test]
    fn test_connect_request_from_headers_invalid_method() {
        let headers: Vec<(&[u8], &[u8])> =
            vec![(b":method", b"GET"), (b":protocol", b"webtransport-h3")];
        assert!(matches!(
            ConnectRequest::from_headers(&headers),
            Err(ConnectError::InvalidMethod)
        ));
    }

    #[test]
    fn test_connect_request_from_headers_invalid_protocol() {
        let headers: Vec<(&[u8], &[u8])> = vec![(b":method", b"CONNECT"), (b":protocol", b"h2c")];
        assert!(matches!(
            ConnectRequest::from_headers(&headers),
            Err(ConnectError::InvalidProtocol)
        ));
    }

    #[test]
    fn test_connect_request_from_headers_missing_method() {
        let headers: Vec<(&[u8], &[u8])> = vec![(b":protocol", b"webtransport-h3")];
        assert!(matches!(
            ConnectRequest::from_headers(&headers),
            Err(ConnectError::InvalidMethod)
        ));
    }

    #[test]
    fn test_connect_request_from_headers_with_origin() {
        let headers: Vec<(&[u8], &[u8])> = vec![
            (b":method", b"CONNECT"),
            (b":protocol", b"webtransport-h3"),
            (b":scheme", b"https"),
            (b":authority", b"example.com"),
            (b":path", b"/wt"),
            (b"origin", b"https://client.example"),
        ];
        let req = ConnectRequest::from_headers(&headers).unwrap();
        assert_eq!(req.origin, Some("https://client.example".to_string()));
    }

    #[test]
    fn test_connect_request_roundtrip() {
        // to_headers で生成したヘッダーを from_headers でパースする
        let original =
            ConnectRequest::new("https", "example.com", "/wt").origin("https://client.example");
        let headers = original.to_headers();
        let pairs: Vec<(&[u8], &[u8])> = headers
            .iter()
            .map(|h| (h.name.as_slice(), h.value.as_slice()))
            .collect();
        let parsed = ConnectRequest::from_headers(&pairs).unwrap();
        assert_eq!(parsed.scheme, original.scheme);
        assert_eq!(parsed.authority, original.authority);
        assert_eq!(parsed.path, original.path);
        assert_eq!(parsed.origin, original.origin);
    }

    #[test]
    fn test_connect_response_to_headers_basic() {
        let resp = ConnectResponse::new(200);
        let headers = resp.to_headers();
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0].name, b":status");
        assert_eq!(headers[0].value, b"200");
    }

    #[test]
    fn test_connect_response_to_headers_with_protocol() {
        let resp = ConnectResponse::new(200).with_protocol("moq");
        let headers = resp.to_headers();
        assert_eq!(headers.len(), 2);
        assert_eq!(headers[1].name, b"wt-protocol");
        assert_eq!(headers[1].value, b"\"moq\"");
    }

    #[test]
    fn test_connect_error_display_new_variants() {
        assert_eq!(
            format!("{}", ConnectError::InvalidMethod),
            ":method must be \"CONNECT\""
        );
        assert!(format!("{}", ConnectError::InvalidProtocol).contains("webtransport-h3"));
        assert_eq!(
            format!("{}", ConnectError::InvalidEncoding),
            "header value is not valid UTF-8"
        );
    }

    #[test]
    fn test_transport_capabilities_validate_success() {
        let caps = TransportCapabilities {
            wt_enabled: true,
            enable_connect_protocol: true,
            h3_datagram: true,
            max_datagram_frame_size: true,
            reset_stream_at: true,
        };
        assert!(caps.validate().is_ok());
    }

    #[test]
    fn test_transport_capabilities_validate_missing_setting() {
        let caps = TransportCapabilities::new();
        assert_eq!(
            caps.validate(),
            Err(CapabilityError::MissingWebTransportSetting)
        );
    }

    #[test]
    fn test_build_server_settings_draft15() {
        let params = ServerSettingsParams {
            initial_max_streams_uni: 500,
            initial_max_streams_bidi: 300,
            ..Default::default()
        };
        let s = DraftVersion::Draft15.build_server_settings(&params);
        assert_eq!(s.wt_enabled, 1);
        assert_eq!(s.wt_initial_max_streams_uni, 500);
        assert_eq!(s.wt_initial_max_streams_bidi, 300);
        // draft-15 でも wt_initial_max_data を反映する (Section 5.5.3)
        assert_eq!(
            s.wt_initial_max_data,
            ServerSettingsParams::default().initial_max_data
        );
        assert_eq!(s.webtransport_max_sessions_draft07, None);
        assert_eq!(s.wt_max_sessions_draft14, None);
    }

    #[test]
    fn test_build_server_settings_draft14() {
        let params = ServerSettingsParams {
            max_sessions: 100,
            initial_max_streams_uni: 1000,
            initial_max_streams_bidi: 1000,
            initial_max_data: 8 * 1024 * 1024,
        };
        let s = DraftVersion::Draft14.build_server_settings(&params);
        // Safari 互換: draft-07 と draft-14 の両方の max_sessions を設定し、
        // 初期フロー制御値はカプセルで通知するため SETTINGS には含めない。
        assert_eq!(s.wt_max_sessions_draft14, Some(100));
        assert_eq!(s.webtransport_max_sessions_draft07, Some(100));
        assert_eq!(s.wt_initial_max_streams_uni, 0);
        assert_eq!(s.wt_initial_max_streams_bidi, 0);
        assert_eq!(s.wt_initial_max_data, 0);
    }

    #[test]
    fn test_build_server_settings_draft07() {
        let params = ServerSettingsParams::default();
        let s = DraftVersion::Draft07.build_server_settings(&params);
        assert_eq!(s.webtransport_max_sessions_draft07, Some(1));
        assert_eq!(s.wt_enabled, 0);
        assert_eq!(s.wt_max_sessions_draft14, None);
        assert_eq!(s.enable_webtransport_draft02, None);
    }

    #[test]
    fn test_build_server_settings_draft02() {
        let params = ServerSettingsParams::default();
        let s = DraftVersion::Draft02.build_server_settings(&params);
        assert_eq!(s.enable_webtransport_draft02, Some(true));
        assert_eq!(s.wt_enabled, 0);
        assert_eq!(s.webtransport_max_sessions_draft07, None);
    }

    #[test]
    fn test_build_server_settings_draft14_safari_roundtrip() {
        // Draft14 用のサーバー SETTINGS は draft-07 / draft-14 両方の ID を含む。
        // SETTINGS ネゴシエーション優先順位では draft-07 を優先するため、
        // detect_draft_pattern は Draft07 を返す。draft-14 固有のカプセルベース
        // フロー制御はセッション確立後に別途扱う。
        let params = ServerSettingsParams::default();
        let s = DraftVersion::Draft14.build_server_settings(&params);
        assert_eq!(s.detect_draft_pattern(), Some(DraftVersion::Draft07));
        assert_eq!(s.wt_max_sessions_draft14, Some(params.max_sessions));
        assert_eq!(
            s.webtransport_max_sessions_draft07,
            Some(params.max_sessions)
        );
    }

    #[test]
    fn test_requires_initial_capsule_flow_control() {
        assert!(!DraftVersion::Draft02.requires_initial_capsule_flow_control());
        assert!(!DraftVersion::Draft07.requires_initial_capsule_flow_control());
        assert!(DraftVersion::Draft14.requires_initial_capsule_flow_control());
        assert!(!DraftVersion::Draft15.requires_initial_capsule_flow_control());
    }

    #[test]
    fn test_requires_enable_connect_protocol() {
        assert!(!DraftVersion::Draft02.requires_enable_connect_protocol());
        assert!(DraftVersion::Draft07.requires_enable_connect_protocol());
        assert!(DraftVersion::Draft14.requires_enable_connect_protocol());
        assert!(DraftVersion::Draft15.requires_enable_connect_protocol());
    }

    #[test]
    fn test_requires_reset_stream_at() {
        assert!(!DraftVersion::Draft02.requires_reset_stream_at());
        assert!(!DraftVersion::Draft07.requires_reset_stream_at());
        assert!(DraftVersion::Draft14.requires_reset_stream_at());
        assert!(DraftVersion::Draft15.requires_reset_stream_at());
    }

    #[test]
    fn test_build_client_settings_draft15() {
        let params = ServerSettingsParams::default();
        let s = DraftVersion::Draft15.build_client_settings(&params);
        assert_eq!(s.wt_enabled, 1);
        assert_eq!(s.wt_initial_max_streams_uni, params.initial_max_streams_uni);
        assert_eq!(
            s.wt_initial_max_streams_bidi,
            params.initial_max_streams_bidi
        );
        assert_eq!(s.wt_initial_max_data, params.initial_max_data);
        assert_eq!(s.webtransport_max_sessions_draft07, None);
        assert_eq!(s.wt_max_sessions_draft14, None);
        assert_eq!(s.detect_draft_pattern(), Some(DraftVersion::Draft15));
    }

    #[test]
    fn test_build_client_settings_draft14() {
        let params = ServerSettingsParams::default();
        let s = DraftVersion::Draft14.build_client_settings(&params);
        // クライアントは Safari 互換の draft-07 ID を送らない (純粋な draft-14)
        assert_eq!(s.wt_max_sessions_draft14, Some(params.max_sessions));
        assert_eq!(s.webtransport_max_sessions_draft07, None);
        assert_eq!(s.wt_initial_max_streams_uni, params.initial_max_streams_uni);
        assert_eq!(
            s.wt_initial_max_streams_bidi,
            params.initial_max_streams_bidi
        );
        assert_eq!(s.wt_initial_max_data, params.initial_max_data);
        assert_eq!(s.wt_enabled, 0);
        assert_eq!(s.detect_draft_pattern(), Some(DraftVersion::Draft14));
    }

    #[test]
    fn test_build_client_settings_draft07() {
        let params = ServerSettingsParams::default();
        let s = DraftVersion::Draft07.build_client_settings(&params);
        assert_eq!(
            s.webtransport_max_sessions_draft07,
            Some(params.max_sessions)
        );
        assert_eq!(s.wt_max_sessions_draft14, None);
        assert_eq!(s.wt_enabled, 0);
        assert_eq!(s.enable_webtransport_draft02, None);
        assert_eq!(s.detect_draft_pattern(), Some(DraftVersion::Draft07));
    }

    #[test]
    fn test_build_client_settings_draft02() {
        let params = ServerSettingsParams::default();
        let s = DraftVersion::Draft02.build_client_settings(&params);
        assert_eq!(s.enable_webtransport_draft02, Some(true));
        assert_eq!(s.wt_enabled, 0);
        assert_eq!(s.webtransport_max_sessions_draft07, None);
        assert_eq!(s.wt_max_sessions_draft14, None);
        assert_eq!(s.detect_draft_pattern(), Some(DraftVersion::Draft02));
    }
}
