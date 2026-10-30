//! tokio-s2n-quic 設定
//!
//! `s2n-quic` のエンドポイント構築 (TLS プロバイダ / limits プロバイダ) も本モジュールで行う。
//! 設定フィールドはすべてここで s2n-quic の API に配線される。

use std::net::SocketAddr;
use std::time::Duration;

use s2n_quic::provider::limits::Limits as S2nLimits;
use shiguredo_http3::Settings as H3Settings;
use shiguredo_http3::limits::Limits;
use shiguredo_http3::webtransport;
use shiguredo_http3::webtransport::DraftVersion;

/// サーバー設定
pub struct ServerConfig {
    /// リッスンアドレス
    pub listen_addr: SocketAddr,
    /// 証明書 PEM 文字列
    pub cert_pem: String,
    /// 秘密鍵 PEM 文字列
    pub key_pem: String,
    /// ALPN プロトコル
    ///
    /// TLS プロバイダのアプリケーションプロトコルとして設定される。
    /// s2n-quic のデフォルト TLS プロバイダは `h3` を自動設定するため、
    /// 本設定で上書きするには TLS プロバイダを明示的に構築する必要がある。
    pub alpn: Vec<Vec<u8>>,
    /// アイドルタイムアウト (ミリ秒)
    ///
    /// QUIC の `max_idle_timeout` transport parameter として広告される
    /// (RFC 9000 Section 10.1)。実際のタイムアウトはピアの広告値との最小値になる。
    pub idle_timeout_ms: u64,
    /// ピアの双方向ストリーム数
    ///
    /// ピアが開ける双方向ストリーム数の上限 (`initial_max_streams_bidi`)
    /// (RFC 9000 Section 18.2)。WebTransport では CONNECT ストリームと
    /// WT 双方向ストリームもこの上限を消費する。
    pub peer_bidi_stream_count: u16,
    /// ピアの単方向ストリーム数
    ///
    /// ピアが開ける単方向ストリーム数の上限 (`initial_max_streams_uni`)
    /// (RFC 9000 Section 18.2)。
    ///
    /// HTTP/3 の制御ストリームと QPACK エンコーダー / デコーダーストリーム
    /// (合計 3 本) もこの上限を消費する。デフォルト値 3 はこの 3 本だけを
    /// 許す値であり、ピアに WebTransport の単方向ストリームも開かせたい場合は
    /// 3 より大きい値を設定すること。
    pub peer_unidi_stream_count: u16,
    /// HTTP/3 設定
    pub h3_settings: H3Settings,
}

impl ServerConfig {
    /// 新しいサーバー設定を作成する
    pub fn new(
        listen_addr: SocketAddr,
        cert_pem: impl Into<String>,
        key_pem: impl Into<String>,
    ) -> Self {
        Self {
            listen_addr,
            cert_pem: cert_pem.into(),
            key_pem: key_pem.into(),
            alpn: vec![b"h3".to_vec()],
            idle_timeout_ms: 30000,
            peer_bidi_stream_count: 100,
            peer_unidi_stream_count: 3,
            h3_settings: H3Settings::from_limits(&Limits::default())
                .expect("Limits::default() values must fit VarInt (RFC 9000 Section 16)"),
        }
    }

    /// WebTransport を有効にする
    ///
    /// WebTransport を扱わない `H3Server::bind` に本設定を有効化した
    /// `ServerConfig` を渡すと `Error::InvalidState` で拒否される。
    /// WebTransport サーバーとして使う場合も本設定を有効化した `ServerConfig` を
    /// `WtServer::bind` に渡すこと。
    pub fn enable_webtransport(mut self, wt: webtransport::Settings) -> Self {
        self.h3_settings = self.h3_settings.enable_webtransport_server(wt);
        self
    }

    /// s2n-quic のサーバーを構築する
    ///
    /// 全フィールドを s2n-quic の API に配線する:
    /// - `cert_pem` / `key_pem`: TLS 証明書と秘密鍵
    /// - `alpn`: TLS プロバイダのアプリケーションプロトコル
    /// - `idle_timeout_ms` / `peer_bidi_stream_count` / `peer_unidi_stream_count`:
    ///   limits プロバイダ
    pub(crate) fn build_server(&self) -> crate::Result<s2n_quic::Server> {
        let tls = s2n_quic::provider::tls::default::Server::builder()
            .with_application_protocols(self.alpn.iter())
            .map_err(crate::Error::transport)?
            .with_certificate(self.cert_pem.as_str(), self.key_pem.as_str())
            .map_err(crate::Error::transport)?
            .build()
            .map_err(crate::Error::transport)?;

        s2n_quic::Server::builder()
            .with_tls(tls)
            .map_err(crate::Error::transport)?
            .with_limits(server_limits(self)?)
            .map_err(crate::Error::transport)?
            .with_io(self.listen_addr)
            .map_err(crate::Error::transport)?
            .start()
            .map_err(crate::Error::transport)
    }
}

