//! WebTransport セッションとストリーム型

use std::sync::{Arc, Mutex as StdMutex};

use bytes::Bytes;
use s2n_quic::connection::BidirectionalStreamAcceptor;
use s2n_quic::stream::{ReceiveStream, SendStream};
use shiguredo_http3::WebTransportEvent;
use shiguredo_http3::webtransport::capsule::{Capsule, CapsuleEncodeError};
use shiguredo_http3::webtransport::error::ErrorCode as WtErrorCode;
use shiguredo_http3::webtransport::stream::{StreamHeader, StreamHeaderDecodeError};
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

/// CONNECT ストリーム送信タスクへの指示
pub(crate) enum ConnectCommand {
    /// データを送信して FIN を送る (`WtSession::close`)
    SendAndFinish {
        /// 送信するカプセルデータ (H3 DATA フレーム)
        data: Bytes,
        /// 送信結果の通知先
        done: oneshot::Sender<crate::Result<()>>,
    },
    /// FIN のみを送る (受信タスクの close 応答 / `Drop`)
    Finish,
    /// RESET_STREAM を送る (WT_CLOSE_SESSION 後の追加データを拒否する経路)
    Reset {
        /// アプリケーションエラーコード
        error_code: u64,
    },
    /// データを送信する (FIN は送らない)
    ///
    /// WebTransport のフロー制御カプセル (`WT_MAX_STREAMS` / `WT_MAX_DATA` /
    /// `WT_STREAMS_BLOCKED` / `WT_DATA_BLOCKED`) をセッション継続中に送出するために
    /// 使う。CONNECT ストリームはセッション中クローズしないため FIN を送らない
    /// (draft-ietf-webtrans-http3-16 Section 5.6)。
    Send {
        /// 送信するカプセルデータ (H3 DATA フレーム)
        data: Bytes,
    },
}

/// CONNECT ストリームの送信端を所有し、指示を順に処理するタスク
///
/// `WtSession::close` / `Drop` / CONNECT ストリーム受信タスクの 3 経路から
/// 同じ送信端を操作するため、所有権をこのタスクに集約して mpsc で指示を送る。
pub(crate) async fn run_connect_send_task(
    mut send: SendStream,
    mut rx: mpsc::UnboundedReceiver<ConnectCommand>,
) {
    while let Some(command) = rx.recv().await {
        match command {
            ConnectCommand::SendAndFinish { data, done } => {
                let result = async {
                    send.send(data).await.map_err(crate::Error::transport)?;
                    send.finish().map_err(crate::Error::transport)
                }
                .await;
                let _ = done.send(result);
            }
            ConnectCommand::Finish => {
                // 既に FIN 送信済みの場合は冪等 (s2n-quic はエラーを返さない)
                let _ = send.finish();
            }
            ConnectCommand::Send { data } => {
                // フロー制御カプセルは CONNECT ストリームを閉じずに送る
                // (draft-ietf-webtrans-http3-16 Section 5.6)
                let _ = send.send(data).await;
            }
            ConnectCommand::Reset { error_code } => {
                // H3 エラーコードは VarInt 値域内のため new は常に成功する
                let error = s2n_quic::application::Error::new(error_code)
                    .expect("H3 error code fits in VarInt range");
                let _ = send.reset(error);
            }
        }
    }
}

/// カプセルを HTTP/3 DATA フレームのバイト列にエンコードする
///
/// `Capsule::encode_as_data_frame` は `Unknown` の生値が VarInt 範囲外の場合のみ
/// 失敗する。呼び出し側は失敗を黙って捨てるか、エラーとして伝播する。
fn encode_capsule_data_frame(capsule: &Capsule) -> Result<Vec<u8>, CapsuleEncodeError> {
    let mut buf = Vec::new();
    capsule.encode_as_data_frame(&mut buf)?;
    Ok(buf)
}

