//! QUIC (shiguredo_ngtcp2_tokio) と HTTP/3 (shiguredo_http3) をつなぐ処理
//!
//! QUIC のイベントを HTTP/3 の状態機械に流し込み、HTTP/3 が生成した
//! ストリームデータを QUIC に書き戻す。クライアントとサーバーで共通の処理を
//! ここに集約する。

use std::collections::VecDeque;
use std::time::Duration;

use shiguredo_http3::{ClientConnection, Event, Header, ServerConnection, Settings};
use shiguredo_ngtcp2_tokio::{
    AcceptedConnection, ClientConnection as QuicClientConnection, ConnectionEvent, StreamId,
};

use crate::{Error, Result};

/// QUIC 接続への参照
///
/// クライアント (`ClientConnection`) とサーバー (`AcceptedConnection`) は
/// 同じ操作を提供するが共通の型を持たないため、列挙型でまとめる。
pub(crate) enum QuicConnectionRef<'a> {
    /// クライアント接続
    Client(&'a mut QuicClientConnection),
    /// サーバー接続
    Server(&'a mut AcceptedConnection),
}

impl QuicConnectionRef<'_> {
    /// 未処理のイベントを 1 つ取り出す
    pub(crate) fn poll_event(&mut self) -> Option<ConnectionEvent> {
        match self {
            Self::Client(conn) => conn.poll_event(),
            Self::Server(conn) => conn.poll_event(),
        }
    }

    /// 接続が閉じているか
    pub(crate) fn is_closed(&self) -> bool {
        match self {
            Self::Client(conn) => conn.is_closed(),
            Self::Server(conn) => conn.is_closed(),
        }
    }

    /// 送信待ちのデータが残っているか
    pub(crate) fn has_pending_data(&self) -> bool {
        match self {
            Self::Client(conn) => conn.has_pending_data(),
            Self::Server(conn) => conn.has_pending_data(),
        }
    }

    /// ストリームをリセットする (RESET_STREAM)
    pub(crate) fn reset_stream(&mut self, stream_id: StreamId, error_code: u64) -> Result<()> {
        match self {
            Self::Client(conn) => conn.reset_stream(stream_id, error_code)?,
            Self::Server(conn) => conn.reset_stream(stream_id, error_code)?,
        }
        Ok(())
    }

    /// ストリームにデータを書き込む
    pub(crate) fn write_stream(
        &mut self,
        stream_id: StreamId,
        data: &[u8],
        fin: bool,
    ) -> Result<()> {
        match self {
            Self::Client(conn) => conn.write_stream(stream_id, data, fin)?,
            Self::Server(conn) => conn.write_stream(stream_id, data, fin)?,
        };
        Ok(())
    }

    /// ストリームのフロー制御クレジットを進める
    ///
    /// ストリームが既に閉じている場合のエラーは無視する。
    pub(crate) fn extend_max_stream_offset(&mut self, stream_id: StreamId, consumed: u64) {
        let result = match self {
            Self::Client(conn) => conn.extend_max_stream_offset(stream_id, consumed),
            Self::Server(conn) => conn.extend_max_stream_offset(stream_id, consumed),
        };
        let _ = result;
    }

    /// 単方向ストリームを開く
    pub(crate) fn open_uni_stream(&mut self) -> Result<StreamId> {
        let stream_id = match self {
            Self::Client(conn) => conn.open_uni_stream()?,
            Self::Server(conn) => conn.open_uni_stream()?,
        };
        Ok(stream_id)
    }

    /// 送信待ちのデータを書き出す
    pub(crate) async fn flush(&mut self) -> Result<()> {
        match self {
            Self::Client(conn) => conn.flush().await?,
            Self::Server(conn) => conn.flush().await?,
        }
        Ok(())
    }

    /// イベントを 1 つ待つ
    pub(crate) async fn recv_event(&mut self) -> Result<ConnectionEvent> {
        let event = match self {
            Self::Client(conn) => conn.recv_event().await?,
            Self::Server(conn) => conn.recv_event().await?,
        };
        Ok(event)
    }

    /// データグラムを送信する
    pub(crate) async fn send_datagram(&mut self, data: &[u8]) -> Result<()> {
        match self {
            Self::Client(conn) => conn.send_datagram(data).await?,
            Self::Server(conn) => conn.send_datagram(data).await?,
        }
        Ok(())
    }

    /// ピアが DATAGRAM を受理するか
    pub(crate) fn can_send_datagram(&self) -> bool {
        match self {
            Self::Client(conn) => conn.can_send_datagram(),
            Self::Server(conn) => conn.can_send_datagram(),
        }
    }
}

