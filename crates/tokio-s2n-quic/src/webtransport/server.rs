//! WebTransport サーバー

use std::net::SocketAddr;
use std::sync::{Arc, Mutex as StdMutex};

use bytes::Bytes;
use s2n_quic::connection::BidirectionalStreamAcceptor;
use shiguredo_http3::Event;
use shiguredo_http3::webtransport::ConnectResponse;
use tokio::sync::Notify;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::session::{WtRecvStream, WtSession};
use crate::config::ServerConfig;
use crate::internal::connection_state::ServerConnectionState;

/// WebTransport サーバー
pub struct WtServer {
    /// ローカルアドレス
    local_addr: SocketAddr,
    /// セッションリクエスト受信チャネル
    request_rx: mpsc::Receiver<crate::Result<WtSessionRequest>>,
}

impl WtServer {
    /// サーバーをバインドする
    pub fn bind(config: ServerConfig) -> crate::Result<Self> {
        let server = s2n_quic::Server::builder()
            .with_tls((config.cert_pem.as_str(), config.key_pem.as_str()))
            .map_err(crate::Error::transport)?
            .with_io(config.listen_addr)
            .map_err(crate::Error::transport)?
            .start()
            .map_err(crate::Error::transport)?;

        let local_addr = server
            .local_addr()
            .map_err(|e| crate::Error::Internal(format!("failed to get local addr: {e}")))?;

        let h3_settings = config.h3_settings;
        let (tx, request_rx) = mpsc::channel::<crate::Result<WtSessionRequest>>(16);

        // バックグラウンドで接続を受け付け、並列に CONNECT ハンドシェイクを処理する
        tokio::spawn(async move {
            let mut server = server;
            while let Some(connection) = server.accept().await {
                let tx = tx.clone();
                tokio::spawn(async move {
                    let result = WtSessionRequest::from_connection(connection, h3_settings).await;
                    let _ = tx.send(result).await;
                });
            }
        });

        Ok(Self {
            local_addr,
            request_rx,
        })
    }

    /// WebTransport セッションリクエストを受け付ける
    pub async fn accept(&mut self) -> crate::Result<WtSessionRequest> {
        self.request_rx
            .recv()
            .await
            .ok_or(crate::Error::ConnectionClosed)?
    }

    /// ローカルアドレスを取得する
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }
}

/// WebTransport セッションリクエスト
pub struct WtSessionRequest {
    /// パス
    path: Vec<u8>,
    /// authority
    authority: Vec<u8>,
    /// セッションストリーム ID
    stream_id: u64,
    /// 接続状態
    state: Arc<StdMutex<ServerConnectionState>>,
    /// 送信ストリーム (CONNECT レスポンス送信用、セッション確立後は CLOSE_WEBTRANSPORT_SESSION 送信用)
    send_stream: s2n_quic::stream::SendStream,
    /// 双方向ストリームアクセプター
    bidi_acceptor: BidirectionalStreamAcceptor,
    /// 接続ハンドル
    handle: s2n_quic::connection::Handle,
    /// WT 単方向ストリーム受信チャネル
    uni_rx: mpsc::Receiver<WtRecvStream>,
    /// 制御ストリームタスク
    _control_task: JoinHandle<()>,
    /// 単方向ストリーム処理タスク
    _uni_task: JoinHandle<()>,
}