/// 送信待ちの WebTransport フロー制御カプセルを CONNECT ストリームへ送出する
///
/// sans-I/O 層が生成した `WT_MAX_STREAMS` / `WT_MAX_DATA` /
/// `WT_STREAMS_BLOCKED` / `WT_DATA_BLOCKED` を取り出し、それぞれを
/// HTTP/3 DATA フレームに包んで送信タスクへ渡す。
///
/// セッション確立直後に呼ぶことで、ピアがカプセルベースのフロー制御を要求して
/// いる場合の初期クレジットを通知する (Safari 26.4 互換。
/// draft-ietf-webtrans-http3-14 Section 5)。ピアの消費に応じた更新は
/// アプリが `consume_data` を呼んだ後に呼ぶ。
/// (draft-ietf-webtrans-http3-16 Section 5.6)
pub(crate) fn flush_flow_control_capsules<S>(
    state: &mut S,
    session_id: u64,
    connect_tx: &mpsc::UnboundedSender<ConnectCommand>,
) where
    S: crate::internal::connection_state::WtFlowControl + ?Sized,
{
    for capsule in state.take_wt_flow_control_capsules(session_id) {
        // CONNECT ストリーム上のカプセルは HTTP/3 DATA フレームとして送出する
        // (RFC 9297 Section 3.1 / RFC 9114 Section 7.2.1)
        //
        // `take_wt_flow_control_capsules` が返すのはフロー制御系の variant のみで、
        // 値は `VarInt` 型のためエンコードは失敗しない。失敗するのは `Unknown` の
        // 生値が VarInt 範囲外の場合だけなので、その場合は黙って捨てる。
        let Ok(buf) = encode_capsule_data_frame(&capsule) else {
            continue;
        };
        let _ = connect_tx.send(ConnectCommand::Send {
            data: Bytes::from(buf),
        });
    }
}

/// WebTransport セッション
///
/// フロー制御の状態操作をロール非依存に行うため、接続状態の型 `S` で総称化する。
/// `S` は `ClientConnectionState` または `ServerConnectionState`。
pub struct WtSession<S = crate::internal::connection_state::ClientConnectionState> {
    /// セッション ID
    session_id: u64,
    /// 双方向ストリームアクセプター
    bidi_acceptor: BidirectionalStreamAcceptor,
    /// 接続ハンドル
    handle: s2n_quic::connection::Handle,
    /// CONNECT ストリーム送信タスクへの指示チャネル
    ///
    /// 送信タスクはこのチャネルが閉じると終了するため JoinHandle は保持しない
    connect_tx: mpsc::UnboundedSender<ConnectCommand>,
    /// WT 単方向ストリーム受信チャネル
    uni_rx: mpsc::Receiver<WtRecvStream>,
    /// WebTransport イベント受信チャネル
    /// (現時点で発火し得るのは `SessionClosed` / `SessionDraining` / `BufferedStreamRejected`)
    event_rx: mpsc::Receiver<WebTransportEvent>,
    /// CONNECT ストリーム受信タスク (`WtSession` ドロップ時に abort する)
    ///
    /// タスクは CONNECT ストリームの受信データを sans-I/O 層へ流し、
    /// WT_CLOSE_SESSION カプセルの検知 / CONNECT ストリームの FIN / RESET_STREAM を
    /// `WebTransportEvent::SessionClosed` に変換して `event_rx` に届ける。
    recv_task: JoinHandle<()>,
    /// 接続状態 (送信側フロー制御の計上とカプセル生成に使う)
    ///
    /// 受信タスクと共有する。await を跨いだロック保持はしない。
    state: Arc<StdMutex<S>>,
}

/// `WtSession` の構築に必要な要素
///
/// 引数が 8 個になると可読性が落ちるため 1 つの構造体にまとめる。
pub(crate) struct WtSessionParts<S> {
    /// セッション ID
    pub(crate) session_id: u64,
    /// 双方向ストリームアクセプター
    pub(crate) bidi_acceptor: BidirectionalStreamAcceptor,
    /// 接続ハンドル
    pub(crate) handle: s2n_quic::connection::Handle,
    /// CONNECT ストリーム送信タスクへの指示チャネル
    pub(crate) connect_tx: mpsc::UnboundedSender<ConnectCommand>,
    /// WT 単方向ストリーム受信チャネル
    pub(crate) uni_rx: mpsc::Receiver<WtRecvStream>,
    /// WebTransport イベント受信チャネル
    pub(crate) event_rx: mpsc::Receiver<WebTransportEvent>,
    /// CONNECT ストリーム受信タスク
    pub(crate) recv_task: JoinHandle<()>,
    /// 接続状態
    pub(crate) state: Arc<StdMutex<S>>,
}