/// HTTP/3 接続 (クライアント / サーバー)
pub(crate) enum H3Connection {
    /// クライアント接続
    Client(ClientConnection),
    /// サーバー接続
    Server(ServerConnection),
}

impl H3Connection {
    /// ストリームデータを流し込む
    ///
    /// エラーの分類 (ストリームエラー / 接続エラー) は呼び出し側で行うため、
    /// HTTP/3 層のエラーをそのまま返す。
    fn feed_stream(
        &mut self,
        stream_id: u64,
        data: &[u8],
        fin: bool,
    ) -> std::result::Result<(), shiguredo_http3::Error> {
        match self {
            Self::Client(conn) => conn.feed_stream(stream_id, data, fin),
            Self::Server(conn) => conn.feed_stream(stream_id, data, fin),
        }
    }

    /// データグラムを流し込む
    fn feed_datagram(&mut self, data: &[u8]) -> Result<()> {
        match self {
            Self::Client(conn) => conn.feed_datagram(data)?,
            Self::Server(conn) => conn.feed_datagram(data)?,
        }
        Ok(())
    }

    /// RESET_STREAM を通知する
    fn stream_reset(&mut self, stream_id: u64, error_code: u64, final_size: u64) -> Result<()> {
        match self {
            Self::Client(conn) => conn.stream_reset(stream_id, error_code, final_size)?,
            Self::Server(conn) => conn.stream_reset(stream_id, error_code, final_size)?,
        }
        Ok(())
    }

    /// 発生したイベントをすべて取り出す
    fn drain_events(&mut self) -> Result<Vec<Event>> {
        let events = match self {
            Self::Client(conn) => conn.drain_events()?,
            Self::Server(conn) => conn.drain_events()?,
        };
        Ok(events)
    }

    /// 送信可能なストリーム ID の一覧
    fn writable_streams(&self) -> Vec<u64> {
        match self {
            Self::Client(conn) => conn.writable_streams().collect(),
            Self::Server(conn) => conn.writable_streams().collect(),
        }
    }

    /// ストリームの送信データを取り出す
    fn take_stream_data(&mut self, stream_id: u64) -> Option<(Vec<u8>, bool)> {
        match self {
            Self::Client(conn) => conn.take_stream_data(stream_id),
            Self::Server(conn) => conn.take_stream_data(stream_id),
        }
    }

    /// ボディを送信する
    fn send_body(&mut self, stream_id: u64, data: &[u8], fin: bool) -> Result<()> {
        match self {
            Self::Client(conn) => conn.send_body(stream_id, data, fin)?,
            Self::Server(conn) => conn.send_body(stream_id, data, fin)?,
        }
        Ok(())
    }

    /// WebTransport データグラムを送信用にエンコードする
    fn send_datagram(&self, session_id: u64, payload: &[u8]) -> Result<Vec<u8>> {
        let encoded = match self {
            Self::Client(conn) => conn.send_datagram(session_id, payload)?,
            Self::Server(conn) => conn.send_datagram(session_id, payload)?,
        };
        Ok(encoded)
    }

    /// ピアの SETTINGS を取得する
    fn peer_settings(&self) -> Option<&Settings> {
        match self {
            Self::Client(conn) => conn.peer_settings(),
            Self::Server(conn) => conn.peer_settings(),
        }
    }

    /// ローカル開始の WebTransport ストリームを登録する
    fn register_local_wt_stream(&mut self, session_id: u64, stream_id: u64) -> Result<()> {
        match self {
            Self::Client(conn) => conn.register_local_wt_stream(session_id, stream_id)?,
            Self::Server(conn) => conn.register_local_wt_stream(session_id, stream_id)?,
        }
        Ok(())
    }

    /// WebTransport の前提条件を注入する
    fn set_webtransport_transport_verified(
        &mut self,
        max_datagram_frame_size_nonzero: bool,
        reset_stream_at_supported: bool,
    ) -> Result<()> {
        match self {
            Self::Client(conn) => conn.set_webtransport_transport_verified(
                max_datagram_frame_size_nonzero,
                reset_stream_at_supported,
            )?,
            Self::Server(conn) => conn.set_webtransport_transport_verified(
                max_datagram_frame_size_nonzero,
                reset_stream_at_supported,
            )?,
        }
        Ok(())
    }
}

/// HTTP/3 接続の状態
pub(crate) struct H3State {
    /// HTTP/3 接続
    conn: H3Connection,
    /// 受信済みで未処理のイベント
    events: VecDeque<Event>,
}

