//! WebTransport サーバー側実装
//!
//! s2n-quic + shiguredo_http3 (Sans I/O) を使い WebTransport セッションを受け付ける。
//! draft-ietf-webtrans-http3 の draft-02 / 07 / 14 / 15 に対応する。
//! ドラフト判定は `Settings::webtransport_draft_pattern()`、CONNECT 拒否は `WtSessionRequest::reject()` を利用する。

#![allow(dead_code)]

use std::sync::{Arc, Mutex as StdMutex};

use bytes::Bytes;
use s2n_quic::connection::BidirectionalStreamAcceptor;
use s2n_quic::stream::{ReceiveStream, SendStream};
use shiguredo_http3::webtransport::capsule::Capsule;
use shiguredo_http3::webtransport::connect::DraftVersion;
use shiguredo_http3::webtransport::connect::{ConnectRequest, ConnectResponse};
use shiguredo_http3::webtransport::stream::{
    ClassifiedUniStream, StreamHeader, StreamHeaderDecodeError, classify_uni_stream_checked,
};
use shiguredo_http3::{Connection, Event, Settings as H3Settings, SettingsPayload};
use tokio::sync::Notify;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

// ---------------------------------------------------------------------------
// エラー型
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum Error {
    /// s2n-quic トランスポートエラー
    Transport(Box<dyn std::error::Error + Send + Sync>),
    /// HTTP/3 プロトコルエラー
    Http3(shiguredo_http3::Error),
    /// 接続がクローズ済み
    ConnectionClosed,
    /// ストリームがクローズ済み
    StreamClosed,
    /// 無効な状態
    InvalidState(String),
    /// 内部エラー
    Internal(String),
}