impl<S> WtSession<S>
where
    S: crate::internal::connection_state::WtFlowControl,
{
    /// 新しいセッションを作成する
    pub(crate) fn new(parts: WtSessionParts<S>) -> Self {
        Self {
            session_id: parts.session_id,
            bidi_acceptor: parts.bidi_acceptor,
            handle: parts.handle,
            connect_tx: parts.connect_tx,
            uni_rx: parts.uni_rx,
            event_rx: parts.event_rx,
            recv_task: parts.recv_task,
            state: parts.state,
        }
    }

    /// セッション ID を取得する
    pub fn session_id(&self) -> u64 {
        self.session_id
    }

    /// WebTransport イベントを受信する
    ///
    /// 現時点で届き得るイベントは以下の 3 種類:
    /// - `SessionClosed`: CONNECT ストリーム上で WT_CLOSE_SESSION カプセル受信 /
    ///   クリーンな FIN 受信 / RESET_STREAM 受信のいずれかを検知した
    /// - `SessionDraining`: WT_DRAIN_SESSION カプセル受信を検知した
    /// - `BufferedStreamRejected`: セッション確立時のバッファリング済みストリームが
    ///   フロー制御違反で拒否された
    ///
    /// `None` を返した場合、受信タスクが完全に終了しており以降イベントは来ない
    /// (セッションの終端。`SessionClosed` が届いた後に閉じるほか、CONNECT ストリームが
    /// 予期せず切断された場合や `WtSession` 自身がドロップされた場合にも `None` になる)。
    ///
    /// tokio-s2n-quic では現状 WT データストリームを sans-I/O 層に登録しないため、
    /// `SessionClosed { reset_streams }` は常に空 `Vec` になる。データストリームの
    /// 後始末はアプリ層で `WtBiStream::finish` 等を用いて行うこと
    /// (draft-ietf-webtrans-http3-16 Section 6)。
    pub async fn recv_event(&mut self) -> Option<WebTransportEvent> {
        self.event_rx.recv().await
    }

    /// 双方向ストリームを受け付ける
    ///
    /// 受信した QUIC 双方向ストリームから WT_STREAM ヘッダー (0x41 + session_id) を解析する
    /// (draft-ietf-webtrans-http3-16 Section 4.2)
    pub async fn accept_bi_stream(&mut self) -> crate::Result<WtBiStream> {
        let stream: s2n_quic::stream::BidirectionalStream = self
            .bidi_acceptor
            .accept_bidirectional_stream()
            .await
            .map_err(crate::Error::transport)?
            .ok_or(crate::Error::StreamClosed)?;

        let stream_id: u64 = stream.id();
        let (mut recv, send) = stream.split();

        // WT_STREAM ヘッダー (0x41 + session_id) をデコードする。
        //
        // `decode_bidirectional` はバッファ不足だけでなく不正フォーマット
        // (`InvalidFormat` / `InvalidSessionId`) でも `None` を返すため、`None` を
        // 待ち続けると非準拠ピアで無限ループしバッファが増え続ける。エラー種別を
        // 区別できる checked 版を使い、バッファ不足のみ待つ。
        // (draft-ietf-webtrans-http3-16 Section 4.3)
        let mut header_buf: Vec<u8> = Vec::new();
        let pending = loop {
            let data = recv
                .receive()
                .await
                .map_err(crate::Error::transport)?
                .ok_or(crate::Error::StreamClosed)?;
            header_buf.extend_from_slice(&data);
            match StreamHeader::decode_bidirectional_checked(&header_buf) {
                Ok((_, consumed)) => break header_buf[consumed..].to_vec(),
                Err(StreamHeaderDecodeError::BufferTooShort) => continue,
                Err(e) => {
                    // 不正なシグナル値 / session_id は当該ストリームを閉じる
                    // (draft-ietf-webtrans-http3-16 Section 4.3)
                    return Err(crate::Error::InvalidState(format!(
                        "invalid WebTransport bidi stream header: {e:?}"
                    )));
                }
            }
        };

        Ok(WtBiStream {
            stream_id,
            recv,
            send,
            pending,
        })
    }

    /// 新しい双方向ストリームを開く
    ///
    /// WT_STREAM ヘッダー (0x41 + session_id) を先頭に送信する
    /// (draft-ietf-webtrans-http3-16 Section 4.2)
    pub async fn open_bi_stream(&mut self) -> crate::Result<WtBiStream> {
        // ピアの WT_MAX_STREAMS を超えて開かない (draft-ietf-webtrans-http3-16 Section 5.6.2)。
        // 上限に達している場合は WT_STREAMS_BLOCKED をピアへ送ってから失敗させる。
        if !self.can_open_bi_stream() {
            self.bi_stream_opened();
            return Err(crate::Error::InvalidState(
                "WebTransport stream limit reached (WT_MAX_STREAMS)".to_string(),
            ));
        }
        let stream = self
            .handle
            .open_bidirectional_stream()
            .await
            .map_err(crate::Error::transport)?;
        let stream_id: u64 = stream.id();
        let (recv, mut send) = stream.split();

        // WT_STREAM ヘッダー (0x41 + session_id) を送信
        let mut header = Vec::new();
        // session_id は CONNECT ストリーム ID なので必ず client-initiated bidi
        StreamHeader::new(self.session_id)
            .expect("session_id must be a client-initiated bidi stream id")
            .encode_bidirectional(&mut header);
        send.send(Bytes::from(header))
            .await
            .map_err(crate::Error::transport)?;

        // 開設を計上する (draft-ietf-webtrans-http3-16 Section 5.6.2)
        self.bi_stream_opened();

        Ok(WtBiStream {
            stream_id,
            recv,
            send,
            pending: Vec::new(),
        })
    }

    /// 単方向ストリームを受け付ける
    ///
    /// uni_task が WT 単方向ストリーム (0x54) をルーティングしたものを返す
    pub async fn accept_uni_stream(&mut self) -> crate::Result<WtRecvStream> {
        self.uni_rx.recv().await.ok_or(crate::Error::StreamClosed)
    }

    /// 新しい単方向ストリームを開く
    ///
    /// WT 単方向ストリームヘッダー (0x54 + session_id) を先頭に送信する
    /// (draft-ietf-webtrans-http3-16 Section 4.3)
    pub async fn open_uni_stream(&mut self) -> crate::Result<WtSendStream> {
        // ピアの WT_MAX_STREAMS を超えて開かない (draft-ietf-webtrans-http3-16 Section 5.6.2)
        if !self.can_open_uni_stream() {
            self.uni_stream_opened();
            return Err(crate::Error::InvalidState(
                "WebTransport stream limit reached (WT_MAX_STREAMS)".to_string(),
            ));
        }
        let stream = self
            .handle
            .open_send_stream()
            .await
            .map_err(crate::Error::transport)?;
        let stream_id: u64 = stream.id();
        let mut send = stream;

        // WT 単方向ストリームヘッダー (0x54 + session_id) を送信
        let mut header = Vec::new();
        // session_id は CONNECT ストリーム ID なので必ず client-initiated bidi
        StreamHeader::new(self.session_id)
            .expect("session_id must be a client-initiated bidi stream id")
            .encode_unidirectional(&mut header);
        send.send(Bytes::from(header))
            .await
            .map_err(crate::Error::transport)?;

        // 開設を計上する (draft-ietf-webtrans-http3-16 Section 5.6.2)
        self.uni_stream_opened();

        Ok(WtSendStream { stream_id, send })
    }

    /// セッションのフロー制御が有効かどうかを取得する
    ///
    /// 両端がカプセルベースのフロー制御を宣言した場合のみ `true`
    /// (draft-ietf-webtrans-http3-16 Section 5.1)。
    pub fn flow_control_enabled(&self) -> bool {
        self.state
            .lock()
            .expect("mutex should not be poisoned")
            .wt_session_flow_control_enabled(self.session_id)
    }

    /// 双方向ストリームを開設してよいかどうかを取得する
    ///
    /// ピアが広告した `WT_MAX_STREAMS` の範囲内であれば `true`。
    /// `true` の場合、実際に開設した後に [`WtSession::bi_stream_opened`] を
    /// 呼んで計上すること。
    /// (draft-ietf-webtrans-http3-16 Section 5.6.2)
    pub fn can_open_bi_stream(&self) -> bool {
        self.state
            .lock()
            .expect("mutex should not be poisoned")
            .can_open_wt_bidi_stream(self.session_id)
    }

    /// 単方向ストリームを開設してよいかどうかを取得する
    /// (draft-ietf-webtrans-http3-16 Section 5.6.2)
    pub fn can_open_uni_stream(&self) -> bool {
        self.state
            .lock()
            .expect("mutex should not be poisoned")
            .can_open_wt_uni_stream(self.session_id)
    }

    /// データを送信してよいかどうかを取得する
    /// (draft-ietf-webtrans-http3-16 Section 5.6.4)
    pub fn can_send_data(&self, bytes: u64) -> bool {
        self.state
            .lock()
            .expect("mutex should not be poisoned")
            .can_send_wt_data(self.session_id, bytes)
    }

    /// 双方向ストリームを開設したことを計上する
    ///
    /// 上限に達している場合は `false` を返し `WT_STREAMS_BLOCKED` を生成して
    /// ピアへ送出する。
    /// (draft-ietf-webtrans-http3-16 Section 5.6.2)
    pub fn bi_stream_opened(&mut self) -> bool {
        self.record_stream_opened(true)
    }

    /// 単方向ストリームを開設したことを計上する
    /// (draft-ietf-webtrans-http3-16 Section 5.6.2)
    pub fn uni_stream_opened(&mut self) -> bool {
        self.record_stream_opened(false)
    }

    /// ストリーム開設を計上し、必要なら生成されたカプセルを送出する
    fn record_stream_opened(&mut self, bidirectional: bool) -> bool {
        let opened = self
            .state
            .lock()
            .expect("mutex should not be poisoned")
            .wt_stream_opened(self.session_id, bidirectional);
        if !opened {
            self.flush_flow_control_capsules();
        }
        opened
    }

    /// データを送信したことを計上する
    ///
    /// 上限に達している場合は `false` を返し `WT_DATA_BLOCKED` を生成して
    /// ピアへ送出する。
    /// (draft-ietf-webtrans-http3-16 Section 5.6.4)
    pub fn data_sent(&mut self, bytes: u64) -> bool {
        let sent = self
            .state
            .lock()
            .expect("mutex should not be poisoned")
            .wt_data_sent(self.session_id, bytes);
        if !sent {
            self.flush_flow_control_capsules();
        }
        sent
    }

    /// 受信データを消費したことを通知する
    ///
    /// 受信ウィンドウが半分を下回っていれば `WT_MAX_DATA` カプセルが生成され、
    /// CONNECT ストリームへ送出される。
    /// (draft-ietf-webtrans-http3-16 Section 5.6.4)
    pub fn consume_data(&mut self, bytes: u64) {
        self.state
            .lock()
            .expect("mutex should not be poisoned")
            .wt_data_consumed(self.session_id, bytes);
        self.flush_flow_control_capsules();
    }

    /// 送信待ちのフロー制御カプセルを CONNECT ストリームへ送出する
    fn flush_flow_control_capsules(&mut self) {
        flush_flow_control_capsules(
            &mut *self.state.lock().expect("mutex should not be poisoned"),
            self.session_id,
            &self.connect_tx,
        );
    }

    /// セッションをクローズする
    ///
    /// WT_CLOSE_SESSION カプセルを H3 DATA フレームに包んで CONNECT ストリームに送信し、
    /// 直後に FIN を送出する
    /// (draft-ietf-webtrans-http3-16 Section 6: WT_CLOSE_SESSION を送信したエンドポイントは
    /// CONNECT ストリームに即座に FIN を送らなければならない)
    pub async fn close(&mut self, code: u32, reason: &str) -> crate::Result<()> {
        let capsule = Capsule::CloseSession {
            error_code: code,
            message: reason.to_string(),
        };
        // CONNECT ストリーム上のカプセルは HTTP/3 DATA フレーム (0x00 + varint 長 + ペイロード)
        // として送出する必要がある (RFC 9297 Section 3.1 / RFC 9114 Section 7.2.1)。
        let mut buf = Vec::new();
        capsule
            .encode_as_data_frame(&mut buf)
            .map_err(|e| crate::Error::InvalidState(format!("capsule encode failed: {e}")))?;
        // 送信タスクへカプセル送出と FIN を依頼し、完了を待つ
        let (done_tx, done_rx) = oneshot::channel();
        self.connect_tx
            .send(ConnectCommand::SendAndFinish {
                data: Bytes::from(buf),
                done: done_tx,
            })
            .map_err(|_| crate::Error::StreamClosed)?;
        done_rx.await.map_err(|_| crate::Error::StreamClosed)?
    }
}

