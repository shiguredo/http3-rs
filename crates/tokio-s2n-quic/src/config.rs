//! tokio-s2n-quic 設定

use std::net::SocketAddr;

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
    pub alpn: Vec<Vec<u8>>,
    /// アイドルタイムアウト (ミリ秒)
    pub idle_timeout_ms: u64,
    /// ピアの双方向ストリーム数
    pub peer_bidi_stream_count: u16,
    /// ピアの単方向ストリーム数
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
            h3_settings: H3Settings::from_limits(&Limits::default()),
        }
    }

    /// WebTransport を有効にする
    pub fn enable_webtransport(mut self, wt: webtransport::Settings) -> Self {
        self.h3_settings = self.h3_settings.enable_webtransport_server(wt);
        self
    }
}

/// クライアント設定
pub struct ClientConfig {
    /// リモートアドレス
    pub remote_addr: SocketAddr,
    /// サーバー名
    pub server_name: String,
    /// ALPN プロトコル
    pub alpn: Vec<Vec<u8>>,
    /// アイドルタイムアウト (ミリ秒)
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
            h3_settings: H3Settings::from_limits(&Limits::default()),
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
}
