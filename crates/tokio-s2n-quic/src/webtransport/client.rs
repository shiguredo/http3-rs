//! WebTransport クライアント

use std::sync::{Arc, Mutex as StdMutex};

use bytes::Bytes;
use s2n_quic::client::Connect;
use shiguredo_http3::Event;
use shiguredo_http3::webtransport::ConnectRequest;
use tokio::sync::Notify;
use tokio::sync::mpsc;

use super::session::{WtRecvStream, WtSession};
use crate::config::ClientConfig;
use crate::internal::connection_state::ClientConnectionState;

/// WebTransport クライアント
pub struct WtClient;

impl WtClient {
    /// WebTransport セッションを確立する
    pub async fn connect(config: ClientConfig, path: &str) -> crate::Result<WtSession> {
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

        // CONNECT リクエスト (双方向ストリーム) を開く
        let connect_stream = connection.open_bidirectional_stream().await?;
        let connect_stream_id: u64 = connect_stream.id();
        let (mut recv_stream, mut send_stream) = connect_stream.split();

        // connection.split() → (Handle, StreamAcceptor)
        let (handle, stream_acceptor) = connection.split();
        let (bidi_acceptor, mut uni_acceptor) = stream_acceptor.split();

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
        tokio::spawn(async move {
            while let Ok(Some(recv)) = uni_acceptor.accept_receive_stream().await {
                let state = Arc::clone(&state_for_uni);
                let notify = Arc::clone(&notify_for_uni);
                let uni_tx = uni_tx.clone();
                let stream_id: u64 = recv.id();
                tokio::spawn(route_uni_stream(stream_id, recv, state, notify, uni_tx));
            }
        });

        // 制御ストリーム・QPACK ストリームを接続中保持する
        tokio::spawn(async move {
            let _control_send = control_send;
            let _encoder_send = encoder_send;
            let _decoder_send = decoder_send;
            std::future::pending::<()>().await;
        });

        // peer SETTINGS の受信を待つ (WebTransport CONNECT の送信に必要)
        // uni_task がサーバーの制御ストリームを処理して SETTINGS を注入するまで待機する
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

        // Sans I/O で CONNECT リクエストをエンコード
        let request_data = {
            let mut s = state.lock().unwrap();

            let connect_req = ConnectRequest::new("https", &config.server_name, path)
                .draft_version(config.draft_version);
            let headers = connect_req.to_headers();

            let h3_stream_id = s.send_request(&headers, false)?;
            if h3_stream_id != connect_stream_id {
                return Err(crate::Error::Internal(format!(
                    "unexpected CONNECT stream id: expected {connect_stream_id}, got {h3_stream_id}"
                )));
            }

            s.get_stream_data(h3_stream_id)
                .map(|(data, _fin)| data)
                .unwrap_or_default()
        };

        // CONNECT リクエスト送信 (fin=false: セッション中ストリームを開いたままにする)
        send_stream
            .send(Bytes::from(request_data))
            .await
            .map_err(crate::Error::transport)?;

        // CONNECT レスポンスを待つ
        //
        // QPACK エンコーダーストリームが先に到着してブロック解除される場合に備え、
        // recv_stream.receive() と unblock_notify.notified() を select! で待つ。
        let mut session_established = false;
        while !session_established {
            // QPACK エンコーダーストリームのデータが先に処理されて
            // notify_one() が取りこぼされる場合に備え、毎回 drain_events を確認する
            {
                let events = {
                    let mut s = state.lock().unwrap();
                    s.drain_events()?
                };
                for event in events {
                    if let Event::HeadersEnd { .. } = event {
                        session_established = true;
                    }
                }
                if session_established {
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
                        if let Event::HeadersEnd { .. } = event {
                            session_established = true;
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
                // notify_one() の取りこぼし対策
                _ = tokio::time::sleep(std::time::Duration::from_millis(10)) => {}
            }
        }

        if !session_established {
            return Err(crate::Error::ConnectionClosed);
        }

        Ok(WtSession::new(
            connect_stream_id,
            bidi_acceptor,
            handle,
            send_stream,
            uni_rx,
        ))
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
    state: Arc<StdMutex<ClientConnectionState>>,
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
