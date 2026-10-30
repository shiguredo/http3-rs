//! WebTransport クライアント

use std::sync::{Arc, Mutex as StdMutex};

use bytes::Bytes;
use s2n_quic::client::Connect;
use shiguredo_http3::webtransport::Capsule;
use shiguredo_http3::webtransport::ConnectRequest;
use shiguredo_http3::webtransport::error::ErrorCode as WtErrorCode;
use shiguredo_http3::{Event, WebTransportEvent};
use tokio::sync::Notify;
use tokio::sync::mpsc;

use super::flow_control::{WtSendFlowControl, peer_wt_send_limits};
use super::session::{
    ConnectCommand, SharedSendFlowControl, UniStreamHeader, WtRecvStream, WtSession,
    apply_flow_control_capsule, classify_uni_stream_header, close_connection_with_id_error,
    flow_control_error_session_closed, is_forwardable_wt_event, run_connect_send_task,
    synthesized_session_closed, terminate_with_flow_control_error,
};
use crate::config::ClientConfig;
use crate::internal::classify_uni_recv;
use crate::internal::connection_state::ClientConnectionState;

/// WebTransport クライアント
pub struct WtClient;

impl WtClient {
    /// WebTransport セッションを確立する
    pub async fn connect(config: ClientConfig, path: &str) -> crate::Result<WtSession> {
        // TLS プロバイダ (ALPN) と limits (アイドルタイムアウト) は
        // ClientConfig::build_client が配線する
        let client = config.build_client()?;

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
            // s2n-quic 1.86 は HTTP/3 DATAGRAM (RFC 9221) をサポートするため
            // 第 1 引数 (max_datagram_frame_size_nonzero) は true。
            //
            // 第 2 引数は RESET_STREAM_AT のサポート可否。s2n-quic 1.86 は
            // RESET_STREAM_AT の API を公開しておらず (transport parameter も
            // フレームも未実装)、実際には使用できない。ただし sans-I/O 層は
            // draft-14/15 のピアに対して reset_stream_at のサポートを WebTransport
            // セッション確立の必須条件とするため、ここでは true を渡している
            // (draft-ietf-webtrans-http3-16 Section 3.1)。
            // s2n-quic が RESET_STREAM_AT を実装するまでの既知の限界。
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
        // 単方向ストリーム受信タスクの終了 (= 接続クローズ) を通知する
        //
        // peer SETTINGS 待ちが無限に待ち続けないよう、受信タスクの終了を待機側へ伝える。
        let uni_closed_notify = Arc::new(Notify::new());

        // WT 単方向ストリームを WtSession に渡すためのチャネル
        let (uni_tx, uni_rx) = mpsc::channel::<WtRecvStream>(16);

        // 単方向ストリーム受信タスク
        //
        // classify_uni_stream_header でストリームタイプを判別する:
        // - WebTransport: WT 単方向ストリーム → uni_tx に送信
        // - Http3: H3 制御ストリームなど → feed_stream_only でフィード
        let state_for_uni = Arc::clone(&state);
        let notify_for_uni = Arc::clone(&unblock_notify);
        let closed_for_uni = Arc::clone(&uni_closed_notify);
        let handle_for_uni = handle.clone();
        tokio::spawn(async move {
            while let Ok(Some(recv)) = uni_acceptor.accept_receive_stream().await {
                let state = Arc::clone(&state_for_uni);
                let notify = Arc::clone(&notify_for_uni);
                let uni_tx = uni_tx.clone();
                let stream_id: u64 = recv.id();
                let handle = handle_for_uni.clone();
                tokio::spawn(route_uni_stream(
                    connect_stream_id,
                    stream_id,
                    recv,
                    state,
                    notify,
                    uni_tx,
                    handle,
                ));
            }
            // acceptor が終了した = 接続がクローズした。peer SETTINGS はもう届かない。
            closed_for_uni.notify_one();
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
        // (draft-ietf-webtrans-http3-16 Section 3.1)。
        //
        // 10ms ポーリングは行わず Notify のみで待つ。条件確認 → 待機登録 → 再確認の
        // 順で取りこぼしを防ぐ (`Notified::enable` で待機登録を明示的に行う)。
        loop {
            let notified = unblock_notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let s = state.lock().expect("mutex should not be poisoned");
                if s.h3_conn.peer_settings().is_some() {
                    break;
                }
            }
            tokio::select! {
                _ = &mut notified => {}
                _ = uni_closed_notify.notified() => {
                    // 単方向ストリーム受信タスクの終了は接続クローズを意味する。
                    // SETTINGS が既に処理済みなら続行し、そうでなければ打ち切る。
                    let s = state.lock().expect("mutex should not be poisoned");
                    if s.h3_conn.peer_settings().is_some() {
                        break;
                    }
                    return Err(crate::Error::ConnectionClosed);
                }
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
        // セッション確立判定は sans-I/O 層の `WebTransportEvent::SessionEstablished` で行う。
        // sans-I/O 層は CONNECT レスポンスの `:status` が 2xx のときのみ SessionEstablished を
        // 発火するため、これを確立判定に使うことで `:status` の検証を委譲する
        // (draft-ietf-webtrans-http3-16 Section 3.2)。
        //
        // ただし非 2xx レスポンスでは SessionEstablished が発火せず、ピアが FIN を送らなければ
        // ハンドシェイクループが永久にブロックしうる。`:status` を監視して最終レスポンスの
        // 非 2xx を検出したら早期に Err で抜ける。1xx 中間レスポンス (例: 103 Early Hints) は
        // 失敗扱いにせずスキップする (RFC 9114 Section 4.1)。
        //
        // ハンドシェイク中に到着した WebTransport イベント (200 レスポンスと同一 receive で
        // 到着した終端カプセル等) は破棄せず `pending_wt_events` に貯めて受信タスクへ
        // 引き継ぐ。フロー制御カプセルも同様に `pending_flow_control_capsules` に貯め、
        // 受信タスクが送信側フロー制御へ反映する (取りこぼしを防ぐため)。
        let mut session_established = false;
        let mut latest_status: Option<u16> = None;
        let mut pending_wt_events: Vec<WebTransportEvent> = Vec::new();
        let mut pending_flow_control_capsules: Vec<Capsule> = Vec::new();
        loop {
            // QPACK エンコーダーストリームのデータが先に処理されて
            // notify_one() が取りこぼされる場合に備え、毎回 drain_events を確認する
            let events = state
                .lock()
                .expect("mutex should not be poisoned")
                .drain_events()?;
            process_handshake_events(
                events,
                &mut latest_status,
                &mut session_established,
                &mut pending_wt_events,
                &mut pending_flow_control_capsules,
            )?;
            if session_established {
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
                    process_handshake_events(
                        events,
                        &mut latest_status,
                        &mut session_established,
                        &mut pending_wt_events,
                        &mut pending_flow_control_capsules,
                    )?;

                    if fin && !session_established {
                        // ピア側で FIN のみ送信 (拒否・切断) されて確立できなかった
                        return Err(crate::Error::ConnectionClosed);
                    }
                }
                _ = unblock_notify.notified() => {
                    // QPACK エンコーダーストリームが更新された可能性がある
                    // ループ先頭の drain_events で処理する
                }
            }
        }

        // ピア SETTINGS の WT 初期値とフロー制御の有効判定から送信側フロー制御を構築する
        // (draft-ietf-webtrans-http3-16 Section 5.1, 5.5)。
        // フロー制御の有効判定は確立済みセッションを参照するため、セッション確立
        // (SessionEstablished の受信) 後に構築する。
        let send_fc: SharedSendFlowControl = {
            let s = state.lock().expect("mutex should not be poisoned");
            let limits = peer_wt_send_limits(s.h3_conn.peer_settings());
            let enabled = s.h3_conn.wt_session_flow_control_enabled(connect_stream_id);
            Arc::new(StdMutex::new(WtSendFlowControl::new(enabled, limits)))
        };

        // CONNECT ストリームの受信タスクを起動する。
        //
        // 受信データを sans-I/O 層へ流し、WT_CLOSE_SESSION カプセルの検知 /
        // CONNECT ストリームの FIN / RESET_STREAM を `WebTransportEvent::SessionClosed`
        // として `event_rx` に届ける (draft-ietf-webtrans-http3-16 Section 6)。
        // フロー制御カプセルは送信側フロー制御へ反映する
        // (draft-ietf-webtrans-http3-16 Section 5.6.2, 5.6.4)。
        //
        // event_rx の容量 64: 現状 CONNECT ストリーム経由で発火し得るのは接続あたり
        // ごく少数の `SessionClosed` / `SessionDraining` / `BufferedStreamRejected` のみ。
        // 将来 `StreamReset` / `Datagram` を wire したときのバースト吸収も見込んで 64 を確保する。
        //
        // クライアントは `send_response` 相当のステップがないため、`pending_wt_events` に
        // 対する追加ドレインは不要 (サーバー側の `accept()` は追加ドレインを行う)。
        let (event_tx, event_rx) = mpsc::channel::<WebTransportEvent>(64);
        // CONNECT ストリームの送信端を専用タスクへ移し、受信タスクの close 応答と
        // `WtSession::close` / `Drop` の双方から操作できるようにする
        // (draft-ietf-webtrans-http3-16 Section 6)
        let (connect_tx, connect_rx) = mpsc::unbounded_channel::<ConnectCommand>();
        // 送信タスクは connect_tx が全て drop されると終了するため detach する
        tokio::spawn(run_connect_send_task(send_stream, connect_rx));
        let recv_task = tokio::spawn(run_client_connect_recv_task(
            connect_stream_id,
            recv_stream,
            Arc::clone(&state),
            event_tx,
            pending_wt_events,
            pending_flow_control_capsules,
            Arc::clone(&send_fc),
            connect_tx.clone(),
        ));

        Ok(WtSession::new(
            connect_stream_id,
            bidi_acceptor,
            handle,
            connect_tx,
            uni_rx,
            event_rx,
            send_fc,
            recv_task,
        ))
    }
}

/// CONNECT ハンドシェイクループのイベント処理を切り出したヘルパー
///
/// 各イベントに対する状態遷移:
/// - `:status` ヘッダー: 直近のレスポンスステータスコードを更新
/// - `HeadersEnd`: `:status` が非 2xx かつ非 1xx なら早期エラー、1xx なら次のレスポンスを待つため
///   状態をクリアする (次のレスポンス `:status` が来ないまま HeadersEnd が飛んだ場合に古い 1xx を
///   最終レスポンス扱いしないよう防御的にクリアする)。sans-I/O 層の `validate_response_headers` が
///   `:status` を必須化しているため `latest_status = None` の分岐は実装上到達しない
/// - `SessionEstablished`: セッション確立完了
/// - **確立前** の `SessionClosed` / `SessionDraining`: sans-I/O 層で AlpnError / wt-protocol 不正等が
///   検知され `SessionEstablished` が発火しないケース。確立不能として `ConnectionClosed` を返す
/// - **確立後** の `SessionClosed` / `SessionDraining`: 200 OK と同じ receive で WT_CLOSE_SESSION や
///   WT_DRAIN_SESSION が届いたケース。ここではエラーにせず `pending_wt_events` に積んで受信タスクへ
///   引き継ぎ、`close_error_code` / `close_message` などの終端情報を保持する
/// - フロー制御カプセル (`Capsule`): `pending_flow_control_capsules` に積んで受信タスクへ引き継ぎ、
///   送信側フロー制御へ反映する (draft-ietf-webtrans-http3-16 Section 5.6.2, 5.6.4)
/// - 転送対象の他 WebTransport イベント (`BufferedStreamRejected` 等): `pending_wt_events` にバッファ
/// - その他: 破棄
///
/// 1xx 範囲判定は `100..200` を使う。RFC 9114 Section 4.5 で HTTP/3 は 101 を持たず、sans-I/O 層の
/// `validate_response_headers` (src/validation.rs) が 101 の `:status` を拒否するため 101 が届く
/// 経路は無い。
fn process_handshake_events(
    events: Vec<Event>,
    latest_status: &mut Option<u16>,
    session_established: &mut bool,
    pending_wt_events: &mut Vec<WebTransportEvent>,
    pending_flow_control_capsules: &mut Vec<Capsule>,
) -> crate::Result<()> {
    for event in events {
        match event {
            Event::Header { name, value, .. } if name == b":status" => {
                *latest_status = parse_status(&value);
            }
            Event::HeadersEnd { .. } => {
                if let Some(status) = *latest_status {
                    if (100..200).contains(&status) {
                        // 1xx 中間レスポンス: 次のレスポンスヘッダーを待つ (RFC 9114 Section 4.1)。
                        // 保持し続けても error 分岐には落ちないが、次のレスポンス開始まで状態を
                        // 持ち越さないことで各レスポンスの評価を独立させる。
                        *latest_status = None;
                    } else if !(200..300).contains(&status) {
                        // 非 2xx / 非 1xx の最終レスポンス: セッション確立失敗
                        // (draft-ietf-webtrans-http3-16 Section 3.2)
                        return Err(crate::Error::ConnectionClosed);
                    }
                }
            }
            Event::WebTransport(WebTransportEvent::SessionEstablished { .. }) => {
                *session_established = true;
            }
            Event::WebTransport(
                WebTransportEvent::SessionClosed { .. } | WebTransportEvent::SessionDraining { .. },
            ) if !*session_established => {
                // 確立前にセッション終端イベントが届くのは sans-I/O 層でセットアップエラーが
                // 検出された場合 (例: AlpnError で wt-protocol が不正)。この時点で
                // `SessionEstablished` は発火しないため、確立ループを打ち切る必要がある。
                return Err(crate::Error::ConnectionClosed);
            }
            Event::WebTransport(WebTransportEvent::Capsule { capsule, .. }) => {
                // フロー制御カプセルは受信タスクが送信側フロー制御へ反映する。
                // 接続上の WT セッションは本クライアントが 1 つだけ確立するため
                // session_id による絞り込みは行わない。
                pending_flow_control_capsules.push(capsule);
            }
            Event::WebTransport(wt) if is_forwardable_wt_event(&wt) => {
                // 確立後の SessionClosed / SessionDraining はここに落ちる (guard で早期エラー化されない)。
                // pending_wt_events に積むことで `close_error_code` / `close_message` を保持したまま
                // 受信タスクへ引き継ぐ。
                pending_wt_events.push(wt);
            }
            _ => {}
        }
    }
    Ok(())
}

/// `:status` ヘッダー値をパースしてステータスコードを返す
///
/// sans-I/O 層の `validate_response_headers` が `:status` を「3 桁の ASCII 数字」に制限するため、
/// 通常は必ず `Some(u16)` が返る。到達不能な非パースケースは `None` として静かに無視する。
fn parse_status(value: &[u8]) -> Option<u16> {
    std::str::from_utf8(value).ok().and_then(|s| s.parse().ok())
}

#[expect(
    clippy::too_many_arguments,
    reason = "ハンドシェイクで確定した共有状態をそのまま受信タスクへ引き継ぐため"
)]
async fn run_client_connect_recv_task(
    session_id: u64,
    recv_stream: s2n_quic::stream::ReceiveStream,
    state: Arc<StdMutex<ClientConnectionState>>,
    event_tx: mpsc::Sender<WebTransportEvent>,
    pending_wt_events: Vec<WebTransportEvent>,
    pending_flow_control_capsules: Vec<Capsule>,
    send_fc: SharedSendFlowControl,
    connect_tx: mpsc::UnboundedSender<ConnectCommand>,
) {
    run_client_connect_recv_task_inner(
        session_id,
        recv_stream,
        state,
        event_tx,
        pending_wt_events,
        pending_flow_control_capsules,
        &send_fc,
        &connect_tx,
    )
    .await;
    // 受信タスクの終了は CONNECT ストリームの終端を意味するため、送信側も close する
    // (draft-ietf-webtrans-http3-16 Section 6: WT_CLOSE_SESSION の受信側は close か reset を返す)
    let _ = connect_tx.send(ConnectCommand::Finish);
}