impl WtSessionRequest {
    /// 既に accept 済みの QUIC 接続から WebTransport セッションリクエストを作成する。
    ///
    /// 同一ポートで複数の ALPN (例: h3, h3-webtransport) をルーティングする場合、
    /// Listener の外部で accept した `s2n_quic::Connection` を直接渡して
    /// WebTransport セッションを開始できる。
    pub async fn from_connection(
        mut connection: s2n_quic::Connection,
        h3_settings: shiguredo_http3::Settings,
    ) -> crate::Result<Self> {
        let state = Arc::new(StdMutex::new(ServerConnectionState::new(h3_settings)));

        // 制御ストリーム・QPACK ストリームを開いて初期化データを送信
        let mut control_send = connection.open_send_stream().await?;
        let mut encoder_send = connection.open_send_stream().await?;
        let mut decoder_send = connection.open_send_stream().await?;
        let control_stream_id: u64 = control_send.id();
        let encoder_stream_id: u64 = encoder_send.id();
        let decoder_stream_id: u64 = decoder_send.id();

        let init_data = {
            let mut s = state.lock().unwrap();
            let data =
                s.init_h3_streams(control_stream_id, encoder_stream_id, decoder_stream_id)?;
            // s2n-quic は DATAGRAM (RFC 9221) と RESET_STREAM_AT をサポートする
            s.h3_conn.set_webtransport_transport_verified(true, true)?;
            data
        };

        // 各ストリームの初期化データを送信
        if !init_data.control_data.is_empty() {
            control_send
                .send(Bytes::from(init_data.control_data))
                .await?;
        }
        if !init_data.encoder_data.is_empty() {
            encoder_send
                .send(Bytes::from(init_data.encoder_data))
                .await?;
        }
        if !init_data.decoder_data.is_empty() {
            decoder_send
                .send(Bytes::from(init_data.decoder_data))
                .await?;
        }

        let (handle, stream_acceptor) = connection.split();
        let (mut bidi_acceptor, mut uni_acceptor) = stream_acceptor.split();

        // 制御ストリーム・QPACK ストリーム送信タスク (接続中保持する)
        let control_task = tokio::spawn(async move {
            let _control_send = control_send;
            let _encoder_send = encoder_send;
            let _decoder_send = decoder_send;
            std::future::pending::<()>().await;
        });

        // QPACK エンコーダーストリームの更新でブロック解除を通知する
        let unblock_notify = Arc::new(Notify::new());

        // WT 単方向ストリームを WtSession に渡すためのチャネル
        let (uni_tx, uni_rx) = mpsc::channel::<WtRecvStream>(16);

        // 単方向ストリーム受信タスク
        //
        // classify_uni_stream でストリームタイプを判別する:
        // - WebTransport: WT 単方向ストリーム → uni_tx に送信
        // - Http3: H3 制御ストリームなど → feed_stream_only でフィード
        let state_for_uni = Arc::clone(&state);
        let notify_for_uni = Arc::clone(&unblock_notify);
        let uni_task = tokio::spawn(async move {
            while let Ok(Some(recv_stream)) = uni_acceptor.accept_receive_stream().await {
                let state = Arc::clone(&state_for_uni);
                let notify = Arc::clone(&notify_for_uni);
                let uni_tx = uni_tx.clone();
                let stream_id: u64 = recv_stream.id();
                tokio::spawn(route_uni_stream(
                    stream_id,
                    recv_stream,
                    state,
                    notify,
                    uni_tx,
                ));
            }
        });

        // 最初の双方向ストリーム (CONNECT リクエスト) を受け付ける
        let stream: s2n_quic::stream::BidirectionalStream = bidi_acceptor
            .accept_bidirectional_stream()
            .await
            .map_err(crate::Error::transport)?
            .ok_or(crate::Error::ConnectionClosed)?;

        let connect_stream_id: u64 = stream.id();
        let (mut recv_stream, send_stream) = stream.split();

        // Sans I/O にストリームの存在を通知
        {
            let mut s = state.lock().unwrap();
            let _ = s.h3_conn.feed_stream(connect_stream_id, &[], false);
        }

        // peer SETTINGS の受信を待つ (WebTransport CONNECT の検証に必要)
        // uni_task がクライアントの制御ストリームを処理して SETTINGS を注入するまで待機する
        // (draft-ietf-webtrans-http3-15 Section 3.1)
        loop {
            {
                let s = state.lock().unwrap();
                if s.h3_conn.peer_settings().is_some() {
                    break;
                }
            }
            tokio::select! {
                _ = unblock_notify.notified() => {}
                _ = tokio::time::sleep(std::time::Duration::from_millis(10)) => {}
            }
        }

        // CONNECT リクエストを受信
        //
        // QPACK エンコーダーストリームが先に到着してブロック解除される場合に備え、
        // recv_stream.receive() と unblock_notify.notified() を select! で待つ。
        let mut path = Vec::new();
        let mut authority = Vec::new();
        let mut headers_complete = false;

        while !headers_complete {
            // QPACK エンコーダーストリームのデータが先に処理されて
            // notify_one() が取りこぼされる場合に備え、毎回 drain_events を確認する
            {
                let events = {
                    let mut s = state.lock().unwrap();
                    s.drain_events()?
                };
                for event in events {
                    match event {
                        Event::Header { name, value, .. } => {
                            if name == b":path" {
                                path = value;
                            } else if name == b":authority" {
                                authority = value;
                            }
                        }
                        Event::HeadersEnd { .. } => {
                            headers_complete = true;
                        }
                        _ => {}
                    }
                }
                if headers_complete {
                    break;
                }
            }

            tokio::select! {
                received = recv_stream.receive() => {
                    let (data, fin) = match received {
                        Ok(Some(data)) => (data.to_vec(), false),
                        Ok(None) => (vec![], true),
                        Err(e) => return Err(crate::Error::transport(e)),
                    };

                    let events = {
                        let mut s = state.lock().unwrap();
                        s.process_stream_data(connect_stream_id, &data, fin)?
                    };

                    for event in events {
                        match event {
                            Event::Header { name, value, .. } => {
                                if name == b":path" {
                                    path = value;
                                } else if name == b":authority" {
                                    authority = value;
                                }
                            }
                            Event::HeadersEnd { .. } => {
                                headers_complete = true;
                            }
                            _ => {}
                        }
                    }

                    if fin {
                        break;
                    }
                }
                _ = unblock_notify.notified() => {
                    // QPACK エンコーダーストリームが更新された可能性がある
                    // ループ先頭の drain_events で処理する
                }
                // notify_one() の取りこぼし対策: 短いタイムアウトで drain_events を再確認する
                _ = tokio::time::sleep(std::time::Duration::from_millis(10)) => {}
            }
        }

        Ok(Self {
            path,
            authority,
            stream_id: connect_stream_id,
            state,
            send_stream,
            bidi_acceptor,
            handle,
            uni_rx,
            _control_task: control_task,
            _uni_task: uni_task,
        })
    }

