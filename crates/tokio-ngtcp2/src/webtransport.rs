//! shiguredo_ngtcp2_tokio 上の WebTransport クライアント / サーバー
//!
//! HTTP/3 層 (shiguredo_http3) の WebTransport イベントと、QUIC 層
//! (shiguredo_ngtcp2_tokio) のストリーム / DATAGRAM を対応付ける。

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use shiguredo_http3::webtransport::{ConnectRequest, ConnectResponse, StreamHeader};
use shiguredo_http3::{Event, Settings, VarInt, WebTransportEvent};
use shiguredo_ngtcp2_tokio::ConnectionId;

use crate::client::Client;
use crate::h3::QuicConnectionRef;
use crate::server::{ConnectionState, Server};
use crate::{Error, Result};

/// WebTransport 用の HTTP/3 設定を作成する
///
/// draft-02 / draft-07 / draft-15 の SETTINGS を同時に送信する。
/// WT_INITIAL_MAX_* を送らないため WebTransport のフロー制御は無効になり、
/// ストリーム数とデータ量は QUIC のフロー制御だけで制限される
/// (相互運用テストではフロー制御カプセルのやり取りを検証しない)。
fn wt_settings() -> shiguredo_http3::webtransport::Settings {
    shiguredo_http3::webtransport::Settings::new()
        .wt_enabled(VarInt::from_static(1))
        .enable_webtransport_draft02(true)
        .webtransport_max_sessions_draft07(VarInt::from_static(1))
}

/// WebTransport クライアント用の HTTP/3 設定
fn client_settings() -> Settings {
    Settings::default().enable_webtransport_client(wt_settings())
}

/// WebTransport サーバー用の HTTP/3 設定
fn server_settings() -> Settings {
    Settings::default().enable_webtransport_server(wt_settings())
}

/// WebTransport クライアント
pub struct ClientWebTransportSession {
    /// HTTP/3 クライアント
    client: Client,
    /// 確立済みセッション ID
    session_id: Option<u64>,
}

impl ClientWebTransportSession {
    /// 証明書検証を行わずに接続する
    pub async fn connect_insecure(
        remote_addr: SocketAddr,
        server_name: &str,
        _path: &str,
    ) -> Result<Self> {
        let client =
            Client::connect_with_settings(remote_addr, server_name, client_settings(), true)
                .await?;
        Ok(Self {
            client,
            session_id: None,
        })
    }

    /// 確立済みセッション ID
    pub fn session_id(&self) -> Option<u64> {
        self.session_id
    }

    /// ピアの SETTINGS を受信するまで待つ
    pub async fn handshake(&mut self) -> Result<()> {
        self.client.handshake().await?;
        self.client.set_webtransport_transport_verified()
    }

    /// WebTransport セッションを開始する
    pub async fn open_session(&mut self, authority: &str, path: &str) -> Result<u64> {
        // ピアが対応するドラフトに合わせて `:protocol` 疑似ヘッダーを変える
        // (draft-02/07/14 は "webtransport"、draft-15 は "webtransport-h3")
        let mut request = ConnectRequest::new("https", authority, path);
        if let Some(draft) = self
            .client
            .peer_settings()
            .and_then(|settings| settings.webtransport_draft_pattern())
        {
            request = request.draft_version(draft);
        }
        let headers = request
            .to_headers()
            .map_err(|e| Error::InvalidArgument(format!("invalid CONNECT headers: {e}")))?;
        self.client.send_request_streaming(&headers)?;

        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            self.client.flush().await?;
            self.client.recv(Duration::from_millis(50)).await?;
            while let Some(event) = self.client.poll() {
                match &event {
                    Event::WebTransport(WebTransportEvent::SessionEstablished {
                        session_id,
                        ..
                    }) => {
                        self.session_id = Some(*session_id);
                        self.deliver_capsules().await?;
                        return Ok(*session_id);
                    }
                    Event::WebTransport(WebTransportEvent::SessionClosed {
                        close_error_code,
                        close_message,
                        ..
                    }) => {
                        return Err(Error::WebTransportClosed {
                            error_code: *close_error_code,
                            message: close_message.clone(),
                        });
                    }
                    _ => {}
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(Error::Timeout);
            }
        }
    }

    /// 新しい双方向ストリームを開く
    pub fn open_bidi_stream(&mut self) -> Result<u64> {
        let session_id = self.session_id.ok_or(Error::InvalidState(
            "WebTransport session is not established",
        ))?;
        let stream_id = self.client.open_bidi_stream()?;
        self.client
            .register_local_wt_stream(session_id, stream_id)?;
        let mut header = Vec::new();
        StreamHeader::new(session_id)
            .map_err(|_| Error::InvalidState("invalid WebTransport session id"))?
            .encode_bidirectional(&mut header);
        self.client.write_wt_stream(stream_id, &header, false)?;
        Ok(stream_id)
    }