impl<S> Drop for WtSession<S> {
    fn drop(&mut self) {
        // 送信タスクへ FIN を依頼する。
        //
        // s2n-quic の `SendStream::drop` にも FIN 送出の暗黙挙動があるため実質同じ結果に
        // なるが、明示呼び出しにより「WtSession の drop でクリーンクローズを相手に届ける」
        // という意図をコードで表明する
        // (draft-ietf-webtrans-http3-16 Section 6: FIN のみでのクリーンクローズは
        // WT_CLOSE_SESSION(error_code=0, message="") と等価)。
        let _ = self.connect_tx.send(ConnectCommand::Finish);
        // 受信タスクに abort を通知する (実際の termination は runtime 依存で遅延しうる)。
        // `abort` を送っても直ちにタスクが停止する保証はないが、ハンドルが drop されて
        // 参照が失われた後は間もなく runtime が回収する。
        self.recv_task.abort();
    }
}

/// アプリ向け `WtSession::recv_event` に転送する WebTransport イベントかどうかを判定する
///
/// 転送対象: CONNECT ストリーム経由で sans-I/O 層が現在発火し得るセッションレベルの
/// 通知イベント (`SessionClosed` / `SessionDraining` / `BufferedStreamRejected`)。
///
/// 除外対象:
/// - `SessionEstablished`: ハンドシェイクループが `SessionEstablished` (クライアント) /
///   `establish_wt_session_server` (サーバー) で確立判定するため転送不要
/// - `BidiStreamOpen` / `BidiStreamData` / `BidiStreamEnd`: `accept_bi_stream` /
///   `WtBiStream::recv` で扱う
/// - `UniStreamOpen` / `UniStreamData` / `UniStreamEnd`: `accept_uni_stream` /
///   `WtRecvStream::recv` で扱う
/// - `Datagram` / `StreamReset` / `StreamStopSending` / `Capsule`: tokio-s2n-quic が
///   これらの生成源となる sans-I/O 呼び出し (DATAGRAM フィード、WT データストリーム
///   RESET_STREAM / STOP_SENDING 経路、フロー制御カプセルの取り出し) を wire していない
///   ため現時点で発火しない。将来 wire する対応が入る際に本フィルタへ追加する。
pub(crate) fn is_forwardable_wt_event(event: &WebTransportEvent) -> bool {
    matches!(
        event,
        WebTransportEvent::SessionClosed { .. }
            | WebTransportEvent::SessionDraining { .. }
            | WebTransportEvent::BufferedStreamRejected { .. }
            | WebTransportEvent::Capsule { .. }
    )
}

