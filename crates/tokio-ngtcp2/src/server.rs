//! shiguredo_ngtcp2_tokio 上の HTTP/3 サーバー

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use shiguredo_http3::{Event, Header, Settings};
use shiguredo_ngtcp2_tokio::{
    AcceptedConnection, ConnectionId, Server as QuicServer, ServerConfig,
};

use crate::Result;
use crate::h3::{H3State, QuicConnectionRef};

/// 接続 1 本の状態
pub(crate) struct ConnectionState {
    /// QUIC 接続
    pub(crate) conn: AcceptedConnection,
    /// HTTP/3 接続状態
    pub(crate) h3: H3State,
    /// リモートアドレス
    pub(crate) addr: SocketAddr,
    /// コネクション ID
    pub(crate) conn_id: ConnectionId,
}

/// HTTP/3 サーバー
pub struct Server {
    /// QUIC サーバー
    pub(crate) quic: QuicServer,
    /// 接続ごとの状態
    pub(crate) connections: HashMap<ConnectionId, ConnectionState>,
    /// サーバーが使用する HTTP/3 設定
    settings: Settings,
}

impl Server {
    /// サーバーを起動する
    pub async fn bind(
        addr: SocketAddr,
        cert_path: impl AsRef<Path>,
        key_path: impl AsRef<Path>,
    ) -> Result<Self> {
        Self::bind_with_settings(addr, cert_path, key_path, Settings::default(), false).await
    }

    /// HTTP/3 の設定と DATAGRAM の有無を指定してサーバーを起動する
    pub(crate) async fn bind_with_settings(
        addr: SocketAddr,
        cert_path: impl AsRef<Path>,
        key_path: impl AsRef<Path>,
        settings: Settings,
        datagram: bool,
    ) -> Result<Self> {
        let mut config = ServerConfig::new(&[b"h3"]);
        if datagram {
            config = config.with_datagram(shiguredo_ngtcp2_tokio::DatagramConfig {
                max_datagram_frame_size: 65535,
                max_tx_datagram_size: 1350,
            });
        }
        let quic = QuicServer::bind(addr, cert_path, key_path, Some(config)).await?;
        Ok(Self {
            quic,
            connections: HashMap::new(),
            settings,
        })
    }

    /// ローカルアドレス
    pub fn local_addr(&self) -> SocketAddr {
        self.quic.local_addr()
    }

    /// 接続を処理し続ける
    ///
    /// ハンドラーはイベントごとに呼ばれ、`Some((headers, body))` を返すと
    /// そのイベントのストリームにレスポンスを送信する。
    pub async fn run<F>(&mut self, mut handler: F) -> Result<()>
    where
        F: FnMut(SocketAddr, Event) -> Option<(Vec<Header>, Vec<u8>)>,
    {
        self.run_inner(&mut handler).await
    }

    /// 接続を処理し続ける共通処理
    async fn run_inner<F>(&mut self, handler: &mut F) -> Result<()>
    where
        F: FnMut(SocketAddr, Event) -> Option<(Vec<Header>, Vec<u8>)>,
    {
        loop {
            // 既存の接続を駆動する。エラーになった接続はサーバー全体を
            // 止めずに切り離す (1 本の不正パケットで全接続を巻き込まない)
            let mut failed = Vec::new();
            for state in self.connections.values_mut() {
                if let Err(e) = pump_connection(state, handler, state.addr).await {
                    eprintln!("[tokio-ngtcp2 server] connection error: {e:?}");
                    failed.push(state.conn_id.clone());
                }
            }
            for conn_id in failed {
                self.connections.remove(&conn_id);
            }

            // 新しい接続を受け付ける
            if self.connections.is_empty() {
                if let Some(conn) = self.quic.accept().await? {
                    self.add_connection(conn).await?;
                }
            } else {
                match tokio::time::timeout(Duration::from_millis(1), self.quic.accept()).await {
                    Ok(Ok(Some(conn))) => self.add_connection(conn).await?,
                    Ok(Ok(None)) => {}
                    Ok(Err(e)) => return Err(e.into()),
                    Err(_) => {}
                }
            }
        }
    }

    /// 接続を追加して HTTP/3 ストリームを初期化する
    pub(crate) async fn add_connection(&mut self, mut conn: AcceptedConnection) -> Result<()> {
        let addr = conn.remote_addr();
        let conn_id = conn.connection_id();
        let mut h3 = H3State::new_server(self.settings);
        {
            let mut quic = QuicConnectionRef::Server(&mut conn);
            h3.init_h3_streams(&mut quic).await?;
            // WebTransport CONNECT の受信前に前提条件を注入する。
            // WebTransport を使わない接続では影響しない。
            h3.set_webtransport_transport_verified()?;
            h3.pump(&mut quic).await?;
        }
        self.connections.insert(
            conn_id.clone(),
            ConnectionState {
                conn,
                h3,
                addr,
                conn_id,
            },
        );
        Ok(())
    }
}

/// 接続 1 本を駆動し、イベントをハンドラーに渡す
async fn pump_connection<F>(
    state: &mut ConnectionState,
    handler: &mut F,
    addr: SocketAddr,
) -> Result<()>
where
    F: FnMut(SocketAddr, Event) -> Option<(Vec<Header>, Vec<u8>)>,
{
    {
        // パケットを受信してイベントを取り込むため、短時間だけ待つ。
        // 待たずに poll だけではソケットからパケットを読まないため、
        // 接続が動かなくなる。
        let mut quic = QuicConnectionRef::Server(&mut state.conn);
        state
            .h3
            .pump_wait(&mut quic, Duration::from_millis(1))
            .await?;
    }
    while let Some(event) = state.h3.poll_event() {
        let stream_id = event.stream_id();
        if let Some((headers, body)) = handler(addr, event)
            && let Some(stream_id) = stream_id
        {
            state.h3.send_response(stream_id, &headers, false)?;
            state.h3.send_body(stream_id, &body, true)?;
        }
    }
    let mut quic = QuicConnectionRef::Server(&mut state.conn);
    state.h3.pump(&mut quic).await?;
    Ok(())
}
