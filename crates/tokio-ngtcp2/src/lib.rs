//! tokio ベースの I/O 実装
//!
//! ngtcp2/nghttp3 を tokio と統合し、非同期 HTTP/3 クライアント/サーバーを提供する。

mod client;
mod server;
mod webtransport;

pub use client::Client;
pub use server::Server;
pub use webtransport::{ClientWebTransportSession, ServerWebTransportSession};

use std::net::SocketAddr;
use std::time::Instant;

use tokio::net::UdpSocket;

/// UDP ソケットのラッパー
pub(crate) struct Socket {
    inner: UdpSocket,
    local_addr: SocketAddr,
}

impl Socket {
    /// 新しいソケットを作成
    pub async fn bind(addr: SocketAddr) -> std::io::Result<Self> {
        let inner = UdpSocket::bind(addr).await?;
        let local_addr = inner.local_addr()?;
        Ok(Self { inner, local_addr })
    }

    /// ローカルアドレスを取得
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// データを送信
    pub async fn send_to(&self, buf: &[u8], target: SocketAddr) -> std::io::Result<usize> {
        self.inner.send_to(buf, target).await
    }

    /// データを受信
    pub async fn recv_from(&self, buf: &mut [u8]) -> std::io::Result<(usize, SocketAddr)> {
        self.inner.recv_from(buf).await
    }
}

/// タイムスタンプを取得 (ナノ秒)
pub(crate) fn timestamp() -> u64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let start = START.get_or_init(Instant::now);
    start.elapsed().as_nanos() as u64
}