/// アプリへ SessionClosed が届かない Err パスを埋めるための合成イベント
///
/// 受信タスクが sans-I/O 層の Err に遭遇し `drain_events` からも `SessionClosed` を
/// 取り出せなかった場合の最終フォールバック。以下の値でクリーンクローズ相当のセマンティクスを
/// 与える (draft-ietf-webtrans-http3-16 Section 6):
/// - `error_code`: `WT_SESSION_GONE` (sans-I/O 側 `terminate_wt_session` の既定と同じ)
/// - `close_error_code`: 0 (WT_CLOSE_SESSION 未受信)
/// - `close_message`: 空文字列
/// - `reset_streams`: 空 (tokio-s2n-quic は WT データストリームを sans-I/O に登録しない)
pub(crate) fn synthesized_session_closed(session_id: u64) -> WebTransportEvent {
    WebTransportEvent::SessionClosed {
        session_id,
        reset_streams: Vec::new(),
        error_code: WtErrorCode::SessionGone as u64,
        close_error_code: 0,
        close_message: String::new(),
    }
}

/// WebTransport 双方向ストリーム
pub struct WtBiStream {
    /// ストリーム ID
    stream_id: u64,
    /// 受信ストリーム
    recv: ReceiveStream,
    /// 送信ストリーム
    send: SendStream,
    /// ヘッダー解析後の残留データ
    pending: Vec<u8>,
}