impl H3State {
    /// クライアント接続の状態を作成する
    pub(crate) fn new_client(settings: Settings) -> Self {
        Self {
            conn: H3Connection::Client(ClientConnection::new(settings)),
            events: VecDeque::new(),
        }
    }

    /// サーバー接続の状態を作成する
    pub(crate) fn new_server(settings: Settings) -> Self {
        Self {
            conn: H3Connection::Server(ServerConnection::new(settings)),
            events: VecDeque::new(),
        }
    }

    /// 制御ストリーム・QPACK ストリームを開いて初期データを送信する
    pub(crate) async fn init_h3_streams(&mut self, quic: &mut QuicConnectionRef<'_>) -> Result<()> {
        let control_stream_id = quic.open_uni_stream()? as u64;
        let encoder_stream_id = quic.open_uni_stream()? as u64;
        let decoder_stream_id = quic.open_uni_stream()? as u64;
        let init = match &mut self.conn {
            H3Connection::Client(conn) => {
                conn.init_h3_streams(control_stream_id, encoder_stream_id, decoder_stream_id)?
            }
            H3Connection::Server(conn) => {
                conn.init_h3_streams(control_stream_id, encoder_stream_id, decoder_stream_id)?
            }
        };
        quic.write_stream(
            init.control_stream_id as StreamId,
            &init.control_data,
            false,
        )?;
        quic.write_stream(
            init.encoder_stream_id as StreamId,
            &init.encoder_data,
            false,
        )?;
        quic.write_stream(
            init.decoder_stream_id as StreamId,
            &init.decoder_data,
            false,
        )?;
        quic.flush().await?;
        Ok(())
    }

    /// WebTransport の前提条件を注入する
    ///
    /// QUIC DATAGRAM と RESET_STREAM_AT に対応しているものとして扱う。
    /// 相互運用テストで接続する s2n-quic / quinn (h3-webtransport) は
    /// QUIC DATAGRAM を transport parameter で広告しないが、WebTransport の
    /// ストリーム機能は利用できるため、tokio-s2n-quic と同じ扱いに合わせる。
    pub(crate) fn set_webtransport_transport_verified(&mut self) -> Result<()> {
        self.conn.set_webtransport_transport_verified(true, true)
    }

    /// 未処理のイベントを 1 つ取り出す
    pub(crate) fn poll_event(&mut self) -> Option<Event> {
        self.events.pop_front()
    }

    /// ピアの SETTINGS を取得する
    pub(crate) fn peer_settings(&self) -> Option<&Settings> {
        self.conn.peer_settings()
    }

    /// リクエストを送信する (クライアント専用)
    pub(crate) fn send_request(&mut self, headers: &[Header], fin: bool) -> Result<u64> {
        let stream_id = match &mut self.conn {
            H3Connection::Client(conn) => conn.send_request(headers, fin)?,
            H3Connection::Server(_) => {
                return Err(Error::InvalidState("send_request is client only"));
            }
        };
        Ok(stream_id)
    }

    /// レスポンスを送信する (サーバー専用)
    pub(crate) fn send_response(
        &mut self,
        stream_id: u64,
        headers: &[Header],
        fin: bool,
    ) -> Result<()> {
        match &mut self.conn {
            H3Connection::Server(conn) => conn.send_response(stream_id, headers, fin)?,
            H3Connection::Client(_) => {
                return Err(Error::InvalidState("send_response is server only"));
            }
        }
        Ok(())
    }

    /// ボディを送信する
    pub(crate) fn send_body(&mut self, stream_id: u64, data: &[u8], fin: bool) -> Result<()> {
        self.conn.send_body(stream_id, data, fin)
    }

    /// WebTransport ストリームを登録する
    pub(crate) fn register_local_wt_stream(
        &mut self,
        session_id: u64,
        stream_id: u64,
    ) -> Result<()> {
        self.conn.register_local_wt_stream(session_id, stream_id)
    }

    /// WebTransport セッションの送信待ちカプセルを DATA フレームとして取り出す
    pub(crate) fn take_wt_capsules(&mut self, session_id: u64) -> Vec<Vec<u8>> {
        let capsules = match &mut self.conn {
            H3Connection::Client(conn) => conn.take_wt_pending_capsules(session_id),
            H3Connection::Server(conn) => conn.take_wt_pending_capsules(session_id),
        };
        capsules
            .iter()
            .map(|capsule| {
                let mut buf = Vec::new();
                capsule.encode_as_data_frame(&mut buf);
                buf
            })
            .collect()
    }

    /// WebTransport セッションのデータ消費を通知する
    pub(crate) fn wt_data_consumed(&mut self, session_id: u64, bytes: u64) {
        match &mut self.conn {
            H3Connection::Client(conn) => conn.wt_data_consumed(session_id, bytes),
            H3Connection::Server(conn) => conn.wt_data_consumed(session_id, bytes),
        }
    }