#[expect(
    clippy::too_many_arguments,
    reason = "ハンドシェイクで確定した共有状態をそのまま受信タスクへ引き継ぐため"
)]
async fn run_client_connect_recv_task_inner(
    session_id: u64,
    mut recv_stream: s2n_quic::stream::ReceiveStream,
    state: Arc<StdMutex<ClientConnectionState>>,
    event_tx: mpsc::Sender<WebTransportEvent>,
    pending_wt_events: Vec<WebTransportEvent>,
    pending_flow_control_capsules: Vec<Capsule>,
    send_fc: &SharedSendFlowControl,
    connect_tx: &mpsc::UnboundedSender<ConnectCommand>,
) {
    // ハンドシェイク中に到着していたフロー制御カプセルを反映する。
    // 単調性違反は WT_FLOW_CONTROL_ERROR でセッションを終了する
    // (draft-ietf-webtrans-http3-16 Section 5.6.2, 5.6.4)。
    for capsule in pending_flow_control_capsules {
        if apply_flow_control_capsule(send_fc, &capsule).is_err() {
            terminate_with_flow_control_error(connect_tx);
            let _ = event_tx
                .send(flow_control_error_session_closed(session_id))
                .await;
            return;
        }
    }

    // ハンドシェイク中に到着していた WebTransport イベントを先に流す。
    // SessionClosed を転送してもタスクは終了せず、ピアの FIN / Err まで読み続ける。
    // 早期に終了すると WT_CLOSE_SESSION 後の追加 DATA を読めず、
    // H3_MESSAGE_ERROR での reset (draft-ietf-webtrans-http3-16 Section 6 の MUST) に
    // 到達できない。
    // 実 SessionClosed を転送済みか (Err 分岐で synthesized を再送しないため)
    let mut session_closed_delivered = false;
    for event in pending_wt_events {
        let is_terminal = matches!(event, WebTransportEvent::SessionClosed { .. });
        if event_tx.send(event).await.is_err() {
            return;
        }
        if is_terminal {
            session_closed_delivered = true;
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
            Err(err) => {
                // WT_CLOSE_SESSION 後の追加データは H3_MESSAGE_ERROR で reset する
                // (draft-ietf-webtrans-http3-16 Section 6 の MUST)。
                if matches!(
                    &err,
                    crate::Error::Http3(shiguredo_http3::Error::StreamError(
                        shiguredo_http3::ErrorCode::MessageError
                    ))
                ) {
                    let _ = connect_tx.send(ConnectCommand::Reset {
                        error_code: shiguredo_http3::ErrorCode::MessageError.code(),
                    });
                }
                // sans-I/O 層が Err を返しても、既に event queue に push 済みの
                // `SessionClosed` (WT_CLOSE_SESSION 受信済みのケース等) が沈殿している
                // 可能性があるため、まず drain_events を試みる。
                //
                // 取り出せた `SessionClosed` を優先的に届け、それも無い場合のみ
                // 合成イベントで終端を通知する。
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
                if !delivered && !session_closed_delivered {
                    let _ = event_tx.send(synthesized_session_closed(session_id)).await;
                }
                return;
            }
        };

        let mut flow_control_violated = false;
        for event in events {
            match event {
                Event::WebTransport(WebTransportEvent::Capsule {
                    session_id: capsule_session_id,
                    capsule,
                }) => {
                    // フロー制御カプセルを送信側フロー制御へ反映する
                    // (draft-ietf-webtrans-http3-16 Section 5.6.2, 5.6.4)。
                    // 他セッション宛のカプセルは本 WtSession には適用しない。
                    flow_control_violated |= capsule_session_id == session_id
                        && apply_flow_control_capsule(send_fc, &capsule).is_err();
                }
                Event::WebTransport(wt) if is_forwardable_wt_event(&wt) => {
                    let is_terminal = matches!(wt, WebTransportEvent::SessionClosed { .. });
                    if event_tx.send(wt).await.is_err() {
                        return;
                    }
                    if is_terminal {
                        session_closed_delivered = true;
                    }
                }
                _ => {}
            }
        }

        if flow_control_violated {
            // 単調性違反: WT_FLOW_CONTROL_ERROR でセッションを終了する
            // (draft-ietf-webtrans-http3-16 Section 5.6.2, 5.6.4)
            terminate_with_flow_control_error(connect_tx);
            if !session_closed_delivered {
                let _ = event_tx
                    .send(flow_control_error_session_closed(session_id))
                    .await;
            }
            return;
        }

        // SessionClosed を転送しても、ピアの FIN を読み切るまでは return しない。
        // 早期に recv_stream を drop すると STOP_SENDING が送出され、ピアの送信
        // (WT_CLOSE_SESSION カプセル + FIN) と競合して RESET_STREAM になりうる。
        if terminated {
            return;
        }
    }
}