impl WtBiStream {
    /// データを送信する
    pub async fn send(&mut self, data: &[u8]) -> crate::Result<()> {
        self.send
            .send(Bytes::copy_from_slice(data))
            .await
            .map_err(crate::Error::transport)
    }

    /// データを受信する
    ///
    /// ヘッダー解析後の残留データがある場合はそれを先に返す
    pub async fn recv(&mut self) -> crate::Result<Vec<u8>> {
        if !self.pending.is_empty() {
            return Ok(std::mem::take(&mut self.pending));
        }
        let received: Result<Option<Bytes>, _> = self.recv.receive().await;
        match received {
            Ok(Some(data)) => Ok(data.to_vec()),
            Ok(None) => Err(crate::Error::StreamClosed),
            Err(e) => Err(crate::Error::transport(e)),
        }
    }

    /// ストリームを終了する
    pub fn finish(&mut self) -> crate::Result<()> {
        self.send.finish().map_err(crate::Error::transport)
    }

    /// ストリーム ID を取得する
    pub fn stream_id(&self) -> u64 {
        self.stream_id
    }
}

/// WebTransport 送信ストリーム
pub struct WtSendStream {
    /// ストリーム ID
    stream_id: u64,
    /// 送信ストリーム
    send: SendStream,
}

impl WtSendStream {
    /// データを送信する
    pub async fn send(&mut self, data: &[u8]) -> crate::Result<()> {
        self.send
            .send(Bytes::copy_from_slice(data))
            .await
            .map_err(crate::Error::transport)
    }

