//! HTTP/3 クライアント

use std::sync::{Arc, Mutex as StdMutex};

use bytes::Bytes;
use s2n_quic::client::Connect;
use shiguredo_http3::{Event, Header};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::config::ClientConfig;
use crate::internal::connection_state::ClientConnectionState;

/// HTTP/3 クライアント
pub struct H3Client {
    /// 接続状態
    state: Arc<StdMutex<ClientConnectionState>>,
    /// 接続ハンドル
    handle: s2n_quic::connection::Handle,
    /// QPACK データ送信チャンネル
    qpack_tx: mpsc::UnboundedSender<(u64, Vec<u8>)>,
    /// 制御ストリームタスク
    _control_task: JoinHandle<()>,
    /// 単方向ストリーム処理タスク
    _uni_task: JoinHandle<()>,
    /// QPACK ストリーム送信タスク
    _qpack_task: JoinHandle<()>,
}

impl H3Client {
    /// サーバーに接続する
    pub async fn connect(config: ClientConfig) -> crate::Result<Self> {
        let client = if config.disable_cert_validation {
            let tls = s2n_quic::provider::tls::default::Client::builder()
                .build()
                .map_err(crate::Error::transport)?;
            s2n_quic::Client::builder()
                .with_tls(tls)
                .map_err(crate::Error::transport)?
                .with_io("0.0.0.0:0")
                .map_err(crate::Error::transport)?
                .start()
                .map_err(crate::Error::transport)?
        } else if let Some(ref ca_pem) = config.ca_cert_pem {
            s2n_quic::Client::builder()
                .with_tls(ca_pem.as_str())
                .map_err(crate::Error::transport)?
                .with_io("0.0.0.0:0")
                .map_err(crate::Error::transport)?
                .start()
                .map_err(crate::Error::transport)?
        } else {
            return Err(crate::Error::InvalidState(
                "ca_cert_pem or disable_cert_validation is required".to_string(),
            ));
        };

        let connect = Connect::new(config.remote_addr).with_server_name(&*config.server_name);
        let mut connection = client.connect(connect).await?;

        let state = Arc::new(StdMutex::new(ClientConnectionState::new(
            config.h3_settings,
        )));

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

        let (handle, acceptor) = connection.split();
        let (_bidi_acceptor, mut uni_acceptor) = acceptor.split();

        // 制御ストリーム送信タスク (接続中保持する)
        let control_task = tokio::spawn(async move {
            let _control_send = control_send;
            std::future::pending::<()>().await;
        });

        // 単方向ストリーム受信タスク
        let state_for_uni = Arc::clone(&state);
        let qpack_tx_for_uni = qpack_tx.clone();
        let uni_task = tokio::spawn(async move {
            while let Ok(Some(mut recv_stream)) = uni_acceptor.accept_receive_stream().await {
                let state = Arc::clone(&state_for_uni);
                let qpack_tx = qpack_tx_for_uni.clone();
                let stream_id: u64 = recv_stream.id();
                tokio::spawn(async move {
                    while let Ok(Some(data)) = recv_stream.receive().await {
                        let _ = state
                            .lock()
                            .unwrap()
                            .process_stream_data(stream_id, &data, false);
                        // QPACK データをドレインして送信タスクに転送
                        flush_qpack(&state, &qpack_tx);
                    }
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
            handle,
            qpack_tx,
            _control_task: control_task,
            _uni_task: uni_task,
            _qpack_task: qpack_task,
        })
    }

    /// リクエストを送信する
    pub async fn send_request(
        &mut self,
        request: H3ClientRequest,
    ) -> crate::Result<H3ClientResponse> {
        // 双方向ストリームを開く
        let stream = self.handle.open_bidirectional_stream().await?;
        let stream_id: u64 = stream.id();
        let (mut recv_stream, mut send_stream) = stream.split();

        // Sans I/O でリクエストをエンコード
        let request_data = {
            let mut s = self.state.lock().unwrap();

            let mut headers = Vec::new();
            headers.push(Header::new(b":method", request.method.as_slice()));
            headers.push(Header::new(b":path", request.path.as_slice()));
            headers.push(Header::new(b":scheme", b"https"));
            if let Some(ref authority) = request.authority {
                headers.push(Header::new(b":authority", authority.as_slice()));
            }
            for (name, value) in &request.headers {
                headers.push(Header::new(name.as_slice(), value.as_slice()));
            }

            let fin = request.body.is_empty();
            let h3_stream_id = s.send_request(&headers, fin)?;

            if !fin {
                s.h3_conn.send_body(h3_stream_id, &request.body, true)?;
            }

            // h3_stream_id は Sans I/O が割り当てたもの
            // s2n-quic の stream_id と一致するはず
            assert_eq!(h3_stream_id, stream_id);

            // エンコード済みデータを取得
            s.get_stream_data(h3_stream_id)
                .map(|(data, _fin)| data)
                .unwrap_or_default()
        };

        // エンコード時に QPACK データが生成される可能性がある
        flush_qpack(&self.state, &self.qpack_tx);

        // リクエスト送信
        send_stream.send(Bytes::from(request_data)).await?;
        send_stream.finish()?;

        // レスポンス受信
        let mut response_headers: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        let mut response_body = Vec::new();
        let mut finished = false;

        while !finished {
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
                        response_headers.push((name, value));
                    }
                    Event::Data { data: d, .. } => {
                        response_body.extend_from_slice(&d);
                    }
                    Event::StreamEnd { .. } => {
                        finished = true;
                    }
                    _ => {}
                }
            }

            if fin {
                break;
            }
        }

        let status = response_headers
            .iter()
            .find(|(name, _)| name == b":status")
            .and_then(|(_, value)| std::str::from_utf8(value).ok())
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(0);

        Ok(H3ClientResponse {
            status,
            headers: response_headers,
            body: response_body,
        })
    }
}