    /// WebTransport ストリームにデータを送信する
    pub async fn send_stream_data(&mut self, stream_id: u64, data: &[u8], fin: bool) -> Result<()> {
        self.client.write_wt_stream(stream_id, data, fin)?;
        // フロー制御と輻輳制御で送りきれなかったデータが残っている間は、
        // 送信と受信 (ACK / MAX_STREAM_DATA の処理) を繰り返す。
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            self.client.flush().await?;
            if !self.client.has_pending_data() {
                self.deliver_capsules().await?;
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(Error::Timeout);
            }
            self.client.recv(Duration::from_millis(5)).await?;
        }
    }

    /// WebTransport データグラムを送信する
    ///
    /// ピアが DATAGRAM 非対応の場合は `false` を返す。
    pub async fn send_datagram(&mut self, data: &[u8]) -> Result<bool> {
        let Some(session_id) = self.session_id else {
            return Ok(false);
        };
        self.client.send_wt_datagram(session_id, data).await
    }

    /// 受信済みのイベントを 1 つ取り出す
    pub fn poll(&mut self) -> Option<Event> {
        let event = self.client.poll()?;
        // アプリケーションが処理したものとしてフロー制御クレジットを戻す
        if let Event::WebTransport(
            WebTransportEvent::BidiStreamData { data, .. }
            | WebTransportEvent::UniStreamData { data, .. },
        ) = &event
            && let Some(session_id) = self.session_id
        {
            self.client.wt_data_consumed(session_id, data.len() as u64);
        }
        Some(event)
    }

    /// `timeout` の間、接続を駆動する
    pub async fn recv(&mut self, timeout: Duration) -> Result<()> {
        self.client.recv(timeout).await?;
        self.deliver_capsules().await
    }

    /// 送信待ちのフロー制御カプセルを CONNECT ストリームへ送信する
    async fn deliver_capsules(&mut self) -> Result<()> {
        let Some(session_id) = self.session_id else {
            return Ok(());
        };
        if !self.client.wt_session_flow_control_enabled(session_id) {
            return Ok(());
        }
        for buf in self.client.take_wt_capsules(session_id) {
            // カプセルは DATA フレームとして CONNECT ストリームへ送る
            self.client.write_wt_stream(session_id, &buf, false)?;
        }
        self.client.flush().await
    }
}

/// CONNECT リクエストのヘッダー (ストリーム ID ごと)
type RequestHeaders = HashMap<u64, Vec<(Vec<u8>, Vec<u8>)>>;

/// 接続 1 本の WebTransport 状態
struct WtConnectionState {
    /// 確立済みセッション ID
    sessions: HashSet<u64>,
    /// ストリーム ID -> セッション ID
    stream_sessions: HashMap<u64, u64>,
    /// CONNECT リクエストのヘッダー
    request_headers: RequestHeaders,
}

impl WtConnectionState {
    /// 新しい状態を作成する
    fn new() -> Self {
        Self {
            sessions: HashSet::new(),
            stream_sessions: HashMap::new(),
            request_headers: HashMap::new(),
        }
    }

    /// セッション ID を解決する
    ///
    /// ストリームに紐づくセッションが無い場合は、確立済みセッションのうち
    /// 最初のものを返す (セッション単位のイベント用)。
    fn resolve_session(&self, stream_id: Option<u64>) -> Option<u64> {
        if let Some(stream_id) = stream_id
            && let Some(session_id) = self.stream_sessions.get(&stream_id)
        {
            return Some(*session_id);
        }
        self.sessions.iter().next().copied()
    }
}

/// WebTransport サーバー
pub struct ServerWebTransportSession {
    /// HTTP/3 サーバー
    server: Server,
    /// 接続ごとの WebTransport 状態
    wt: HashMap<ConnectionId, WtConnectionState>,
}

impl ServerWebTransportSession {
    /// サーバーを起動する
    pub async fn bind(
        addr: SocketAddr,
        cert_path: impl AsRef<Path>,
        key_path: impl AsRef<Path>,
    ) -> Result<Self> {
        let server =
            Server::bind_with_settings(addr, cert_path, key_path, server_settings(), true).await?;
        Ok(Self {
            server,
            wt: HashMap::new(),
        })
    }

    /// ローカルアドレス
    pub fn local_addr(&self) -> SocketAddr {
        self.server.local_addr()
    }

