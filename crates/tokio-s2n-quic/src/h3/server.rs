//! HTTP/3 サーバー

use std::net::SocketAddr;
use std::sync::{Arc, Mutex as StdMutex};

use bytes::Bytes;
use s2n_quic::connection::BidirectionalStreamAcceptor;
use s2n_quic::stream::SendStream;
use shiguredo_http3::{Event, Header};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::config::ServerConfig;
use crate::internal::connection_state::ServerConnectionState;

/// HTTP/3 サーバー
pub struct H3Server {
    /// s2n-quic Server
    server: s2n_quic::Server,
    /// ローカルアドレス
    local_addr: SocketAddr,
    /// H3 設定
    h3_settings: shiguredo_http3::Settings,
}

impl H3Server {
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

        Ok(Self {
            server,
            local_addr,
            h3_settings: config.h3_settings,
        })
    }

    /// 新しい接続を受け付ける
    pub async fn accept(&mut self) -> crate::Result<H3ServerConnection> {
        let connection = self
            .server
            .accept()
            .await
            .ok_or(crate::Error::ConnectionClosed)?;

        H3ServerConnection::new(connection, self.h3_settings).await
    }

    /// ローカルアドレスを取得する
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }
}

/// HTTP/3 サーバー接続
pub struct H3ServerConnection {
    /// 接続状態
    state: Arc<StdMutex<ServerConnectionState>>,
    /// 双方向ストリームアクセプター
    bidi_acceptor: BidirectionalStreamAcceptor,
    /// 接続ハンドル
    _handle: s2n_quic::connection::Handle,
    /// QPACK データ送信チャンネル
    qpack_tx: mpsc::UnboundedSender<(u64, Vec<u8>)>,
    /// 制御ストリームタスク
    _control_task: JoinHandle<()>,
    /// 単方向ストリーム処理タスク
    _uni_task: JoinHandle<()>,
    /// QPACK ストリーム送信タスク
    _qpack_task: JoinHandle<()>,
}