impl Error {
    fn transport(e: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> Self {
        Self::Transport(e.into())
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(e) => write!(f, "transport error: {e}"),
            Self::Http3(e) => write!(f, "http3 error: {e}"),
            Self::ConnectionClosed => write!(f, "connection closed"),
            Self::StreamClosed => write!(f, "stream closed"),
            Self::InvalidState(msg) => write!(f, "invalid state: {msg}"),
            Self::Internal(msg) => write!(f, "internal error: {msg}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<shiguredo_http3::Error> for Error {
    fn from(e: shiguredo_http3::Error) -> Self {
        Self::Http3(e)
    }
}

impl From<s2n_quic::connection::Error> for Error {
    fn from(e: s2n_quic::connection::Error) -> Self {
        Self::transport(e)
    }
}

impl From<s2n_quic::stream::Error> for Error {
    fn from(e: s2n_quic::stream::Error) -> Self {
        Self::transport(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

// ---------------------------------------------------------------------------
// HTTP/3 接続状態 (Sans I/O ラッパー)
// ---------------------------------------------------------------------------

struct ServerConnectionState {
    h3_conn: Connection,
    control_stream_id: Option<u64>,
}

impl ServerConnectionState {
    fn new(settings: H3Settings) -> Self {
        Self {
            h3_conn: Connection::server(settings),
            control_stream_id: None,
        }
    }

    fn set_control_stream_id(&mut self, id: u64) {
        self.control_stream_id = Some(id);
    }

    fn process_stream_data(&mut self, id: u64, data: &[u8], fin: bool) -> Result<Vec<Event>> {
        self.h3_conn.feed_stream(id, data, fin)?;
        Ok(self.h3_conn.drain_events()?)
    }

    fn feed_stream_only(&mut self, id: u64, data: &[u8], fin: bool) -> Result<()> {
        self.h3_conn.feed_stream(id, data, fin)?;
        Ok(())
    }

    fn take_control_pending(&mut self) -> Option<Vec<u8>> {
        let id = self.control_stream_id?;
        self.h3_conn.take_stream_data(id).map(|(d, _)| d)
    }

    fn drain_events(&mut self) -> Result<Vec<Event>> {
        Ok(self.h3_conn.drain_events()?)
    }
}

// ---------------------------------------------------------------------------
// WebTransport セッションリクエスト
// ---------------------------------------------------------------------------

/// WebTransport セッションリクエスト
///
/// accept 済み QUIC 接続から HTTP/3 CONNECT ハンドシェイクを処理する。
pub struct WtSessionRequest {
    path: String,
    authority: String,
    draft: DraftVersion,
    stream_id: u64,
    state: Arc<StdMutex<ServerConnectionState>>,
    send_stream: SendStream,
    /// CONNECT ストリームの受信半分 (ドロップすると STOP_SENDING が送信されるため保持する)
    _recv_stream: ReceiveStream,
    bidi_acceptor: BidirectionalStreamAcceptor,
    handle: s2n_quic::connection::Handle,
    uni_rx: mpsc::Receiver<WtRecvStream>,
    _control_task: JoinHandle<()>,
    _uni_task: JoinHandle<()>,
}

impl WtSessionRequest {
    /// QUIC 接続から WebTransport セッションリクエストを作成する
    ///
    /// クライアントの SETTINGS を先に受信し、draft バージョンを判定してから
    /// そのバージョンに合わせたサーバー SETTINGS を返す。
    pub async fn from_connection(
        connection: s2n_quic::Connection,
        _h3_settings: H3Settings,
    ) -> Result<Self> {
        let (mut handle, stream_acceptor) = connection.split();
        let (mut bidi_acceptor, mut uni_acceptor) = stream_acceptor.split();

        // ── Phase 1: クライアントの制御ストリームを受信し SETTINGS を取得する ──

        tracing::info!("WebTransport: waiting for client control stream...");
        let mut pending_uni: Vec<(u64, ReceiveStream, Vec<u8>)> = Vec::new();
        let (client_settings, client_control_id, client_control_recv, client_control_buf) = 'find_control: {
            loop {
                let mut recv = uni_acceptor
                    .accept_receive_stream()
                    .await
                    .map_err(Error::transport)?
                    .ok_or(Error::ConnectionClosed)?;
                let sid: u64 = recv.id();
                tracing::debug!("WebTransport: received uni stream {} (0x{:x})", sid, sid);

                let first = recv
                    .receive()
                    .await
                    .map_err(Error::transport)?
                    .ok_or(Error::StreamClosed)?;
                let mut buf = first.to_vec();

                // ストリームタイプを判定する
                let (stream_type, _) = shiguredo_http3::varint::decode(&buf)
                    .map_err(|_| Error::InvalidState("stream type decode error".into()))?;
                let stream_type = stream_type.get();

                tracing::info!("WebTransport: uni stream {} type: 0x{:x}", sid, stream_type);

                if stream_type != 0x00 {
                    // 制御ストリーム以外は保留する
                    tracing::info!(
                        "WebTransport: uni stream {} is not control (type=0x{:x}), pending (raw: {:02x?})",
                        sid,
                        stream_type,
                        &buf[..std::cmp::min(buf.len(), 64)]
                    );
                    pending_uni.push((sid, recv, buf));
                    continue;
                }

                tracing::info!(
                    "WebTransport: found client control stream {} (0x{:x})",
                    sid,
                    sid
                );

                // H3 制御ストリーム: SETTINGS フレームを読み切る
                loop {
                    match parse_settings_from_control(&buf) {
                        Ok(settings) => break 'find_control (settings, sid, recv, buf),
                        Err(SettingsParseState::NeedMoreData) => {
                            tracing::debug!(
                                "WebTransport: control stream {}: need more data for SETTINGS ({} bytes so far)",
                                sid,
                                buf.len()
                            );
                            let more = recv
                                .receive()
                                .await
                                .map_err(Error::transport)?
                                .ok_or(Error::StreamClosed)?;
                            buf.extend_from_slice(&more);
                        }
                        Err(SettingsParseState::Error(msg)) => {
                            return Err(Error::InvalidState(msg));
                        }
                    }
                }
            }
        };

        // shiguredo_http3 のドラフト検出 (複合 SETTINGS はクレート側の優先順位に従う)
        let draft = client_settings
            .webtransport_draft_pattern()
            .unwrap_or_else(|| {
                tracing::warn!(
                    "WebTransport: no recognized draft in client SETTINGS, fallback to draft-02"
                );
                DraftVersion::Draft02
            });
        tracing::info!("WebTransport: client draft detected: {draft:?}");

        // ── Phase 2: draft に合わせたサーバー SETTINGS を構築する ──

        let server_wt = build_server_wt_settings(draft);
        let server_settings = H3Settings::default().enable_webtransport_server(server_wt);
        tracing::info!("WebTransport: server SETTINGS: {server_settings:?}");
        let state = Arc::new(StdMutex::new(ServerConnectionState::new(server_settings)));

        // ── Phase 3: サーバー H3 ストリームを初期化して SETTINGS を送信する ──

        tracing::info!("WebTransport: initializing H3 streams...");
        let mut control_send = handle.open_send_stream().await.map_err(Error::transport)?;
        let mut encoder_send = handle.open_send_stream().await.map_err(Error::transport)?;
        let mut decoder_send = handle.open_send_stream().await.map_err(Error::transport)?;

        let init_data = {
            let mut s = state.lock().unwrap();
            let data = s.h3_conn.init_h3_streams(
                control_send.id(),
                encoder_send.id(),
                decoder_send.id(),
            )?;
            s.set_control_stream_id(control_send.id());
            data
        };

        tracing::info!(
            "WebTransport: server control stream data ({} bytes): {:02x?}",
            init_data.control_data.len(),
            init_data.control_data
        );
        control_send
            .send(Bytes::from(init_data.control_data))
            .await?;
        encoder_send
            .send(Bytes::from(init_data.encoder_data))
            .await?;
        decoder_send
            .send(Bytes::from(init_data.decoder_data))
            .await?;
        tracing::info!("WebTransport: H3 streams initialized (control, encoder, decoder)");

        // QUIC transport parameter レベルの前提条件を注入する
        // s2n-quic は DATAGRAM (RFC 9221) をサポートするため max_datagram_frame_size > 0
        // reset_stream_at はピアがサポートしている場合のみ true (draft-02 では不要)
        {
            let mut s = state.lock().unwrap();
            // TODO: ピアの transport parameters から reset_stream_at サポートを確認する
            let reset_stream_at_supported = false;
            // s2n-quic は DATAGRAM (RFC 9221) をサポートするため max_datagram_frame_size > 0
            s.h3_conn
                .set_webtransport_transport_verified(true, reset_stream_at_supported)
                .map_err(|e| Error::transport(format!("transport verify failed: {e:?}")))?;
        }
        tracing::info!("WebTransport: transport parameters verified");

        // ── Phase 4: クライアントの制御ストリームデータを H3 に注入する ──

        let delayed_control_data = {
            let mut s = state.lock().unwrap();
            s.feed_stream_only(client_control_id, &client_control_buf, false)?;
            for event in s.drain_events()? {
                tracing::info!("WebTransport: client control stream event: {event:?}");
            }
            // サーバー SETTINGS はクライアント SETTINGS 受信後に初めてエンキューされる
            // (shiguredo_http3 の WebTransport サーバーはピアの draft を見て SETTINGS を組み立てるため)
            s.take_control_pending()
        };
        tracing::debug!("WebTransport: client control stream data injected into H3 state machine");

        if let Some(data) = delayed_control_data {
            tracing::info!(
                "WebTransport: sending delayed server control SETTINGS ({} bytes): {:02x?}",
                data.len(),
                data
            );
            if !data.is_empty() {
                control_send.send(Bytes::from(data)).await?;
            }
        }

        // ── Phase 5: バックグラウンドタスクを開始する ──

        // 制御ストリームと QPACK ストリームを接続中保持する
        // (クローズすると H3_CLOSED_CRITICAL_STREAM エラーになる)
        let control_task = tokio::spawn(async move {
            let _control_send = control_send;
            let _encoder_send = encoder_send;
            let _decoder_send = decoder_send;
            std::future::pending::<()>().await;
        });

        let unblock_notify = Arc::new(Notify::new());
        let (uni_tx, uni_rx) = mpsc::channel::<WtRecvStream>(16);

        // 単方向ストリーム受信タスク
        let state_for_uni = Arc::clone(&state);
        let notify_for_uni = Arc::clone(&unblock_notify);
        let uni_task = tokio::spawn(async move {
            // クライアント制御ストリームの残りデータを処理する
            {
                let state = Arc::clone(&state_for_uni);
                let notify = Arc::clone(&notify_for_uni);
                let cid = client_control_id;
                tokio::spawn(async move {
                    let mut recv = client_control_recv;
                    while let Ok(Some(data)) = recv.receive().await {
                        tracing::debug!(
                            "WebTransport: client control stream {} received {} bytes",
                            cid,
                            data.len()
                        );
                        {
                            let mut s = state.lock().unwrap();
                            let _ = s.feed_stream_only(cid, &data, false);
                        }
                        notify.notify_one();
                    }
                    tracing::debug!("WebTransport: client control stream {} closed", cid);
                    {
                        let mut s = state.lock().unwrap();
                        let _ = s.feed_stream_only(cid, &[], true);
                    }
                    notify.notify_one();
                });
            }

            // 保留中の uni ストリームを処理する
            tracing::debug!(
                "WebTransport: processing {} pending uni streams",
                pending_uni.len()
            );
            for (stream_id, recv_stream, initial_data) in pending_uni {
                let state = Arc::clone(&state_for_uni);
                let notify = Arc::clone(&notify_for_uni);
                let uni_tx = uni_tx.clone();
                tokio::spawn(route_uni_stream_with_initial(
                    stream_id,
                    recv_stream,
                    initial_data,
                    state,
                    notify,
                    uni_tx,
                ));
            }

            // 新しいストリームを受け付ける
            while let Ok(Some(recv_stream)) = uni_acceptor.accept_receive_stream().await {
                let state = Arc::clone(&state_for_uni);
                let notify = Arc::clone(&notify_for_uni);
                let uni_tx = uni_tx.clone();
                let stream_id: u64 = recv_stream.id();
                tracing::info!(
                    "WebTransport: new uni stream {} (0x{:x})",
                    stream_id,
                    stream_id
                );
                tokio::spawn(route_uni_stream(
                    stream_id,
                    recv_stream,
                    state,
                    notify,
                    uni_tx,
                ));
            }
            tracing::debug!("WebTransport: uni stream acceptor closed");
        });

        // ── Phase 6: 最初の双方向ストリーム (CONNECT リクエスト) を受け付ける ──

        tracing::info!("WebTransport: waiting for CONNECT bidi stream...");
        let stream: s2n_quic::stream::BidirectionalStream = bidi_acceptor
            .accept_bidirectional_stream()
            .await
            .map_err(Error::transport)?
            .ok_or(Error::ConnectionClosed)?;

        let connect_stream_id: u64 = stream.id();
        tracing::info!(
            "WebTransport: received CONNECT bidi stream {} (0x{:x})",
            connect_stream_id,
            connect_stream_id
        );
        let (mut recv_stream, send_stream) = stream.split();

        {
            let mut s = state.lock().unwrap();
            let _ = s.h3_conn.feed_stream(connect_stream_id, &[], false);
            for event in s.drain_events()? {
                tracing::debug!("WebTransport: bidi stream init event: {event:?}");
            }
        }

        // CONNECT リクエストを受信する
        let mut collected_headers: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        let mut headers_complete = false;

        tracing::info!("WebTransport: waiting for CONNECT headers...");
        while !headers_complete {
            // QPACK エンコーダーストリームのデータが先に処理されて
            // notify_one() が取りこぼされる場合に備え、毎回 drain_events を確認する
            {
                let events = state.lock().unwrap().drain_events()?;
                for event in events {
                    match event {
                        Event::Header { name, value, .. } => {
                            let n = String::from_utf8_lossy(&name);
                            let v = String::from_utf8_lossy(&value);
                            tracing::info!("WebTransport: CONNECT header: {n}: {v}");
                            collected_headers.push((name, value));
                        }
                        Event::HeadersEnd { .. } => {
                            tracing::info!("WebTransport: CONNECT headers complete");
                            headers_complete = true;
                        }
                        other => {
                            tracing::debug!("WebTransport: event: {other:?}");
                        }
                    }
                }
                if headers_complete {
                    break;
                }
            }

            tokio::select! {
                received = recv_stream.receive() => {
                    let (data, fin) = match received {
                        Ok(Some(data)) => {
                            tracing::info!(
                                "WebTransport: received {} bytes on CONNECT stream",
                                data.len()
                            );
                            tracing::info!(
                                "WebTransport: CONNECT stream data: {:02x?}",
                                data.as_ref()
                            );
                            (data.to_vec(), false)
                        }
                        Ok(None) => {
                            tracing::debug!("WebTransport: CONNECT stream ended (FIN)");
                            (vec![], true)
                        }
                        Err(e) => {
                            tracing::error!(
                                "WebTransport: CONNECT stream receive error: {e:?}"
                            );
                            return Err(Error::transport(e));
                        }
                    };
                    let events = {
                        let mut s = state.lock().unwrap();
                        s.process_stream_data(connect_stream_id, &data, fin)?
                    };
                    for event in events {
                        match event {
                            Event::Header { name, value, .. } => {
                                let n = String::from_utf8_lossy(&name);
                                let v = String::from_utf8_lossy(&value);
                                tracing::info!("WebTransport: CONNECT header: {n}: {v}");
                                collected_headers.push((name, value));
                            }
                            Event::HeadersEnd { .. } => {
                                tracing::info!("WebTransport: CONNECT headers complete");
                                headers_complete = true;
                            }
                            other => {
                                tracing::debug!("WebTransport: event: {other:?}");
                            }
                        }
                    }
                    if fin { break; }
                }
                _ = unblock_notify.notified() => {}
                _ = tokio::time::sleep(std::time::Duration::from_millis(10)) => {}
            }
        }

        // ConnectRequest::from_headers() で検証付きパースする
        let header_refs: Vec<(&[u8], &[u8])> = collected_headers
            .iter()
            .map(|(n, v)| (n.as_slice(), v.as_slice()))
            .collect();
        let connect_request = ConnectRequest::from_headers(&header_refs)
            .map_err(|e| Error::Internal(format!("invalid CONNECT request: {e}")))?;

        tracing::info!(
            "WebTransport: CONNECT request parsed: path={}, authority={}",
            connect_request.path,
            connect_request.authority
        );

        Ok(Self {
            path: connect_request.path,
            authority: connect_request.authority,
            draft,
            stream_id: connect_stream_id,
            state,
            send_stream,
            _recv_stream: recv_stream,
            bidi_acceptor,
            handle,
            uni_rx,
            _control_task: control_task,
            _uni_task: uni_task,
        })
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn authority(&self) -> &str {
        &self.authority
    }

    pub fn draft(&self) -> DraftVersion {
        self.draft
    }

    /// セッションリクエストを受け入れる
    pub async fn accept(self) -> Result<WtSession> {
        tracing::info!("WebTransport: sending 200 response for CONNECT...");
        let response_headers = ConnectResponse::new(200).to_headers();

        let data = {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            s.h3_conn
                .send_response(self.stream_id, &response_headers, false)?;
            let mut all_data = Vec::new();
            while let Some((chunk, _fin)) = s.h3_conn.take_stream_data(self.stream_id) {
                all_data.extend_from_slice(&chunk);
            }
            all_data
        };

        let mut send_stream = self.send_stream;
        send_stream
            .send(Bytes::from(data))
            .await
            .map_err(Error::transport)?;
        tracing::info!("WebTransport: session accepted (200 response sent)");

        Ok(WtSession {
            session_id: self.stream_id,
            draft: self.draft,
            bidi_acceptor: self.bidi_acceptor,
            handle: self.handle,
            connect_send: send_stream,
            _connect_recv: self._recv_stream,
            uni_rx: self.uni_rx,
        })
    }

    /// セッションリクエストを拒否する (4xx / 5xx)
    ///
    /// CONNECT ストリームにエラーレスポンスを送り、送信側を終了する。
    pub async fn reject(self, status: u16) -> Result<()> {
        if !(400..=599).contains(&status) {
            return Err(Error::InvalidState(format!(
                "reject status must be 4xx or 5xx, got {status}"
            )));
        }
        let response_headers = ConnectResponse::new(status).to_headers();
        let data = {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            s.h3_conn
                .send_response(self.stream_id, &response_headers, true)?;
            let mut all_data = Vec::new();
            while let Some((chunk, _fin)) = s.h3_conn.take_stream_data(self.stream_id) {
                all_data.extend_from_slice(&chunk);
            }
            all_data
        };
        let mut send_stream = self.send_stream;
        send_stream
            .send(Bytes::from(data))
            .await
            .map_err(Error::transport)?;
        send_stream.finish().map_err(Error::transport)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// WebTransport セッション
// ---------------------------------------------------------------------------

pub struct WtSession {
    session_id: u64,
    draft: DraftVersion,
    bidi_acceptor: BidirectionalStreamAcceptor,
    handle: s2n_quic::connection::Handle,
    connect_send: SendStream,
    /// CONNECT ストリームの受信半分 (ドロップすると STOP_SENDING が送信されるため保持する)
    _connect_recv: ReceiveStream,
    uni_rx: mpsc::Receiver<WtRecvStream>,
}

/// WtSession を分解した各コンポーネント
///
/// bidi acceptor と uni receiver を独立に使用し、デッドロックを防ぐ。
pub struct WtSessionParts {
    pub session_id: u64,
    pub draft: DraftVersion,
    pub handle: s2n_quic::connection::Handle,
    pub bidi_acceptor: BidirectionalStreamAcceptor,
    pub uni_rx: mpsc::Receiver<WtRecvStream>,
    /// CONNECT ストリーム (ドロップ防止のため保持する)
    pub _connect_send: SendStream,
    pub _connect_recv: ReceiveStream,
}

impl WtSession {
    pub fn session_id(&self) -> u64 {
        self.session_id
    }

    pub fn draft(&self) -> DraftVersion {
        self.draft
    }

    /// セッションを独立したコンポーネントに分解する
    ///
    /// bidi acceptor と uni receiver を個別に使用するため、
    /// tokio::select! での二重借用を回避する。
    pub fn into_parts(self) -> WtSessionParts {
        WtSessionParts {
            session_id: self.session_id,
            draft: self.draft,
            handle: self.handle,
            bidi_acceptor: self.bidi_acceptor,
            uni_rx: self.uni_rx,
            _connect_send: self.connect_send,
            _connect_recv: self._connect_recv,
        }
    }

    /// 単方向送信ストリームを開く
    pub async fn open_uni_stream(&mut self) -> Result<WtSendStream> {
        let stream = self
            .handle
            .open_send_stream()
            .await
            .map_err(Error::transport)?;
        let stream_id: u64 = stream.id();
        let mut send = stream;

        let mut header = Vec::new();
        StreamHeader::new(self.session_id)
            .expect("session_id must be a client-initiated bidirectional stream id")
            .encode_unidirectional(&mut header);
        send.send(Bytes::from(header))
            .await
            .map_err(Error::transport)?;

        tracing::debug!(
            "WebTransport: opened uni send stream {} (0x{:x})",
            stream_id,
            stream_id
        );

        Ok(WtSendStream { stream_id, send })
    }

    /// 単方向受信ストリームを受け付ける
    pub async fn accept_uni_stream(&mut self) -> Result<WtRecvStream> {
        let stream = self.uni_rx.recv().await.ok_or(Error::StreamClosed)?;
        tracing::debug!(
            "WebTransport: accepted uni recv stream {} (0x{:x})",
            stream.stream_id(),
            stream.stream_id()
        );
        Ok(stream)
    }

    /// 双方向ストリームを受け付ける
    pub async fn accept_bi_stream(&mut self) -> Result<WtBiStream> {
        let stream: s2n_quic::stream::BidirectionalStream = self
            .bidi_acceptor
            .accept_bidirectional_stream()
            .await
            .map_err(Error::transport)?
            .ok_or(Error::StreamClosed)?;

        let stream_id: u64 = stream.id();
        let (mut recv, send) = stream.split();

        let mut header_buf: Vec<u8> = Vec::new();
        let pending = loop {
            let data = recv
                .receive()
                .await
                .map_err(Error::transport)?
                .ok_or(Error::StreamClosed)?;
            header_buf.extend_from_slice(&data);
            match StreamHeader::decode_bidirectional_checked(&header_buf) {
                Ok((_, consumed)) => break header_buf[consumed..].to_vec(),
                Err(StreamHeaderDecodeError::BufferTooShort) => continue,
                Err(e) => return Err(Error::Internal(format!("bidi header decode error: {e:?}"))),
            }
        };

        tracing::debug!(
            "WebTransport: accepted bidi stream {} (0x{:x})",
            stream_id,
            stream_id
        );

        Ok(WtBiStream {
            stream_id,
            recv,
            send,
            pending,
        })
    }

    /// 双方向ストリームを開く
    pub async fn open_bi_stream(&mut self) -> Result<WtBiStream> {
        let stream = self
            .handle
            .open_bidirectional_stream()
            .await
            .map_err(Error::transport)?;
        let stream_id: u64 = stream.id();
        let (recv, mut send) = stream.split();

        let mut header = Vec::new();
        StreamHeader::new(self.session_id)
            .expect("session_id must be a client-initiated bidirectional stream id")
            .encode_bidirectional(&mut header);
        send.send(Bytes::from(header))
            .await
            .map_err(Error::transport)?;

        tracing::debug!(
            "WebTransport: opened bidi stream {} (0x{:x})",
            stream_id,
            stream_id
        );

        Ok(WtBiStream {
            stream_id,
            recv,
            send,
            pending: Vec::new(),
        })
    }

    /// セッションをクローズする
    pub async fn close(&mut self, code: u32, reason: &str) -> Result<()> {
        tracing::info!(
            "WebTransport: closing session (code={}, reason={})",
            code,
            reason
        );
        let capsule = Capsule::CloseSession {
            error_code: code,
            message: reason.to_string(),
        };
        let mut buf = Vec::new();
        capsule.encode(&mut buf);
        self.connect_send
            .send(Bytes::from(buf))
            .await
            .map_err(Error::transport)
    }
}

// ---------------------------------------------------------------------------
// ストリーム型
// ---------------------------------------------------------------------------

pub struct WtSendStream {
    stream_id: u64,
    send: SendStream,
}

impl WtSendStream {
    pub fn new(stream_id: u64, send: SendStream) -> Self {
        Self { stream_id, send }
    }

    pub async fn send(&mut self, data: &[u8]) -> Result<()> {
        self.send
            .send(Bytes::copy_from_slice(data))
            .await
            .map_err(Error::transport)
    }

    pub fn finish(&mut self) -> Result<()> {
        self.send.finish().map_err(Error::transport)
    }

    pub fn stream_id(&self) -> u64 {
        self.stream_id
    }
}

pub struct WtRecvStream {
    stream_id: u64,
    recv: ReceiveStream,
    pending: Vec<u8>,
}

impl WtRecvStream {
    pub fn new(stream_id: u64, recv: ReceiveStream, pending: Vec<u8>) -> Self {
        Self {
            stream_id,
            recv,
            pending,
        }
    }

    pub async fn recv(&mut self) -> Result<Vec<u8>> {
        if !self.pending.is_empty() {
            return Ok(std::mem::take(&mut self.pending));
        }
        match self.recv.receive().await {
            Ok(Some(data)) => Ok(data.to_vec()),
            Ok(None) => Err(Error::StreamClosed),
            Err(e) => Err(Error::transport(e)),
        }
    }

    pub fn stream_id(&self) -> u64 {
        self.stream_id
    }
}

pub struct WtBiStream {
    stream_id: u64,
    recv: ReceiveStream,
    send: SendStream,
    pending: Vec<u8>,
}

impl WtBiStream {
    /// 送信と受信のストリームから双方向ストリームを構築する
    pub fn from_parts(send: WtSendStream, recv: WtRecvStream) -> Self {
        Self {
            stream_id: send.stream_id,
            recv: recv.recv,
            send: send.send,
            pending: recv.pending,
        }
    }

    /// 双方向ストリームを送信側と受信側に分解する
    pub fn into_parts(self) -> (WtSendStream, WtRecvStream) {
        (
            WtSendStream {
                stream_id: self.stream_id,
                send: self.send,
            },
            WtRecvStream::new(self.stream_id, self.recv, self.pending),
        )
    }

    pub async fn send(&mut self, data: &[u8]) -> Result<()> {
        self.send
            .send(Bytes::copy_from_slice(data))
            .await
            .map_err(Error::transport)
    }

    pub async fn recv(&mut self) -> Result<Vec<u8>> {
        if !self.pending.is_empty() {
            return Ok(std::mem::take(&mut self.pending));
        }
        match self.recv.receive().await {
            Ok(Some(data)) => Ok(data.to_vec()),
            Ok(None) => Err(Error::StreamClosed),
            Err(e) => Err(Error::transport(e)),
        }
    }

    pub fn finish(&mut self) -> Result<()> {
        self.send.finish().map_err(Error::transport)
    }

    pub fn stream_id(&self) -> u64 {
        self.stream_id
    }
}

// ---------------------------------------------------------------------------
// SETTINGS パースと draft 判定
// ---------------------------------------------------------------------------

/// SETTINGS パース中間状態
enum SettingsParseState {
    NeedMoreData,
    Error(String),
}

/// H3 制御ストリームのバッファから SETTINGS をパースする
///
/// buf にはストリームタイプバイト (0x00) を含む全データが入っている。
fn parse_settings_from_control(buf: &[u8]) -> std::result::Result<H3Settings, SettingsParseState> {
    // ストリームタイプバイト (0x00) をスキップする
    let (stream_type, type_len) =
        shiguredo_http3::varint::decode(buf).map_err(|_| SettingsParseState::NeedMoreData)?;
    let stream_type = stream_type.get();
    if stream_type != 0x00 {
        return Err(SettingsParseState::Error(format!(
            "expected control stream type 0x00, got {stream_type:#x}"
        )));
    }
    let rest = &buf[type_len..];

    // フレームタイプを読む
    let (frame_type, ft_len) =
        shiguredo_http3::varint::decode(rest).map_err(|_| SettingsParseState::NeedMoreData)?;
    let frame_type = frame_type.get();
    if frame_type != 0x04 {
        return Err(SettingsParseState::Error(format!(
            "expected SETTINGS frame type 0x04, got {frame_type:#x}"
        )));
    }

    // フレーム長を読む
    let (payload_len, pl_len) = shiguredo_http3::varint::decode(&rest[ft_len..])
        .map_err(|_| SettingsParseState::NeedMoreData)?;
    let payload_len = payload_len.get();
    let payload_start = ft_len + pl_len;
    let payload_end = payload_start + payload_len as usize;
    if rest.len() < payload_end {
        return Err(SettingsParseState::NeedMoreData);
    }

    // SETTINGS ペイロードをパースする
    let payload_buf = &rest[payload_start..payload_end];
    tracing::info!(
        "WebTransport: client SETTINGS payload ({} bytes): {:02x?}",
        payload_buf.len(),
        payload_buf
    );
    let mut payload = SettingsPayload::new();
    let mut pos = 0;
    while pos < payload_buf.len() {
        let (id, id_len) = shiguredo_http3::varint::decode(&payload_buf[pos..])
            .map_err(|_| SettingsParseState::Error("settings id decode error".into()))?;
        pos += id_len;
        let (value, value_len) = shiguredo_http3::varint::decode(&payload_buf[pos..])
            .map_err(|_| SettingsParseState::Error("settings value decode error".into()))?;
        pos += value_len;
        let id = id.get();
        let value = value.get();
        tracing::info!("WebTransport: client SETTINGS entry: id=0x{id:x}, value={value}");
        payload.add(id, value);
    }

    let settings = H3Settings::from_payload(&payload)
        .map_err(|e| SettingsParseState::Error(format!("settings parse error: {e}")))?;
    tracing::info!("WebTransport: client SETTINGS parsed: {settings:?}");
    Ok(settings)
}

/// draft バージョンに合わせたサーバー WT 設定を構築する
fn build_server_wt_settings(draft: DraftVersion) -> shiguredo_http3::webtransport::Settings {
    tracing::info!("WebTransport: building server settings for {draft:?}");
    match draft {
        DraftVersion::Draft15 => shiguredo_http3::webtransport::Settings::new()
            .wt_enabled(1)
            .wt_initial_max_streams_uni(1000)
            .wt_initial_max_streams_bidi(1000),
        // Draft-14: Safari 26.4 は 0xc671706a (draft-07) と 0x14e9cd29 (draft-14) の両方を送る。
        // サーバーも両方返すことで、どちらの ID で判定しても WebTransport 対応と認識させる。
        DraftVersion::Draft14 => shiguredo_http3::webtransport::Settings::new()
            .wt_max_sessions_draft14(100)
            .webtransport_max_sessions_draft07(100)
            .wt_initial_max_streams_uni(1000)
            .wt_initial_max_streams_bidi(1000)
            .wt_initial_max_data(8 * 1024 * 1024),
        // Draft-07: Safari 26.4 は応答 SETTINGS に draft-14 系の
        // WT_INITIAL_MAX_STREAMS_* / WT_INITIAL_MAX_DATA を含めると
        // H3_REQUEST_CANCELLED (0x10C) で CONNECT をリセットする。
        // (docs/SAFARI_WT.md 参照) 初期フロー制御値はセッション確立後の
        // WT_MAX_STREAMS / WT_MAX_DATA カプセルで通知する。
        DraftVersion::Draft07 => {
            shiguredo_http3::webtransport::Settings::new().webtransport_max_sessions_draft07(100)
        }
        DraftVersion::Draft02 => {
            shiguredo_http3::webtransport::Settings::new().enable_webtransport_draft02(true)
        }
    }
}

// ---------------------------------------------------------------------------
// 単方向ストリームルーティング
// ---------------------------------------------------------------------------

/// 初期データ付きの単方向ストリームルーティング
///
/// from_connection で制御ストリーム検出前に受信した uni ストリームを処理する。
async fn route_uni_stream_with_initial(
    stream_id: u64,
    mut recv_stream: ReceiveStream,
    initial_data: Vec<u8>,
    state: Arc<StdMutex<ServerConnectionState>>,
    notify: Arc<Notify>,
    uni_tx: mpsc::Sender<WtRecvStream>,
) {
    let mut type_buf = initial_data;

    let classified = loop {
        match classify_uni_stream_checked(&type_buf) {
            Ok(result) => break Some(result),
            Err(StreamHeaderDecodeError::BufferTooShort) => {}
            Err(e) => {
                tracing::warn!(
                    "WebTransport: uni stream {} classification error: {e:?}",
                    stream_id
                );
                return;
            }
        }
        match recv_stream.receive().await {
            Ok(Some(data)) => type_buf.extend_from_slice(&data),
            Ok(None) => {
                let _ = state.lock().unwrap().feed_stream_only(stream_id, &[], true);
                notify.notify_one();
                return;
            }
            Err(_) => return,
        }
    };

    route_classified_stream(
        stream_id,
        recv_stream,
        type_buf,
        classified,
        state,
        notify,
        uni_tx,
    )
    .await;
}

async fn route_uni_stream(
    stream_id: u64,
    mut recv_stream: ReceiveStream,
    state: Arc<StdMutex<ServerConnectionState>>,
    notify: Arc<Notify>,
    uni_tx: mpsc::Sender<WtRecvStream>,
) {
    let mut type_buf: Vec<u8> = Vec::new();

    let classified = loop {
        match recv_stream.receive().await {
            Ok(Some(data)) => {
                type_buf.extend_from_slice(&data);
                match classify_uni_stream_checked(&type_buf) {
                    Ok(result) => break Some(result),
                    Err(StreamHeaderDecodeError::BufferTooShort) => continue,
                    Err(e) => {
                        tracing::warn!(
                            "WebTransport: uni stream {} classification error: {e:?}",
                            stream_id
                        );
                        return;
                    }
                }
            }
            Ok(None) => {
                let _ = state.lock().unwrap().feed_stream_only(stream_id, &[], true);
                notify.notify_one();
                return;
            }
            Err(_) => return,
        }
    };

    route_classified_stream(
        stream_id,
        recv_stream,
        type_buf,
        classified,
        state,
        notify,
        uni_tx,
    )
    .await;
}

/// 分類済みストリームを処理する共通関数
async fn route_classified_stream(
    stream_id: u64,
    mut recv_stream: ReceiveStream,
    type_buf: Vec<u8>,
    classified: Option<ClassifiedUniStream>,
    state: Arc<StdMutex<ServerConnectionState>>,
    notify: Arc<Notify>,
    uni_tx: mpsc::Sender<WtRecvStream>,
) {
    match classified {
        Some(ClassifiedUniStream::WebTransport { data_offset, .. }) => {
            tracing::info!(
                "WebTransport: uni stream {} classified as WebTransport (data_offset={})",
                stream_id,
                data_offset
            );
            let pending = type_buf[data_offset..].to_vec();
            let wt_recv = WtRecvStream::new(stream_id, recv_stream, pending);
            let _ = uni_tx.send(wt_recv).await;
        }
        _ => {
            tracing::info!(
                "WebTransport: uni stream {} classified as H3 stream, raw: {:02x?}",
                stream_id,
                &type_buf[..std::cmp::min(type_buf.len(), 64)]
            );
            {
                let mut s = state.lock().unwrap();
                let _ = s.feed_stream_only(stream_id, &type_buf, false);
                // drain_events は呼ばない: QPACK ブロック解除は drain_events 内で
                // 遅延実行される。ここで drain すると CONNECT ヘッダー待ちループが
                // イベントを取得できなくなる。
            }
            notify.notify_one();
            while let Ok(Some(data)) = recv_stream.receive().await {
                {
                    let mut s = state.lock().unwrap();
                    let _ = s.feed_stream_only(stream_id, &data, false);
                }
                notify.notify_one();
            }
            {
                let mut s = state.lock().unwrap();
                let _ = s.feed_stream_only(stream_id, &[], true);
            }
            notify.notify_one();
        }
    }
}
