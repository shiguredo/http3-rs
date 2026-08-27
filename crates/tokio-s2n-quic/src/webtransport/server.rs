//! WebTransport サーバー

use std::net::SocketAddr;
use std::sync::{Arc, Mutex as StdMutex};

use bytes::Bytes;
use s2n_quic::connection::BidirectionalStreamAcceptor;
use shiguredo_http3::webtransport::error::ErrorCode as WtErrorCode;
use shiguredo_http3::webtransport::{ConnectError, ConnectRequest, ConnectResponse};
use shiguredo_http3::{Event, WebTransportEvent};
use tokio::sync::Notify;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::session::{
    WtRecvStream, WtSession, is_forwardable_wt_event, synthesized_session_closed,
};
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
    /// CONNECT ストリームの送信端 (CONNECT レスポンス送信用。`accept()` 後は `WtSession` に move される)
    send_stream: s2n_quic::stream::SendStream,
    /// CONNECT ストリームの受信端 (`accept()` で受信タスクへ引き渡す。ドロップすると STOP_SENDING が飛ぶため保持する)
    recv_stream: s2n_quic::stream::ReceiveStream,
    /// 双方向ストリームアクセプター
    bidi_acceptor: BidirectionalStreamAcceptor,
    /// 接続ハンドル
    handle: s2n_quic::connection::Handle,
    /// WT 単方向ストリーム受信チャネル
    uni_rx: mpsc::Receiver<WtRecvStream>,
    /// ハンドシェイク中に到着した WebTransport イベント (`accept()` で受信タスクへ引き継ぐ)
    pending_wt_events: Vec<WebTransportEvent>,
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
        let (mut recv_stream, mut send_stream) = stream.split();

        // Sans I/O にストリームの存在を通知
        {
            let mut s = state.lock().expect("mutex should not be poisoned");
            let _ = s.h3_conn.feed_stream(connect_stream_id, &[], false);
        }

        // peer SETTINGS の受信を待つ (WebTransport CONNECT の検証に必要)
        // uni_task がクライアントの制御ストリームを処理して SETTINGS を注入するまで待機する
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

        // CONNECT リクエストを受信
        //
        // QPACK エンコーダーストリームが先に到着してブロック解除される場合に備え、
        // recv_stream.receive() と unblock_notify.notified() を select! で待つ。
        //
        // 全ヘッダーを収集し、HeadersEnd で `ConnectRequest::from_headers` により
        // `:method = CONNECT` / `:protocol = webtransport-h3 | webtransport` を検証する
        // (draft-ietf-webtrans-http3-16 Section 3.2)。
        //
        // ハンドシェイク中に到着した WebTransport イベント (楽観的な WT_CLOSE_SESSION 等) は
        // 破棄せず `pending_wt_events` に貯めて受信タスクへ引き継ぐ。
        let mut collected_headers: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        let mut headers_complete = false;
        let mut pending_wt_events: Vec<WebTransportEvent> = Vec::new();

        while !headers_complete {
            let events = state
                .lock()
                .expect("mutex should not be poisoned")
                .drain_events()?;
            collect_request_events(
                events,
                &mut collected_headers,
                &mut headers_complete,
                &mut pending_wt_events,
            );
            if headers_complete {
                break;
            }

            tokio::select! {
                received = recv_stream.receive() => {
                    let (data, fin) = match received {
                        Ok(Some(data)) => (data.to_vec(), false),
                        Ok(None) => (vec![], true),
                        Err(e) => return Err(crate::Error::transport(e)),
                    };

                    let events = match state
                        .lock()
                        .expect("mutex should not be poisoned")
                        .process_stream_data(connect_stream_id, &data, fin)
                    {
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
                    };
                    collect_request_events(
                        events,
                        &mut collected_headers,
                        &mut headers_complete,
                        &mut pending_wt_events,
                    );

                    if fin && !headers_complete {
                        // ピア側で FIN のみ送信 (拒否・切断) されて CONNECT ヘッダーが揃わなかった
                        return Err(crate::Error::ConnectionClosed);
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

        // 収集したヘッダーを `ConnectRequest::from_headers` で検証する。
        // `:method` / `:protocol` が不正な場合 (通常の GET 等) はピアに 405 を返してから Err にする
        // (draft-ietf-webtrans-http3-16 Section 3.2: 非対応リソースへの応答は 405 SHOULD)。
        //
        // それ以外の `ConnectError` は原則 sans-I/O 層の `validate_wt_connect_request_server` が
        // 先に弾くが、`InvalidEncoding` はエッジケースで到達し得る (例: `origin` /
        // `wt-available-protocols` 値の非 UTF-8。sans-I/O 側の `is_valid_field_value` は
        // obs-text `0x80..=0xFF` を許容するため通過する)。到達時は Internal エラーで
        // ハンドシェイクを中断する (RESET_STREAM 送出は行わない)。
        let header_refs: Vec<(&[u8], &[u8])> = collected_headers
            .iter()
            .map(|(n, v)| (n.as_slice(), v.as_slice()))
            .collect();
        let connect_request = match ConnectRequest::from_headers(&header_refs) {
            Ok(req) => req,
            Err(ConnectError::InvalidMethod | ConnectError::InvalidProtocol) => {
                send_reject_response(&mut send_stream, &state, connect_stream_id, 405).await?;
                return Err(crate::Error::ConnectionClosed);
            }
            Err(e) => {
                return Err(crate::Error::Internal(format!(
                    "invalid CONNECT request: {e}"
                )));
            }
        };

        Ok(Self {
            path: connect_request.path.into_bytes(),
            authority: connect_request.authority.into_bytes(),
            stream_id: connect_stream_id,
            state,
            send_stream,
            recv_stream,
            bidi_acceptor,
            handle,
            uni_rx,
            pending_wt_events,
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
        let (data, fin) = {
            let mut s = self.state.lock().expect("mutex should not be poisoned");
            s.h3_conn
                .send_response(self.stream_id, &response_headers, false)?;

            // エンコード済みデータを取得 (fin=false のためデータのみ)
            // fin=true を交付する場合に備えて fin 受領で break し finish() すること
            let mut buf = Vec::new();
            let mut fin = false;
            while let Some((chunk, f)) = s.get_stream_data(self.stream_id) {
                buf.extend_from_slice(&chunk);
                fin = f;
                if fin {
                    break;
                }
            }
            (buf, fin)
        };

        // レスポンスを送信
        let mut send_stream = self.send_stream;
        send_stream
            .send(Bytes::from(data))
            .await
            .map_err(crate::Error::transport)?;

        // FIN 交付を受領した場合のみ finish() する
        if fin {
            send_stream.finish().map_err(crate::Error::transport)?;
        }

        // send_response の内部で establish_wt_session_server が走り、
        // 楽観的にバッファされていた WT_CLOSE_SESSION 等が sans-I/O イベントキューに
        // push されている可能性がある。受信タスク起動前にドレインして
        // `pending_wt_events` に追加する (draft-ietf-webtrans-http3-16 Section 3.2)。
        let mut pending_wt_events = self.pending_wt_events;
        {
            let events = self
                .state
                .lock()
                .expect("mutex should not be poisoned")
                .drain_events()?;
            for event in events {
                if let shiguredo_http3::Event::WebTransport(wt) = event
                    && is_forwardable_wt_event(&wt)
                {
                    pending_wt_events.push(wt);
                }
            }
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
        let (event_tx, event_rx) = mpsc::channel::<WebTransportEvent>(64);
        let recv_task = tokio::spawn(run_server_connect_recv_task(
            self.stream_id,
            self.recv_stream,
            self.state,
            event_tx,
            pending_wt_events,
        ));

        Ok(WtSession::new(
            self.stream_id,
            self.bidi_acceptor,
            self.handle,
            send_stream,
            self.uni_rx,
            event_rx,
            recv_task,
        ))
    }

    /// セッションリクエストを拒否する
    ///
    /// `status` は 4xx または 5xx でなければならない。
    ///
    /// 拒否時に `self.recv_stream` は本関数終了で暗黙 drop され、s2n-quic の
    /// `ReceiveStream::drop` が STOP_SENDING を送出する。拒否ケースでは以降の
    /// カプセルを受信する意味がないため意図した挙動 (受信タスクは起動しない)。
    pub async fn reject(self, status: u16) -> crate::Result<()> {
        if !(400..=599).contains(&status) {
            return Err(crate::Error::InvalidState(format!(
                "reject status must be 4xx or 5xx, got {status}"
            )));
        }
        let mut send_stream = self.send_stream;
        send_reject_response(&mut send_stream, &self.state, self.stream_id, status).await
    }
}

/// CONNECT ハンドシェイクループのイベント処理を切り出したヘルパー
///
/// - `Event::Header`: 全ヘッダーを `collected_headers` に収集する (`ConnectRequest::from_headers` 用)
/// - `Event::HeadersEnd`: ヘッダー受信完了
/// - 転送対象の WebTransport イベント: `pending_wt_events` にバッファ
/// - その他: 破棄
fn collect_request_events(
    events: Vec<Event>,
    collected_headers: &mut Vec<(Vec<u8>, Vec<u8>)>,
    headers_complete: &mut bool,
    pending_wt_events: &mut Vec<WebTransportEvent>,
) {
    for event in events {
        match event {
            Event::Header { name, value, .. } => {
                collected_headers.push((name, value));
            }
            Event::HeadersEnd { .. } => {
                *headers_complete = true;
            }
            Event::WebTransport(wt) if is_forwardable_wt_event(&wt) => {
                pending_wt_events.push(wt);
            }
            _ => {}
        }
    }
}

/// CONNECT リクエスト拒否レスポンス (4xx/5xx) を送信する共通ヘルパー
///
/// `from_connection` (405 送信) と `WtSessionRequest::reject` の共通実装。
/// `self` を消費せず `send_stream` / `state` を借用する形にすることで、
/// `WtSessionRequest` が確定していない `from_connection` 内からも利用できる。
async fn send_reject_response(
    send_stream: &mut s2n_quic::stream::SendStream,
    state: &Arc<StdMutex<ServerConnectionState>>,
    stream_id: u64,
    status: u16,
) -> crate::Result<()> {
    let response_headers = ConnectResponse::new(status).to_headers()?;

    let (data, fin) = {
        let mut s = state.lock().expect("mutex should not be poisoned");
        s.h3_conn
            .send_response(stream_id, &response_headers, true)?;
        // FIN 交付までデータをドレインする (send_response の fin=true 指定に対応)
        let mut buf = Vec::new();
        let mut fin = false;
        while let Some((chunk, f)) = s.get_stream_data(stream_id) {
            buf.extend_from_slice(&chunk);
            fin = f;
            if fin {
                break;
            }
        }
        (buf, fin)
    };

    send_stream
        .send(Bytes::from(data))
        .await
        .map_err(crate::Error::transport)?;

    if fin {
        send_stream.finish().map_err(crate::Error::transport)?;
    }

    Ok(())
}

async fn run_server_connect_recv_task(
    session_id: u64,
    mut recv_stream: s2n_quic::stream::ReceiveStream,
    state: Arc<StdMutex<ServerConnectionState>>,
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