/// QPACK ストリームの送信待ちデータをドレインしてチャンネルに送信する
fn flush_qpack(
    state: &Arc<StdMutex<ClientConnectionState>>,
    tx: &mpsc::UnboundedSender<(u64, Vec<u8>)>,
) {
    let data = state.lock().unwrap().drain_qpack_data();
    for item in data {
        let _ = tx.send(item);
    }
}

/// HTTP/3 クライアントリクエスト (ビルダー)
pub struct H3ClientRequest {
    /// メソッド
    method: Vec<u8>,
    /// パス
    path: Vec<u8>,
    /// authority
    authority: Option<Vec<u8>>,
    /// ヘッダー
    headers: Vec<(Vec<u8>, Vec<u8>)>,
    /// ボディ
    body: Vec<u8>,
}

impl H3ClientRequest {
    /// GET リクエストを作成する
    pub fn get(path: impl Into<Vec<u8>>) -> Self {
        Self {
            method: b"GET".to_vec(),
            path: path.into(),
            authority: None,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    /// POST リクエストを作成する
    pub fn post(path: impl Into<Vec<u8>>) -> Self {
        Self {
            method: b"POST".to_vec(),
            path: path.into(),
            authority: None,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    /// authority を設定する
    pub fn authority(mut self, authority: impl Into<Vec<u8>>) -> Self {
        self.authority = Some(authority.into());
        self
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

/// HTTP/3 クライアントレスポンス
pub struct H3ClientResponse {
    /// ステータスコード
    status: u16,
    /// レスポンスヘッダー
    headers: Vec<(Vec<u8>, Vec<u8>)>,
    /// レスポンスボディ
    body: Vec<u8>,
}

impl H3ClientResponse {
    /// ステータスコードを取得する
    pub fn status(&self) -> u16 {
        self.status
    }

    /// ヘッダーを取得する
    pub fn headers(&self) -> &[(Vec<u8>, Vec<u8>)] {
        &self.headers
    }

    /// ボディを取得する
    pub fn body(&self) -> &[u8] {
        &self.body
    }
}