    /// ストリームを終了する
    pub fn finish(&mut self) -> crate::Result<()> {
        self.send.finish().map_err(crate::Error::transport)
    }

    /// ストリーム ID を取得する
    pub fn stream_id(&self) -> u64 {
        self.stream_id
    }
}

/// WebTransport 受信ストリーム
pub struct WtRecvStream {
    /// ストリーム ID
    stream_id: u64,
    /// 受信ストリーム
    recv: ReceiveStream,
    /// ヘッダー解析後の残留データ
    pending: Vec<u8>,
}

impl WtRecvStream {
    /// 新しい受信ストリームを作成する
    pub(crate) fn new(stream_id: u64, recv: ReceiveStream, pending: Vec<u8>) -> Self {
        Self {
            stream_id,
            recv,
            pending,
        }
    }

    /// データを受信する
    ///
    /// ヘッダー解析後の残留データがある場合はそれを先に返す
    pub async fn recv(&mut self) -> crate::Result<Vec<u8>> {
        if !self.pending.is_empty() {
            return Ok(std::mem::take(&mut self.pending));
        }
        let received: Result<Option<Bytes>, _> = self.recv.receive().await;
        match received {
            Ok(Some(data)) => Ok(data.to_vec()),
            Ok(None) => Err(crate::Error::StreamClosed),
            Err(e) => Err(crate::Error::transport(e)),
        }
    }

    /// ストリーム ID を取得する
    pub fn stream_id(&self) -> u64 {
        self.stream_id
    }
}