/// `ServerConfig` から s2n-quic の limits を構築する
///
/// s2n-quic の limits は値を取得する公開 API を持たないため、配線の検証は
/// テストで `Debug` 表現を確認して行う。
fn server_limits(config: &ServerConfig) -> crate::Result<S2nLimits> {
    let limits = S2nLimits::new()
        .with_max_idle_timeout(Duration::from_millis(config.idle_timeout_ms))
        .map_err(invalid_limits)?
        .with_max_open_remote_bidirectional_streams(u64::from(config.peer_bidi_stream_count))
        .map_err(invalid_limits)?
        .with_max_open_remote_unidirectional_streams(u64::from(config.peer_unidi_stream_count))
        .map_err(invalid_limits)?;
    Ok(limits)
}

/// s2n-quic の limits 値域エラーを `crate::Error` に変換する
///
/// `s2n_quic::provider::limits` の値域エラーは `std::error::Error` を実装しないため、
/// `Error::transport` ではなく `Error::InvalidState` として報告する。
fn invalid_limits<E: std::fmt::Display>(e: E) -> crate::Error {
    crate::Error::InvalidState(format!("invalid s2n-quic limits: {e}"))
}

/// クライアント設定
pub struct ClientConfig {
    /// リモートアドレス
    pub remote_addr: SocketAddr,
    /// サーバー名
    pub server_name: String,
    /// ALPN プロトコル
    ///
    /// TLS プロバイダのアプリケーションプロトコルとして設定される。
    /// s2n-quic のデフォルト TLS プロバイダは `h3` を自動設定するため、
    /// 本設定で上書きするには TLS プロバイダを明示的に構築する必要がある。
    pub alpn: Vec<Vec<u8>>,
    /// アイドルタイムアウト (ミリ秒)
    ///
    /// QUIC の `max_idle_timeout` transport parameter として広告される
    /// (RFC 9000 Section 10.1)。実際のタイムアウトはピアの広告値との最小値になる。
    pub idle_timeout_ms: u64,
    /// CA 証明書 PEM 文字列
    pub ca_cert_pem: Option<String>,
    /// 証明書検証を無効にする
    pub disable_cert_validation: bool,
    /// HTTP/3 設定
    pub h3_settings: H3Settings,
    /// WebTransport ドラフトバージョン (デフォルト: Draft15)
    pub draft_version: DraftVersion,
}

impl ClientConfig {
    /// 新しいクライアント設定を作成する
    pub fn new(remote_addr: SocketAddr, server_name: impl Into<String>) -> Self {
        Self {
            remote_addr,
            server_name: server_name.into(),
            alpn: vec![b"h3".to_vec()],
            idle_timeout_ms: 5000,
            ca_cert_pem: None,
            disable_cert_validation: false,
            h3_settings: H3Settings::from_limits(&Limits::default())
                .expect("Limits::default() values must fit VarInt (RFC 9000 Section 16)"),
            draft_version: DraftVersion::Draft15,
        }
    }

    /// CA 証明書を設定する
    pub fn ca_cert(mut self, pem: impl Into<String>) -> Self {
        self.ca_cert_pem = Some(pem.into());
        self
    }

    /// 証明書検証を無効にする
    pub fn insecure(mut self) -> Self {
        self.disable_cert_validation = true;
        self
    }

    /// WebTransport を有効にする
    pub fn enable_webtransport(mut self, wt: webtransport::Settings) -> Self {
        self.h3_settings = self.h3_settings.enable_webtransport_client(wt);
        self
    }

    /// WebTransport ドラフトバージョンを設定する
    pub fn wt_draft_version(mut self, version: DraftVersion) -> Self {
        self.draft_version = version;
        self
    }

    /// s2n-quic のクライアントを構築する
    ///
    /// 全フィールドを s2n-quic の API に配線する:
    /// - `ca_cert_pem` / `disable_cert_validation`: TLS 認証の構成
    /// - `alpn`: TLS プロバイダのアプリケーションプロトコル
    /// - `idle_timeout_ms`: limits プロバイダ
    pub(crate) fn build_client(&self) -> crate::Result<s2n_quic::Client> {
        let tls = self.build_client_tls()?;

        s2n_quic::Client::builder()
            .with_tls(tls)
            .map_err(crate::Error::transport)?
            .with_limits(client_limits(self)?)
            .map_err(crate::Error::transport)?
            .with_io("0.0.0.0:0")
            .map_err(crate::Error::transport)?
            .start()
            .map_err(crate::Error::transport)
    }

