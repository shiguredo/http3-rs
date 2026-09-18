//! shiguredo_ngtcp2_tokio 上の HTTP/3 クライアント

use std::net::SocketAddr;
use std::time::Duration;

use shiguredo_http3::{Event, Header, Settings};
use shiguredo_ngtcp2_tokio::{
    Client as QuicClient, ClientConfig, ClientConnection, DatagramConfig,
};

use crate::h3::{H3State, QuicConnectionRef};
use crate::{Error, Result};

/// ハンドシェイク完了までの最大待ち時間
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// HTTP/3 クライアント
pub struct Client {
    /// QUIC 接続
    pub(crate) quic: ClientConnection,
    /// HTTP/3 接続状態
    pub(crate) h3: H3State,
}

impl Client {
    /// 証明書検証を行わずに接続する
    pub async fn connect_insecure(remote_addr: SocketAddr, server_name: &str) -> Result<Self> {
        let config = ClientConfig::new(&[b"h3"]).with_verify_peer(false);
        Self::connect_with_config(remote_addr, server_name, config, Settings::default(), false)
            .await
    }

    /// HTTP/3 の設定と DATAGRAM の有無を指定して接続する
    pub(crate) async fn connect_with_settings(
        remote_addr: SocketAddr,
        server_name: &str,
        h3_settings: Settings,
        datagram: bool,
    ) -> Result<Self> {
        let config = ClientConfig::new(&[b"h3"]).with_verify_peer(false);
        Self::connect_with_config(remote_addr, server_name, config, h3_settings, datagram).await
    }

    /// 設定を指定して接続する
    ///
    /// 接続後に HTTP/3 の制御ストリームと QPACK ストリームを開く。
    async fn connect_with_config(
        remote_addr: SocketAddr,
        server_name: &str,
        config: ClientConfig,
        h3_settings: Settings,
        datagram: bool,
    ) -> Result<Self> {
        let config = if datagram {
            config.with_datagram(DatagramConfig {
                max_datagram_frame_size: 65535,
                max_tx_datagram_size: 1350,
            })
        } else {
            config
        };
        let local_addr: SocketAddr = if remote_addr.is_ipv4() {
            "0.0.0.0:0".parse().expect("valid address")
        } else {
            "[::]:0".parse().expect("valid address")
        };
        let mut quic =
            QuicClient::connect_with_config(remote_addr, local_addr, server_name, &config).await?;
        let mut h3 = H3State::new_client(h3_settings);
        {
            let mut conn = QuicConnectionRef::Client(&mut quic);
            h3.init_h3_streams(&mut conn).await?;
            h3.pump(&mut conn).await?;
        }
        Ok(Self { quic, h3 })
    }

    /// ピアの SETTINGS を取得する
    pub(crate) fn peer_settings(&self) -> Option<&Settings> {
        self.h3.peer_settings()
    }

    /// ピアの SETTINGS を受信するまで待つ
    pub async fn handshake(&mut self) -> Result<()> {
        let deadline = tokio::time::Instant::now() + HANDSHAKE_TIMEOUT;
        while self.h3.peer_settings().is_none() {
            if tokio::time::Instant::now() >= deadline {
                return Err(Error::Timeout);
            }
            let mut conn = QuicConnectionRef::Client(&mut self.quic);
            self.h3
                .pump_wait(&mut conn, Duration::from_millis(50))
                .await?;
        }
        Ok(())
    }

    /// リクエストを送信する
    ///
    /// 送信データは次の [`Client::flush`] または [`Client::recv`] で書き出される。
    pub fn send_request(&mut self, headers: &[Header]) -> Result<u64> {
        let stream_id = self.h3.send_request(headers, true)?;
        self.open_bidi_stream_for(stream_id)?;
        Ok(stream_id)
    }

    /// ボディ付きのリクエストを送信する
    pub fn send_request_with_body(&mut self, headers: &[Header], body: Vec<u8>) -> Result<u64> {
        let stream_id = self.h3.send_request(headers, false)?;
        self.open_bidi_stream_for(stream_id)?;
        self.h3.send_body(stream_id, &body, true)?;
        Ok(stream_id)
    }