impl H3ServerConnection {
    /// 新しいサーバー接続を作成する
    async fn new(
        mut connection: s2n_quic::Connection,
        h3_settings: shiguredo_http3::Settings,
    ) -> crate::Result<Self> {
        let state = Arc::new(StdMutex::new(ServerConnectionState::new(h3_settings)));

        // 制御ストリーム・QPACK ストリームを開く
        let mut control_send = connection.open_send_stream().await?;
        let mut encoder_send = connection.open_send_stream().await?;
        let mut decoder_send = connection.open_send_stream().await?;

        let init_data = {
            let mut s = state.lock().unwrap();
            s.init_h3_streams(control_send.id(), encoder_send.id(), decoder_send.id())?
        };

        // 初期データを各ストリームに送信
        control_send
            .send(Bytes::from(init_data.control_data))
            .await?;
        encoder_send
            .send(Bytes::from(init_data.encoder_data))
            .await?;
        decoder_send
            .send(Bytes::from(init_data.decoder_data))
            .await?;

        let encoder_stream_id = init_data.encoder_stream_id;
        let decoder_stream_id = init_data.decoder_stream_id;

        // QPACK データ送信用チャンネル
        let (qpack_tx, mut qpack_rx) = mpsc::unbounded_channel::<(u64, Vec<u8>)>();

        // QPACK ストリーム送信タスク
        // エンコーダーストリームとデコーダーストリームを保持し、
        // チャンネル経由で受け取ったデータを送信する。
        // RFC 9204 Section 4.2: ストリームを閉じてはならない。
        let qpack_task = tokio::spawn(async move {
            while let Some((stream_id, data)) = qpack_rx.recv().await {
                let send_result = if stream_id == encoder_stream_id {
                    encoder_send.send(Bytes::from(data)).await
                } else if stream_id == decoder_stream_id {
                    decoder_send.send(Bytes::from(data)).await
                } else {
                    continue;
                };
                if send_result.is_err() {
                    break;
                }
            }
            // チャンネルが閉じたらストリームを保持したまま待機
            // (接続終了まで閉じない)
            std::future::pending::<()>().await;
        });

        // connection.split() → (Handle, StreamAcceptor) → StreamAcceptor.split()
        let (handle, stream_acceptor) = connection.split();
        let (bidi_acceptor, mut uni_acceptor) = stream_acceptor.split();

        // 制御ストリーム送信タスク (接続中保持する)
        let control_task = tokio::spawn(async move {
            let _control_send = control_send;
            std::future::pending::<()>().await;
        });

        // 単方向ストリーム受信タスク (ピアの制御ストリーム等)
        let state_for_uni = Arc::clone(&state);
        let qpack_tx_for_uni = qpack_tx.clone();
        let uni_task = tokio::spawn(async move {
            while let Ok(Some(mut recv_stream)) = uni_acceptor.accept_receive_stream().await {
                let state = Arc::clone(&state_for_uni);
                let qpack_tx = qpack_tx_for_uni.clone();
                let stream_id: u64 = recv_stream.id();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    while let Ok(Some(data)) = recv_stream.receive().await {
                        buf.extend_from_slice(&data);
                        let _ = {
                            let mut s = state.lock().unwrap();
                            s.process_stream_data(stream_id, &buf, false).ok()
                        };
                        // SETTINGS 受信後に Set Capacity が生成される可能性がある
                        flush_qpack(&state, &qpack_tx);
                        buf.clear();
                    }
                    // FIN
                    let _ = state
                        .lock()
                        .unwrap()
                        .process_stream_data(stream_id, &[], true);
                    flush_qpack(&state, &qpack_tx);
                });
            }
        });

        Ok(Self {
            state,
            bidi_acceptor,
            _handle: handle,
            qpack_tx,
            _control_task: control_task,
            _uni_task: uni_task,
            _qpack_task: qpack_task,
        })
    }

    /// リクエストを受け付ける
    pub async fn accept_request(&mut self) -> crate::Result<H3Request> {
        let stream: s2n_quic::stream::BidirectionalStream = self
            .bidi_acceptor
            .accept_bidirectional_stream()
            .await
            .map_err(crate::Error::transport)?
            .ok_or(crate::Error::ConnectionClosed)?;

        let stream_id: u64 = stream.id();
        let (mut recv_stream, send_stream) = stream.split();

        // Sans I/O にストリームの存在を通知
        {
            let mut s = self.state.lock().unwrap();
            let _ = s.h3_conn.feed_stream(stream_id, &[], false);
        }

        // リクエストデータを受信して Sans I/O に feed する
        let mut headers: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        let mut body = Vec::new();
        let mut headers_complete = false;
        let mut stream_ended = false;

        loop {
            let received: Result<Option<Bytes>, _> = recv_stream.receive().await;
            let (data, fin) = match received {
                Ok(Some(data)) => (data.to_vec(), false),
                Ok(None) => (vec![], true),
                Err(e) => return Err(crate::Error::transport(e)),
            };

            let events = {
                let mut s = self.state.lock().unwrap();
                s.process_stream_data(stream_id, &data, fin)?
            };

            // ヘッダーデコード後に Section Ack が生成される可能性がある
            flush_qpack(&self.state, &self.qpack_tx);

            for event in events {
                match event {
                    Event::Header { name, value, .. } => {
                        headers.push((name, value));
                    }
                    Event::HeadersEnd { .. } => {
                        headers_complete = true;
                    }
                    Event::Data { data: d, .. } => {
                        body.extend_from_slice(&d);
                    }
                    Event::StreamEnd { .. } => {
                        stream_ended = true;
                    }
                    _ => {}
                }
            }

            if headers_complete || fin {
                break;
            }
        }

        // ヘッダー完了後にボディの残りを読む
        if headers_complete && !stream_ended {
            loop {
                let received: Result<Option<Bytes>, _> = recv_stream.receive().await;
                let (data, fin) = match received {
                    Ok(Some(data)) => (data.to_vec(), false),
                    Ok(None) => (vec![], true),
                    Err(_) => break,
                };

                let events = {
                    let mut s = self.state.lock().unwrap();
                    s.process_stream_data(stream_id, &data, fin)?
                };

                // QPACK データのドレイン
                flush_qpack(&self.state, &self.qpack_tx);

                for event in events {
                    match event {
                        Event::Data { data: d, .. } => {
                            body.extend_from_slice(&d);
                        }
                        Event::StreamEnd { .. } => {
                            stream_ended = true;
                        }
                        _ => {}
                    }
                }

                if stream_ended || fin {
                    break;
                }
            }
        }

        Ok(H3Request {
            headers,
            stream_id,
            body,
            state: Arc::clone(&self.state),
            qpack_tx: self.qpack_tx.clone(),
            send_stream: StdMutex::new(Some(send_stream)),
        })
    }
}

