//! WebTransport クライアント

use std::sync::{Arc, Mutex as StdMutex};

use bytes::Bytes;
use s2n_quic::client::Connect;
use shiguredo_http3::webtransport::ConnectRequest;
use shiguredo_http3::webtransport::error::ErrorCode as WtErrorCode;
use shiguredo_http3::{Event, WebTransportEvent};
use tokio::sync::Notify;
use tokio::sync::mpsc;

use super::session::{
    WtRecvStream, WtSession, is_forwardable_wt_event, synthesized_session_closed,
};
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
            let mut s = state.lock().expect("mutex should not be poisoned");
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
                let s = state.lock().expect("mutex should not be poisoned");
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
            let mut s = state.lock().expect("mutex should not be poisoned");

            let connect_req = ConnectRequest::new("https", &config.server_name, path)
                .draft_version(config.draft_version);
            let headers = connect_req.to_headers()?;

            let h3_stream_id = s.send_request(&headers, false)?;
            if h3_stream_id != connect_stream_id {
                return Err(crate::Error::Internal(format!(
                    "unexpected CONNECT stream id: expected {connect_stream_id}, got {h3_stream_id}"
                )));
            }

            // CONNECT リクエストは fin=false のため、データのみを取得する
            // (fin 交付は発生しない)
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
        //
        // ハンドシェイク中に到着した WebTransport イベント (200 レスポンスと同一 receive で
        // 到着した終端カプセル等) は破棄せず `pending_wt_events` に貯めて受信タスクへ
        // 引き継ぐ (取りこぼしを防ぐため)。
        let mut session_established = false;
        let mut pending_wt_events: Vec<WebTransportEvent> = Vec::new();
        while !session_established {
            // QPACK エンコーダーストリームのデータが先に処理されて
            // notify_one() が取りこぼされる場合に備え、毎回 drain_events を確認する
            {
                let events = {
                    let mut s = state.lock().expect("mutex should not be poisoned");
                    s.drain_events()?
                };
                for event in events {
                    match event {
                        Event::HeadersEnd { .. } => session_established = true,
                        Event::WebTransport(wt) if is_forwardable_wt_event(&wt) => {
                            pending_wt_events.push(wt);
                        }
                        _ => {}
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
                        let mut s = state.lock().expect("mutex should not be poisoned");
                        match s.process_stream_data(connect_stream_id, &data, fin) {
                            Ok(events) => events,
                            Err(e) => {
                                // ストリームレベルのエラーは RESET_STREAM でピアに伝える
                                // (接続は維持する: RFC 9114 Section 8)
                                crate::internal::reset_stream_on_stream_error(
                                    &mut send_stream,
                                    &e,
                                );
                                return Err(e);
                            }
                        }
                    };

                    for event in events {
                        match event {
                            Event::HeadersEnd { .. } => session_established = true,
                            Event::WebTransport(wt) if is_forwardable_wt_event(&wt) => {
                                pending_wt_events.push(wt);
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
                // notify_one() の取りこぼし対策
                _ = tokio::time::sleep(std::time::Duration::from_millis(10)) => {}
            }
        }

        if !session_established {
            return Err(crate::Error::ConnectionClosed);
        }

        // CONNECT ストリームの受信タスクを起動する。
        //
        // 受信データを sans-I/O 層へ流し、WT_CLOSE_SESSION カプセルの検知 /
        // CONNECT ストリームの FIN / RESET_STREAM を `WebTransportEvent::SessionClosed`
        // として `event_rx` に届ける (draft-ietf-webtrans-http3-16 Section 6)。
        //
        // event_rx の容量 64: 現状 CONNECT ストリーム経由で発火し得るのは接続あたり
        // ごく少数の `SessionClosed` / `SessionDraining` / `BufferedStreamRejected` のみ。
        // 将来 `StreamReset` / `Datagram` を wire したときのバースト吸収も見込んで 64 を確保する。
        //
        // クライアントは `send_response` 相当のステップがないため、`pending_wt_events` に
        // 対する追加ドレインは不要 (サーバー側の `accept()` は追加ドレインを行う)。
        let (event_tx, event_rx) = mpsc::channel::<WebTransportEvent>(64);
        let recv_task = tokio::spawn(run_client_connect_recv_task(
            connect_stream_id,
            recv_stream,
            Arc::clone(&state),
            event_tx,
            pending_wt_events,
        ));

        Ok(WtSession::new(
            connect_stream_id,
            bidi_acceptor,
            handle,
            send_stream,
            uni_rx,
            event_rx,
            recv_task,
        ))
    }
}

async fn run_client_connect_recv_task(
    session_id: u64,
    mut recv_stream: s2n_quic::stream::ReceiveStream,
    state: Arc<StdMutex<ClientConnectionState>>,
    event_tx: mpsc::Sender<WebTransportEvent>,
    pending_wt_events: Vec<WebTransportEvent>,
) {
    // ハンドシェイク中に到着していた WebTransport イベントを先に流す。
    // SessionClosed が既にキューに乗っていた場合はここでタスクを終了する。
    for event in pending_wt_events {
        let is_terminal = matches!(event, WebTransportEvent::SessionClosed { .. });
        if event_tx.send(event).await.is_err() {
            return;
        }
        if is_terminal {
            return;
        }
    }

    loop {
        let received = recv_stream.receive().await;
        // MutexGuard は await を跨げないため、ブロックで囲って先にドロップする。
        let (result, terminated) = match received {
            Ok(Some(data)) => {
                let r = state
                    .lock()
                    .expect("mutex should not be poisoned")
                    .process_stream_data(session_id, &data, false);
                (r, false)
            }
            Ok(None) => {
                // クリーンな FIN 受信: sans-I/O 層に通知して SessionClosed を生成させる
                // (draft-16 Section 6: FIN のみの終了は error_code=0 / empty message として扱う)
                let r = state
                    .lock()
                    .expect("mutex should not be poisoned")
                    .process_stream_data(session_id, &[], true);
                (r, true)
            }
            Err(_) => {
                // アブラプトクローズ (RESET_STREAM 等): connect_stream_reset を呼んで
                // sans-I/O 層に SessionClosed を生成させる (RFC 9000 Section 3.5)。
                let r = state
                    .lock()
                    .expect("mutex should not be poisoned")
                    .connect_stream_reset(session_id, WtErrorCode::SessionGone as u64);
                (r, true)
            }
        };
        let events = match result {
            Ok(events) => events,
            Err(_) => {
                // sans-I/O 層が Err を返しても、既に event queue に push 済みの
                // `SessionClosed` (WT_CLOSE_SESSION 受信済みのケース等) が沈殿している
                // 可能性があるため、まず drain_events を試みる。
                //
                // 取り出せた `SessionClosed` を優先的に届け、それも無い場合のみ
                // 合成イベントで終端を通知する。真の H3_MESSAGE_ERROR RESET_STREAM
                // 送出は別対応とする (draft-ietf-webtrans-http3-16 Section 6 の MUST)。
                let residual = state
                    .lock()
                    .expect("mutex should not be poisoned")
                    .drain_events()
                    .unwrap_or_default();
                let mut delivered = false;
                for event in residual {
                    if let Event::WebTransport(wt) = event
                        && is_forwardable_wt_event(&wt)
                    {
                        let is_terminal = matches!(wt, WebTransportEvent::SessionClosed { .. });
                        if event_tx.send(wt).await.is_err() {
                            return;
                        }
                        if is_terminal {
                            delivered = true;
                            break;
                        }
                    }
                }
                if !delivered {
                    let _ = event_tx.send(synthesized_session_closed(session_id)).await;
                }
                return;
            }
        };

        let mut session_closed = false;
        for event in events {
            if let Event::WebTransport(wt) = event
                && is_forwardable_wt_event(&wt)
            {
                let is_terminal = matches!(wt, WebTransportEvent::SessionClosed { .. });
                if event_tx.send(wt).await.is_err() {
                    return;
                }
                if is_terminal {
                    session_closed = true;
                }
            }
        }

        if terminated || session_closed {
            return;
        }
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
                let _ = state
                    .lock()
                    .expect("mutex should not be poisoned")
                    .feed_stream_only(stream_id, &[], true);
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
                .expect("mutex should not be poisoned")
                .feed_stream_only(stream_id, &type_buf, false);
            notify.notify_one();

            while let Ok(Some(data)) = recv_stream.receive().await {
                let _ = state
                    .lock()
                    .expect("mutex should not be poisoned")
                    .feed_stream_only(stream_id, &data, false);
                notify.notify_one();
            }
            let _ = state
                .lock()
                .expect("mutex should not be poisoned")
                .feed_stream_only(stream_id, &[], true);
            notify.notify_one();
        }
    }
}