    /// ストリームを開いたままのリクエストを送信する (WebTransport CONNECT 用)
    pub(crate) fn send_request_streaming(&mut self, headers: &[Header]) -> Result<u64> {
        let stream_id = self.h3.send_request(headers, false)?;
        self.open_bidi_stream_for(stream_id)?;
        Ok(stream_id)
    }

    /// ローカル開始の双方向ストリームを開く
    pub(crate) fn open_bidi_stream(&mut self) -> Result<u64> {
        let stream_id = self.quic.open_bidi_stream()? as u64;
        Ok(stream_id)
    }

    /// ローカル開始の WebTransport ストリームを登録する
    pub(crate) fn register_local_wt_stream(
        &mut self,
        session_id: u64,
        stream_id: u64,
    ) -> Result<()> {
        self.h3.register_local_wt_stream(session_id, stream_id)
    }

    /// WebTransport データストリームに生データを書き込む
    pub(crate) fn write_wt_stream(&mut self, stream_id: u64, data: &[u8], fin: bool) -> Result<()> {
        self.quic.write_stream(stream_id as i64, data, fin)?;
        Ok(())
    }

    /// WebTransport の前提条件を注入する
    pub(crate) fn set_webtransport_transport_verified(&mut self) -> Result<()> {
        self.h3.set_webtransport_transport_verified()
    }

    /// WebTransport セッションの送信待ちカプセルを DATA フレームとして取り出す
    pub(crate) fn take_wt_capsules(&mut self, session_id: u64) -> Vec<Vec<u8>> {
        self.h3.take_wt_capsules(session_id)
    }

    /// WebTransport セッションのデータ消費を通知する
    pub(crate) fn wt_data_consumed(&mut self, session_id: u64, bytes: u64) {
        self.h3.wt_data_consumed(session_id, bytes);
    }

    /// WebTransport セッションのフロー制御が有効かどうか
    pub(crate) fn wt_session_flow_control_enabled(&self, session_id: u64) -> bool {
        self.h3.wt_session_flow_control_enabled(session_id)
    }

    /// WebTransport データグラムを送信する
    ///
    /// ピアが DATAGRAM 非対応の場合は `false` を返す。
    pub(crate) async fn send_wt_datagram(
        &mut self,
        session_id: u64,
        payload: &[u8],
    ) -> Result<bool> {
        if !self.quic.can_send_datagram() {
            return Ok(false);
        }
        {
            let mut conn = QuicConnectionRef::Client(&mut self.quic);
            self.h3
                .send_wt_datagram(&mut conn, session_id, payload)
                .await?;
        }
        self.flush().await?;
        Ok(true)
    }

    /// 受信済みのイベントを 1 つ取り出す
    pub fn poll(&mut self) -> Option<Event> {
        self.h3.poll_event()
    }

    /// 送信待ちのデータが残っているか
    pub(crate) fn has_pending_data(&self) -> bool {
        self.quic.has_pending_data()
    }

    /// 送信待ちのデータを書き出す
    pub async fn flush(&mut self) -> Result<()> {
        let mut conn = QuicConnectionRef::Client(&mut self.quic);
        self.h3.pump(&mut conn).await
    }

    /// `timeout` の間、接続を駆動する
    pub async fn recv(&mut self, timeout: Duration) -> Result<()> {
        let mut conn = QuicConnectionRef::Client(&mut self.quic);
        self.h3.pump_wait(&mut conn, timeout).await
    }

    /// HTTP/3 が割り当てたストリーム ID に対応する QUIC ストリームを開く
    ///
    /// ngtcp2 では双方向ストリームを明示的に開く必要がある。HTTP/3 層と QUIC 層の
    /// ストリーム ID は同じ規則で採番されるため、両者のカウンターがずれると
    /// データを送信できない。
    fn open_bidi_stream_for(&mut self, stream_id: u64) -> Result<()> {
        let opened = self.quic.open_bidi_stream()? as u64;
        if opened != stream_id {
            return Err(Error::InvalidState(
                "QUIC and HTTP/3 stream id mismatch for request",
            ));
        }
        Ok(())
    }
}