    /// クライアント用の TLS プロバイダを構築する
    ///
    /// s2n-quic のデフォルト TLS プロバイダは ALPN `h3` を自動設定するため、
    /// `alpn` 設定を反映するにはプロバイダを明示的に構築する必要がある。
    fn build_client_tls(&self) -> crate::Result<s2n_quic::provider::tls::default::Client> {
        let builder = s2n_quic::provider::tls::default::Client::builder()
            .with_application_protocols(self.alpn.iter())
            .map_err(crate::Error::transport)?;

        if self.disable_cert_validation {
            // 証明書検証を無効化する経路。s2n-quic 1.86 の TLS プロバイダは検証無効化用の
            // 公開 API を持たないため、証明書ストアを指定せずに構築する。
            builder.build().map_err(crate::Error::transport)
        } else if let Some(ref ca_pem) = self.ca_cert_pem {
            builder
                .with_certificate(ca_pem.as_str())
                .map_err(crate::Error::transport)?
                .build()
                .map_err(crate::Error::transport)
        } else {
            Err(crate::Error::InvalidState(
                "ca_cert_pem or disable_cert_validation is required".to_string(),
            ))
        }
    }
}

/// `ClientConfig` から s2n-quic の limits を構築する
///
/// s2n-quic の limits は値を取得する公開 API を持たないため、配線の検証は
/// テストで `Debug` 表現を確認して行う。
fn client_limits(config: &ClientConfig) -> crate::Result<S2nLimits> {
    let limits = S2nLimits::new()
        .with_max_idle_timeout(Duration::from_millis(config.idle_timeout_ms))
        .map_err(invalid_limits)?;
    Ok(limits)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// テスト用: サーバー設定を作る
    fn test_server_config() -> ServerConfig {
        ServerConfig::new(
            SocketAddr::from(([127, 0, 0, 1], 0)),
            "dummy cert",
            "dummy key",
        )
    }

    /// テスト用: クライアント設定を作る
    fn test_client_config() -> ClientConfig {
        ClientConfig::new(SocketAddr::from(([127, 0, 0, 1], 0)), "localhost")
    }

    #[test]
    fn test_server_limits_reflect_config() {
        // idle_timeout_ms / peer_bidi_stream_count / peer_unidi_stream_count が
        // s2n-quic の limits に反映されること。
        // s2n-quic の Limits は値を取得する公開 API を持たないため、
        // Debug 表現に設定値が現れることで検証する。
        let mut config = test_server_config();
        config.idle_timeout_ms = 12_345;
        config.peer_bidi_stream_count = 7_001;
        config.peer_unidi_stream_count = 3_001;

        let limits = server_limits(&config).expect("テスト用の limits 構築に成功すること");
        let debug = format!("{limits:?}");
        assert!(
            debug.contains("MaxIdleTimeout") && debug.contains("12345"),
            "idle_timeout_ms が max_idle_timeout に反映されること: {debug}"
        );
        assert!(
            debug.contains("InitialMaxStreamsBidi") && debug.contains("7001"),
            "peer_bidi_stream_count が initial_max_streams_bidi に反映されること: {debug}"
        );
        assert!(
            debug.contains("InitialMaxStreamsUni") && debug.contains("3001"),
            "peer_unidi_stream_count が initial_max_streams_uni に反映されること: {debug}"
        );
    }

    #[test]
    fn test_server_limits_rejects_out_of_range_idle_timeout() {
        // idle_timeout_ms が VarInt 値域を超える場合は設定エラーになること。
        // フィールドが実際に limits 構築へ渡されていることの裏付けになる。
        let mut config = test_server_config();
        config.idle_timeout_ms = u64::MAX;
        let result = server_limits(&config);
        assert!(
            matches!(result, Err(crate::Error::InvalidState(_))),
            "値域外の idle_timeout_ms は InvalidState になること: {result:?}"
        );
    }

    #[test]
    fn test_client_limits_reflect_config() {
        // idle_timeout_ms が s2n-quic の limits に反映されること
        let mut config = test_client_config();
        config.idle_timeout_ms = 54_321;

        let limits = client_limits(&config).expect("テスト用の limits 構築に成功すること");
        let debug = format!("{limits:?}");
        assert!(
            debug.contains("MaxIdleTimeout") && debug.contains("54321"),
            "idle_timeout_ms が max_idle_timeout に反映されること: {debug}"
        );
    }

    #[test]
    fn test_client_limits_rejects_out_of_range_idle_timeout() {
        // idle_timeout_ms が VarInt 値域を超える場合は設定エラーになること
        let mut config = test_client_config();
        config.idle_timeout_ms = u64::MAX;
        let result = client_limits(&config);
        assert!(
            matches!(result, Err(crate::Error::InvalidState(_))),
            "値域外の idle_timeout_ms は InvalidState になること: {result:?}"
        );
    }

    #[test]
    fn test_client_tls_requires_ca_or_insecure() {
        // CA 証明書も disable_cert_validation も指定されていない場合は
        // TLS プロバイダ構築前に InvalidState を返すこと
        let config = test_client_config();
        let result = config.build_client_tls();
        assert!(
            matches!(&result, Err(crate::Error::InvalidState(_))),
            "CA 証明書も insecure 指定もない場合は InvalidState になること"
        );
    }
}