/// 単方向ストリームをタイプで判別してルーティングする
///
/// `classify_uni_stream_header` で先頭バイトからストリームタイプを分類する:
/// - WebTransport: WT 単方向ストリーム → uni_tx に送信
/// - Http3: H3 制御ストリームなど → feed_stream_only でフィード
/// - 不正な session ID: H3_ID_ERROR で接続を閉じる
///   (draft-ietf-webtrans-http3-16 Section 4)
async fn route_uni_stream(
    session_id: u64,
    stream_id: u64,
    mut recv_stream: s2n_quic::stream::ReceiveStream,
    state: Arc<StdMutex<ClientConnectionState>>,
    notify: Arc<Notify>,
    uni_tx: mpsc::Sender<WtRecvStream>,
    handle: s2n_quic::connection::Handle,
) {
    let mut type_buf: Vec<u8> = Vec::new();

    // ストリームタイプと (WT ストリームの場合は) session ID が確定するまでデータを読む
    let header = loop {
        match recv_stream.receive().await {
            Ok(Some(data)) => {
                type_buf.extend_from_slice(&data);
                if let Some(header) = classify_uni_stream_header(&type_buf, Some(session_id)) {
                    break header;
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

    match header {
        UniStreamHeader::WebTransport { data_offset } => {
            // WT 単方向ストリームとして WtSession に渡す
            let pending = type_buf[data_offset..].to_vec();
            let wt_recv = WtRecvStream::new(stream_id, recv_stream, pending);
            let _ = uni_tx.send(wt_recv).await;
        }
        UniStreamHeader::Http3 => {
            // H3 制御ストリームとしてフィード
            let _ = state
                .lock()
                .expect("mutex should not be poisoned")
                .feed_stream_only(stream_id, &type_buf, false);
            notify.notify_one();

            loop {
                let action = classify_uni_recv(recv_stream.receive().await);
                // 受信アクションを sans-I/O 層へ適用する。クリティカルストリームの
                // RESET は H3_CLOSED_CRITICAL_STREAM としてラッチされる
                // (RFC 9114 Section 6.2.1 / RFC 9204 Section 4.2)。
                // 接続エラー等の Err はラッチ済みのためタスクを終了する
                // (unwrap_or_default() で false になる)
                let cont = state
                    .lock()
                    .expect("mutex should not be poisoned")
                    .apply_uni_recv_action(stream_id, action)
                    .unwrap_or_default();
                notify.notify_one();
                if !cont {
                    break;
                }
            }
        }
        UniStreamHeader::MalformedSessionId => {
            // 形式不正な session ID。sans-I/O 層にも接続エラーを記録させたうえで、
            // 接続を H3_ID_ERROR で閉じる (draft-ietf-webtrans-http3-16 Section 4 の MUST)。
            let _ = state
                .lock()
                .expect("mutex should not be poisoned")
                .feed_stream_only(stream_id, &type_buf, false);
            notify.notify_one();
            close_connection_with_id_error(&handle);
        }
        UniStreamHeader::ForeignSession => {
            // 形式は正しいが本セッション以外の session ID。本クレートは接続上の
            // 単方向ストリームアクセプターを単一の WtSession が保持するため
            // 他セッションへ配送できず、接続状態を維持したまま処理を続けられない
            // (draft-ietf-webtrans-http3-16 Section 4)
            close_connection_with_id_error(&handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shiguredo_http3::event::WtStreamReset;

    const STREAM_ID: u64 = 0;

    /// テスト用: `Event::Header` を組み立てる
    fn header(name: &[u8], value: &[u8]) -> Event {
        Event::Header {
            stream_id: STREAM_ID,
            name: name.to_vec(),
            value: value.to_vec(),
        }
    }

    /// テスト用: `Event::HeadersEnd` を組み立てる
    fn headers_end() -> Event {
        Event::HeadersEnd {
            stream_id: STREAM_ID,
        }
    }

    /// テスト用: `SessionEstablished` イベントを組み立てる
    fn wt_session_established() -> Event {
        Event::WebTransport(WebTransportEvent::SessionEstablished {
            session_id: STREAM_ID,
            flow_control_enabled: true,
        })
    }

    /// テスト用: `SessionClosed` イベントを組み立てる (`close_error_code` / `close_message` は保持される値)
    fn wt_session_closed() -> Event {
        Event::WebTransport(WebTransportEvent::SessionClosed {
            session_id: STREAM_ID,
            reset_streams: Vec::<WtStreamReset>::new(),
            error_code: 0,
            close_error_code: 42,
            close_message: String::from("bye"),
        })
    }

    /// テスト用: `SessionDraining` イベントを組み立てる
    fn wt_session_draining() -> Event {
        Event::WebTransport(WebTransportEvent::SessionDraining {
            session_id: STREAM_ID,
        })
    }

    /// テスト用: `BufferedStreamRejected` イベントを組み立てる
    fn wt_buffered_stream_rejected() -> Event {
        Event::WebTransport(WebTransportEvent::BufferedStreamRejected {
            stream_id: 4,
            error_code: 0,
        })
    }

    /// テスト用: フロー制御カプセル (`WT_MAX_STREAMS`) イベントを組み立てる
    fn wt_capsule() -> Event {
        Event::WebTransport(WebTransportEvent::Capsule {
            session_id: STREAM_ID,
            capsule: Capsule::MaxStreams {
                bidirectional: true,
                maximum: 10,
            },
        })
    }

    #[test]
    fn test_process_handshake_events_1xx_then_2xx_without_established_waits_next_response() {
        // 1xx (103) → 2xx (200) の HeadersEnd が飛んでも、SessionEstablished が来ていない
        // 状態では established は立たず (Err にもならず) 次のレスポンスを待つ経路が成立する
        let events = vec![
            header(b":status", b"103"),
            headers_end(),
            header(b":status", b"200"),
            headers_end(),
        ];
        let mut status = None;
        let mut established = false;
        let mut pending = Vec::new();
        let mut capsules = Vec::new();
        process_handshake_events(
            events,
            &mut status,
            &mut established,
            &mut pending,
            &mut capsules,
        )
        .expect("1xx → 2xx 経路が Err にならないこと");
        assert!(
            !established,
            "SessionEstablished が来ないため established は false のまま"
        );
        assert!(pending.is_empty(), "終端イベント無しなので pending は空");
        assert!(capsules.is_empty(), "カプセルイベント無しなので空");
    }

    #[test]
    fn test_process_handshake_events_1xx_then_2xx_established() {
        // 1xx (103) → 2xx (200) + SessionEstablished で確立成功
        let events = vec![
            header(b":status", b"103"),
            headers_end(),
            header(b":status", b"200"),
            headers_end(),
            wt_session_established(),
        ];
        let mut status = None;
        let mut established = false;
        let mut pending = Vec::new();
        let mut capsules = Vec::new();
        process_handshake_events(
            events,
            &mut status,
            &mut established,
            &mut pending,
            &mut capsules,
        )
        .expect("1xx → 2xx + SessionEstablished 経路が Err にならないこと");
        assert!(established, "SessionEstablished 経由で確立判定されること");
        assert!(
            pending.is_empty(),
            "確立系イベントは pending に積まれないこと"
        );
    }

    #[test]
    fn test_process_handshake_events_non_2xx_after_1xx_returns_connection_closed() {
        // 1xx の後に非 2xx (404) が来たら早期エラー
        let events = vec![
            header(b":status", b"103"),
            headers_end(),
            header(b":status", b"404"),
            headers_end(),
        ];
        let mut status = None;
        let mut established = false;
        let mut pending = Vec::new();
        let mut capsules = Vec::new();
        let result = process_handshake_events(
            events,
            &mut status,
            &mut established,
            &mut pending,
            &mut capsules,
        );
        assert!(
            matches!(result, Err(crate::Error::ConnectionClosed)),
            "非 2xx で ConnectionClosed になること: {result:?}"
        );
        assert!(!established, "確立判定されないこと");
    }

    #[test]
    fn test_process_handshake_events_direct_non_2xx_returns_connection_closed() {
        // 1xx なしで非 2xx (405) が来ても即エラー
        let events = vec![header(b":status", b"405"), headers_end()];
        let mut status = None;
        let mut established = false;
        let mut pending = Vec::new();
        let mut capsules = Vec::new();
        let result = process_handshake_events(
            events,
            &mut status,
            &mut established,
            &mut pending,
            &mut capsules,
        );
        assert!(
            matches!(result, Err(crate::Error::ConnectionClosed)),
            "非 2xx で ConnectionClosed になること: {result:?}"
        );
    }

    #[test]
    fn test_process_handshake_events_session_established_alone_marks_established() {
        // `SessionEstablished` 単発でも確立判定される (:status 検証は sans-I/O に委譲)
        let events = vec![wt_session_established()];
        let mut status = None;
        let mut established = false;
        let mut pending = Vec::new();
        let mut capsules = Vec::new();
        process_handshake_events(
            events,
            &mut status,
            &mut established,
            &mut pending,
            &mut capsules,
        )
        .expect("SessionEstablished 単発が Err にならないこと");
        assert!(established, "確立判定されること");
    }

    #[test]
    fn test_process_handshake_events_early_session_closed_returns_connection_closed() {
        // 確立前に SessionClosed が届いたら早期エラー (200 OK + AlpnError 等のセットアップ失敗ケース)
        let events = vec![
            wt_session_closed(),
            header(b":status", b"200"),
            headers_end(),
        ];
        let mut status = None;
        let mut established = false;
        let mut pending = Vec::new();
        let mut capsules = Vec::new();
        let result = process_handshake_events(
            events,
            &mut status,
            &mut established,
            &mut pending,
            &mut capsules,
        );
        assert!(
            matches!(result, Err(crate::Error::ConnectionClosed)),
            "確立前 SessionClosed で ConnectionClosed になること: {result:?}"
        );
    }

    #[test]
    fn test_process_handshake_events_early_session_draining_returns_connection_closed() {
        // 確立前に SessionDraining が届いたら早期エラー (SessionClosed と同じ or パターン)
        let events = vec![wt_session_draining()];
        let mut status = None;
        let mut established = false;
        let mut pending = Vec::new();
        let mut capsules = Vec::new();
        let result = process_handshake_events(
            events,
            &mut status,
            &mut established,
            &mut pending,
            &mut capsules,
        );
        assert!(
            matches!(result, Err(crate::Error::ConnectionClosed)),
            "確立前 SessionDraining で ConnectionClosed になること: {result:?}"
        );
    }

    #[test]
    fn test_process_handshake_events_session_established_then_session_closed_is_forwarded() {
        // 確立後に SessionClosed が届いた場合、エラーにせず pending_wt_events に転送する
        // (200 OK + WT_CLOSE_SESSION が同一 receive で届いたケース。regression 防止)
        let events = vec![
            wt_session_established(),
            header(b":status", b"200"),
            headers_end(),
            wt_session_closed(),
        ];
        let mut status = None;
        let mut established = false;
        let mut pending = Vec::new();
        let mut capsules = Vec::new();
        process_handshake_events(
            events,
            &mut status,
            &mut established,
            &mut pending,
            &mut capsules,
        )
        .expect("確立後の SessionClosed はエラーで返さないこと");
        assert!(established, "確立判定されること");
        assert_eq!(
            pending.len(),
            1,
            "確立後の SessionClosed は pending_wt_events に転送されること"
        );
        match &pending[0] {
            WebTransportEvent::SessionClosed {
                close_error_code,
                close_message,
                ..
            } => {
                assert_eq!(
                    *close_error_code, 42,
                    "close_error_code が保持されていること"
                );
                assert_eq!(close_message, "bye", "close_message が保持されていること");
            }
            other => panic!("SessionClosed 以外が pending に入った: {other:?}"),
        }
    }

    #[test]
    fn test_process_handshake_events_buffered_stream_rejected_is_forwarded() {
        // 転送対象の WebTransport イベント (BufferedStreamRejected) は pending_wt_events に積まれる
        let events = vec![wt_buffered_stream_rejected()];
        let mut status = None;
        let mut established = false;
        let mut pending = Vec::new();
        let mut capsules = Vec::new();
        process_handshake_events(
            events,
            &mut status,
            &mut established,
            &mut pending,
            &mut capsules,
        )
        .expect("BufferedStreamRejected が Err にならないこと");
        assert!(
            !established,
            "確立系イベントではないので established は false"
        );
        assert_eq!(
            pending.len(),
            1,
            "BufferedStreamRejected は pending_wt_events に転送されること"
        );
    }

    #[test]
    fn test_process_handshake_events_flow_control_capsule_is_collected() {
        // フロー制御カプセルは pending_wt_events ではなく
        // pending_flow_control_capsules に積まれ、受信タスクへ引き継がれること
        // (draft-ietf-webtrans-http3-16 Section 5.6.2)
        let events = vec![wt_capsule()];
        let mut status = None;
        let mut established = false;
        let mut pending = Vec::new();
        let mut capsules = Vec::new();
        process_handshake_events(
            events,
            &mut status,
            &mut established,
            &mut pending,
            &mut capsules,
        )
        .expect("フロー制御カプセルが Err にならないこと");
        assert!(
            pending.is_empty(),
            "カプセルは pending_wt_events に積まれないこと"
        );
        assert_eq!(capsules.len(), 1, "カプセルが引き継がれること");
        assert_eq!(
            capsules[0],
            Capsule::MaxStreams {
                bidirectional: true,
                maximum: 10,
            }
        );
    }
}