/// QPACK ストリームの送信待ちデータをドレインしてチャンネルに送信する
fn flush_qpack(
    state: &Arc<StdMutex<ServerConnectionState>>,
    tx: &mpsc::UnboundedSender<(u64, Vec<u8>)>,
) {
    let data = state.lock().unwrap().drain_qpack_data();
    for item in data {
        let _ = tx.send(item);
    }
}

/// HTTP/3 リクエスト
pub struct H3Request {
    /// リクエストヘッダー
    headers: Vec<(Vec<u8>, Vec<u8>)>,
    /// ストリーム ID
    stream_id: u64,
    /// リクエストボディ
    body: Vec<u8>,
    /// 接続状態
    state: Arc<StdMutex<ServerConnectionState>>,
    /// QPACK データ送信チャンネル
    qpack_tx: mpsc::UnboundedSender<(u64, Vec<u8>)>,
    /// 送信ストリーム (&self で使うため StdMutex<Option<>> で保持)
    send_stream: StdMutex<Option<SendStream>>,
}

impl H3Request {
    /// ヘッダーを取得する
    pub fn headers(&self) -> &[(Vec<u8>, Vec<u8>)] {
        &self.headers
    }

    /// メソッドを取得する
    pub fn method(&self) -> &[u8] {
        for (name, value) in &self.headers {
            if name == b":method" {
                return value;
            }
        }
        b""
    }

    /// パスを取得する
    pub fn path(&self) -> &[u8] {
        for (name, value) in &self.headers {
            if name == b":path" {
                return value;
            }
        }
        b""
    }

    /// ボディを取得する
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    /// ストリーム ID を取得する
    pub fn stream_id(&self) -> u64 {
        self.stream_id
    }

    /// レスポンスを送信する
    pub async fn send_response(&self, response: H3Response) -> crate::Result<()> {
        // Sans I/O でレスポンスをエンコード
        let data = {
            let mut s = self.state.lock().unwrap();

            let mut headers = vec![Header::new(
                b":status",
                response.status.to_string().as_bytes(),
            )];
            for (name, value) in &response.headers {
                headers.push(Header::new(name.as_slice(), value.as_slice()));
            }

            s.prepare_response(self.stream_id, &headers, &response.body)?;

            // エンコード済みデータを取得
            s.get_stream_data(self.stream_id)
                .map(|(data, _fin)| data)
                .unwrap_or_default()
        };

        // エンコード時に QPACK データが生成される可能性がある
        flush_qpack(&self.state, &self.qpack_tx);

        // SendStream を take して送信
        let mut send_stream = self
            .send_stream
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| crate::Error::InvalidState("response already sent".to_string()))?;

        send_stream
            .send(Bytes::from(data))
            .await
            .map_err(crate::Error::transport)?;

        send_stream.finish().map_err(crate::Error::transport)?;

        Ok(())
    }
}

/// HTTP/3 レスポンス (ビルダー)
pub struct H3Response {
    /// ステータスコード
    status: u16,
    /// レスポンスヘッダー
    headers: Vec<(Vec<u8>, Vec<u8>)>,
    /// レスポンスボディ
    body: Vec<u8>,
}

impl H3Response {
    /// 新しいレスポンスを作成する
    pub fn new(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    /// ヘッダーを追加する
    pub fn header(mut self, name: impl Into<Vec<u8>>, value: impl Into<Vec<u8>>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// ボディを設定する
    pub fn body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }
}