    /// 接続を処理し続ける
    ///
    /// ハンドラーは `(アドレス, セッション ID, イベント)` で呼ばれる。
    /// WebTransport CONNECT の `HeadersEnd` で `true` を返すと 200 応答を送り、
    /// セッションを確立する。
    pub async fn run<F>(&mut self, mut handler: F) -> Result<()>
    where
        F: FnMut(SocketAddr, u64, Event) -> bool,
    {
        loop {
            self.drive_all(&mut handler).await?;
            if self.server.connections.is_empty() {
                if let Some(conn) = self.server.quic.accept().await? {
                    self.add_connection(conn).await?;
                }
            } else {
                match tokio::time::timeout(Duration::from_millis(1), self.server.quic.accept())
                    .await
                {
                    Ok(Ok(Some(conn))) => self.add_connection(conn).await?,
                    Ok(Ok(None)) => {}
                    Ok(Err(e)) => return Err(e.into()),
                    Err(_) => {}
                }
            }
        }
    }

    /// `timeout` の間だけ接続を駆動する
    pub async fn recv_once<F>(&mut self, timeout: Duration, handler: &mut F) -> Result<()>
    where
        F: FnMut(SocketAddr, u64, Event) -> bool,
    {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            self.drive_all(handler).await?;
            let now = tokio::time::Instant::now();
            if now >= deadline {
                break;
            }
            let remaining = deadline - now;
            let wait = remaining.min(Duration::from_millis(1));
            match tokio::time::timeout(wait, self.server.quic.accept()).await {
                Ok(Ok(Some(conn))) => self.add_connection(conn).await?,
                Ok(Ok(None)) => {}
                Ok(Err(e)) => return Err(e.into()),
                Err(_) => {}
            }
        }
        Ok(())
    }

    /// 送信待ちのデータを書き出す
    pub async fn flush(&mut self) -> Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let mut pending = false;
            for state in self.server.connections.values_mut() {
                let mut quic = QuicConnectionRef::Server(&mut state.conn);
                state.h3.pump(&mut quic).await?;
                pending |= quic.has_pending_data();
            }
            if !pending {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(Error::Timeout);
            }
            for state in self.server.connections.values_mut() {
                let mut quic = QuicConnectionRef::Server(&mut state.conn);
                state
                    .h3
                    .pump_wait(&mut quic, Duration::from_millis(5))
                    .await?;
            }
        }
    }

    /// 新しい双方向ストリームを開く
    pub fn open_bidi_stream_for(&mut self, addr: &SocketAddr) -> Result<u64> {
        let conn_id = self
            .server
            .connections
            .values()
            .find(|state| state.addr == *addr)
            .map(|state| state.conn_id.clone())
            .ok_or(Error::InvalidState("connection not found"))?;
        self.open_bidi_stream_inner(&conn_id)
    }

    /// WebTransport ストリームにデータを送信する
    pub fn send_stream_data_for(
        &mut self,
        addr: &SocketAddr,
        stream_id: u64,
        data: &[u8],
        fin: bool,
    ) -> Result<()> {
        let conn_id = self
            .server
            .connections
            .values()
            .find(|state| state.addr == *addr)
            .map(|state| state.conn_id.clone())
            .ok_or(Error::InvalidState("connection not found"))?;
        self.send_stream_data_inner(&conn_id, stream_id, data, fin)
    }

    /// 接続を追加して HTTP/3 ストリームを初期化する
    async fn add_connection(
        &mut self,
        conn: shiguredo_ngtcp2_tokio::AcceptedConnection,
    ) -> Result<()> {
        let conn_id = conn.connection_id();
        self.server.add_connection(conn).await?;
        self.wt.insert(conn_id, WtConnectionState::new());
        Ok(())
    }

    /// 双方向ストリームを開く (接続 ID 指定)
    fn open_bidi_stream_inner(&mut self, conn_id: &ConnectionId) -> Result<u64> {
        let session_id = self
            .wt
            .get(conn_id)
            .and_then(|wt| wt.sessions.iter().next().copied())
            .ok_or(Error::InvalidState(
                "WebTransport session is not established",
            ))?;
        let state = self
            .server
            .connections
            .get_mut(conn_id)
            .ok_or(Error::InvalidState("connection not found"))?;
        let stream_id = state.conn.open_bidi_stream()? as u64;
        state.h3.register_local_wt_stream(session_id, stream_id)?;
        if let Some(wt) = self.wt.get_mut(conn_id) {
            wt.stream_sessions.insert(stream_id, session_id);
        }
        let mut header = Vec::new();
        StreamHeader::new(session_id)
            .map_err(|_| Error::InvalidState("invalid WebTransport session id"))?
            .encode_bidirectional(&mut header);
        state.conn.write_stream(stream_id as i64, &header, false)?;
        Ok(stream_id)
    }

    /// ストリームデータを送信する (接続 ID 指定)
    fn send_stream_data_inner(
        &mut self,
        conn_id: &ConnectionId,
        stream_id: u64,
        data: &[u8],
        fin: bool,
    ) -> Result<()> {
        let state = self
            .server
            .connections
            .get_mut(conn_id)
            .ok_or(Error::InvalidState("connection not found"))?;
        state.conn.write_stream(stream_id as i64, data, fin)?;
        Ok(())
    }

    /// 全接続を駆動してイベントをハンドラーに渡す
    async fn drive_all<F>(&mut self, handler: &mut F) -> Result<()>
    where
        F: FnMut(SocketAddr, u64, Event) -> bool,
    {
        let mut failed = Vec::new();
        for state in self.server.connections.values_mut() {
            let wt = self
                .wt
                .entry(state.conn_id.clone())
                .or_insert_with(WtConnectionState::new);
            if let Err(e) = drive_connection(state, wt, handler).await {
                eprintln!("[tokio-ngtcp2 server] WebTransport connection error: {e:?}");
                failed.push(state.conn_id.clone());
            }
        }
        for conn_id in failed {
            self.server.connections.remove(&conn_id);
            self.wt.remove(&conn_id);
        }
        Ok(())
    }
}