    /// WebTransport セッションのフロー制御が有効かどうか
    pub(crate) fn wt_session_flow_control_enabled(&self, session_id: u64) -> bool {
        match &self.conn {
            H3Connection::Client(conn) => conn.wt_session_flow_control_enabled(session_id),
            H3Connection::Server(conn) => conn.wt_session_flow_control_enabled(session_id),
        }
    }

    /// WebTransport データグラムを送信する
    pub(crate) async fn send_wt_datagram(
        &mut self,
        quic: &mut QuicConnectionRef<'_>,
        session_id: u64,
        payload: &[u8],
    ) -> Result<()> {
        if !quic.can_send_datagram() {
            return Err(Error::InvalidState("peer does not support DATAGRAM"));
        }
        let encoded = self.conn.send_datagram(session_id, payload)?;
        quic.send_datagram(&encoded).await?;
        Ok(())
    }

    /// QUIC のイベントを HTTP/3 に反映する
    ///
    /// 戻り値は接続が閉じたかどうか。
    fn handle_quic_event(
        &mut self,
        event: ConnectionEvent,
        quic: &mut QuicConnectionRef<'_>,
    ) -> Result<()> {
        match event {
            ConnectionEvent::StreamData {
                stream_id,
                data,
                fin,
            } => {
                match self.conn.feed_stream(stream_id as u64, &data, fin) {
                    Ok(()) => {}
                    // ストリームエラーは該当ストリームをリセットして接続を維持する
                    Err(shiguredo_http3::Error::StreamError(code)) => {
                        quic.reset_stream(stream_id, code as u64)?;
                        return Ok(());
                    }
                    Err(e) => return Err(e.into()),
                }
                quic.extend_max_stream_offset(stream_id, data.len() as u64);
            }
            ConnectionEvent::StreamReset {
                stream_id,
                final_size,
                app_error_code,
            } => {
                self.conn
                    .stream_reset(stream_id as u64, app_error_code, final_size)?;
            }
            ConnectionEvent::Datagram { data } => {
                self.conn.feed_datagram(&data)?;
            }
            ConnectionEvent::ConnectionClosed { .. } => {}
            _ => {}
        }
        Ok(())
    }

    /// HTTP/3 が生成した送信データを QUIC に書き込む
    fn write_pending(&mut self, quic: &mut QuicConnectionRef<'_>) -> Result<()> {
        for stream_id in self.conn.writable_streams() {
            while let Some((data, fin)) = self.conn.take_stream_data(stream_id) {
                let fin_only = data.is_empty() && fin;
                quic.write_stream(stream_id as StreamId, &data, fin)?;
                if fin_only {
                    break;
                }
            }
        }
        Ok(())
    }

    /// QUIC と HTTP/3 のデータを 1 往復やり取りする (待たない)
    pub(crate) async fn pump(&mut self, quic: &mut QuicConnectionRef<'_>) -> Result<()> {
        let mut closed = false;
        while let Some(event) = quic.poll_event() {
            if matches!(event, ConnectionEvent::ConnectionClosed { .. }) {
                closed = true;
            }
            self.handle_quic_event(event, quic)?;
        }
        if !closed {
            self.write_pending(quic)?;
        }
        quic.flush().await?;
        while let Some(event) = quic.poll_event() {
            // flush で発生したイベントも取り込む (終了イベントは次の pump で扱う)
            self.handle_quic_event(event, quic)?;
        }
        for event in self.conn.drain_events()? {
            self.events.push_back(event);
        }
        Ok(())
    }

    /// イベントが発生するか `timeout` が経過するまで接続を駆動する
    pub(crate) async fn pump_wait(
        &mut self,
        quic: &mut QuicConnectionRef<'_>,
        timeout: Duration,
    ) -> Result<()> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            self.pump(quic).await?;
            if quic.is_closed() {
                break;
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                break;
            }
            match tokio::time::timeout(deadline - now, quic.recv_event()).await {
                Ok(Ok(event)) => {
                    let closed = matches!(event, ConnectionEvent::ConnectionClosed { .. });
                    self.handle_quic_event(event, quic)?;
                    if closed {
                        break;
                    }
                }
                // 接続終了 (ConnectionClosed) や回復不能なエラーでは駆動をやめる。
                // 受信済みのイベントは呼び出し側が poll_event で取り出せる。
                Ok(Err(_)) => break,
                Err(_) => break,
            }
        }
        self.pump(quic).await?;
        Ok(())
    }
}