    /// パスを取得する
    pub fn path(&self) -> &[u8] {
        &self.path
    }

    /// authority を取得する
    pub fn authority(&self) -> &[u8] {
        &self.authority
    }

    /// セッションリクエストを受け入れる
    pub async fn accept(self) -> crate::Result<WtSession> {
        let response_headers = ConnectResponse::new(200).to_headers()?;

        // Sans I/O でレスポンスをエンコード
        let data = {
            let mut s = self.state.lock().unwrap();
            s.h3_conn
                .send_response(self.stream_id, &response_headers, false)?;

            s.get_stream_data(self.stream_id)
                .map(|(data, _fin)| data)
                .unwrap_or_default()
        };

        // レスポンスを送信
        let mut send_stream = self.send_stream;
        send_stream
            .send(Bytes::from(data))
            .await
            .map_err(crate::Error::transport)?;

        Ok(WtSession::new(
            self.stream_id,
            self.bidi_acceptor,
            self.handle,
            send_stream,
            self.uni_rx,
        ))
    }

    /// セッションリクエストを拒否する
    ///
    /// `status` は 4xx または 5xx でなければならない
    pub async fn reject(self, status: u16) -> crate::Result<()> {
        if !(400..=599).contains(&status) {
            return Err(crate::Error::InvalidState(format!(
                "reject status must be 4xx or 5xx, got {status}"
            )));
        }

        let response_headers = ConnectResponse::new(status).to_headers()?;

        let data = {
            let mut s = self.state.lock().unwrap();
            s.h3_conn
                .send_response(self.stream_id, &response_headers, true)?;

            s.get_stream_data(self.stream_id)
                .map(|(data, _fin)| data)
                .unwrap_or_default()
        };

        let mut send_stream = self.send_stream;
        send_stream
            .send(Bytes::from(data))
            .await
            .map_err(crate::Error::transport)?;

        send_stream.finish().map_err(crate::Error::transport)
    }
}

/// 単方向ストリームをタイプで判別してルーティングする
///
/// classify_uni_stream で先頭バイトからストリームタイプを分類する:
/// - WebTransport: WT 単方向ストリーム → uni_tx に送信
/// - Http3: H3 制御ストリームなど → feed_stream_only でフィード
async fn route_uni_stream(
    stream_id: u64,
    mut recv_stream: s2n_quic::stream::ReceiveStream,
    state: Arc<StdMutex<ServerConnectionState>>,
    notify: Arc<Notify>,
    uni_tx: mpsc::Sender<WtRecvStream>,
) {
    use shiguredo_http3::webtransport::{
        ClassifiedUniStream, StreamHeaderDecodeError, classify_uni_stream_checked,
    };

    let mut type_buf: Vec<u8> = Vec::new();

    // ストリームタイプが確定するまでデータを読む
    let classified = loop {
        match recv_stream.receive().await {
            Ok(Some(data)) => {
                type_buf.extend_from_slice(&data);
                match classify_uni_stream_checked(&type_buf) {
                    Ok(result) => break result,
                    Err(StreamHeaderDecodeError::BufferTooShort) => continue,
                    Err(_) => return, // 不正な session_id 等
                }
            }
            Ok(None) => {
                // ストリームが先に閉じた
                let _ = state.lock().unwrap().feed_stream_only(stream_id, &[], true);
                notify.notify_one();
                return;
            }
            Err(_) => return,
        }
    };

    match classified {
        ClassifiedUniStream::WebTransport { data_offset, .. } => {
            // WT 単方向ストリームとして WtSession に渡す
            let pending = type_buf[data_offset..].to_vec();
            let wt_recv = WtRecvStream::new(stream_id, recv_stream, pending);
            let _ = uni_tx.send(wt_recv).await;
        }
        ClassifiedUniStream::Http3 { .. } => {
            // H3 制御ストリームとしてフィード
            let _ = state
                .lock()
                .unwrap()
                .feed_stream_only(stream_id, &type_buf, false);
            notify.notify_one();

            while let Ok(Some(data)) = recv_stream.receive().await {
                let _ = state
                    .lock()
                    .unwrap()
                    .feed_stream_only(stream_id, &data, false);
                notify.notify_one();
            }
            let _ = state.lock().unwrap().feed_stream_only(stream_id, &[], true);
            notify.notify_one();
        }
    }
}