/// 接続 1 本を駆動してイベントをハンドラーに渡す
async fn drive_connection<F>(
    state: &mut ConnectionState,
    wt: &mut WtConnectionState,
    handler: &mut F,
) -> Result<()>
where
    F: FnMut(SocketAddr, u64, Event) -> bool,
{
    {
        let mut quic = QuicConnectionRef::Server(&mut state.conn);
        state
            .h3
            .pump_wait(&mut quic, Duration::from_millis(1))
            .await?;
    }
    while let Some(event) = state.h3.poll_event() {
        match event {
            Event::Header {
                stream_id,
                name,
                value,
            } => {
                wt.request_headers
                    .entry(stream_id)
                    .or_default()
                    .push((name, value));
            }
            Event::HeadersEnd { stream_id } => {
                let headers = wt.request_headers.remove(&stream_id).unwrap_or_default();
                let is_connect = headers
                    .iter()
                    .any(|(name, value)| name == b":method" && value == b"CONNECT");
                let has_protocol = headers.iter().any(|(name, _)| name == b":protocol");
                if is_connect && has_protocol {
                    let pairs: Vec<(&[u8], &[u8])> = headers
                        .iter()
                        .map(|(name, value)| (name.as_slice(), value.as_slice()))
                        .collect();
                    let valid = ConnectRequest::from_headers(&pairs).is_ok();
                    if valid && handler(state.addr, stream_id, Event::HeadersEnd { stream_id }) {
                        let response = ConnectResponse::new(200).to_headers().map_err(|e| {
                            Error::InvalidArgument(format!("invalid CONNECT response: {e}"))
                        })?;
                        state.h3.send_response(stream_id, &response, false)?;
                    }
                }
            }
            Event::WebTransport(wt_event) => {
                match &wt_event {
                    WebTransportEvent::SessionEstablished { session_id, .. } => {
                        wt.sessions.insert(*session_id);
                        handler(
                            state.addr,
                            *session_id,
                            Event::WebTransport(wt_event.clone()),
                        );
                    }
                    WebTransportEvent::BidiStreamOpen {
                        stream_id,
                        session_id,
                    }
                    | WebTransportEvent::UniStreamOpen {
                        stream_id,
                        session_id,
                    } => {
                        wt.stream_sessions.insert(*stream_id, *session_id);
                        handler(
                            state.addr,
                            *session_id,
                            Event::WebTransport(wt_event.clone()),
                        );
                    }
                    other => {
                        let session_id = wt
                            .resolve_session(other.stream_id())
                            .unwrap_or(wt.sessions.iter().next().copied().unwrap_or(0));
                        // アプリケーションに渡したデータのフロー制御クレジットを戻す
                        if let WebTransportEvent::BidiStreamData { data, .. }
                        | WebTransportEvent::UniStreamData { data, .. } = other
                        {
                            state.h3.wt_data_consumed(session_id, data.len() as u64);
                        }
                        handler(state.addr, session_id, Event::WebTransport(other.clone()));
                    }
                }
            }
            _ => {}
        }
    }
    // 送信待ちのフロー制御カプセルを CONNECT ストリームへ送る
    for session_id in wt.sessions.iter().copied().collect::<Vec<_>>() {
        if !state.h3.wt_session_flow_control_enabled(session_id) {
            continue;
        }
        for buf in state.h3.take_wt_capsules(session_id) {
            state.conn.write_stream(session_id as i64, &buf, false)?;
        }
    }
    let mut quic = QuicConnectionRef::Server(&mut state.conn);
    state.h3.pump(&mut quic).await?;
    Ok(())
}
