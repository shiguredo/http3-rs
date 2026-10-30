//! WebTransport セッションとストリーム型

use std::sync::{Arc, Mutex as StdMutex};

use bytes::Bytes;
use s2n_quic::connection::BidirectionalStreamAcceptor;
use s2n_quic::stream::{ReceiveStream, SendStream};
use shiguredo_http3::WebTransportEvent;
use shiguredo_http3::webtransport::Capsule;
use shiguredo_http3::webtransport::CapsuleValidationError;
use shiguredo_http3::webtransport::error::ErrorCode as WtErrorCode;
use shiguredo_http3::webtransport::stream::{
    ClassifiedUniStream, StreamHeader, StreamHeaderDecodeError, classify_uni_stream_checked,
};
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use super::flow_control::WtSendFlowControl;

/// 送信側フロー制御の共有状態
///
/// `WtSession` (アプリタスク) がストリーム開設・データ送信で更新し、CONNECT ストリーム
/// 受信タスクがフロー制御カプセルで更新するため、両者から参照できるよう
/// `Arc<StdMutex<...>>` で共有する。
///
/// チャネルで単一所有者に寄せる案も検討したが、受信タスクは CONNECT ストリームの
/// 受信待ちでブロックしており、ストリーム開設の可否は開設前に同期的に確定させる
/// 必要がある。ロック区間は数命令で await を跨がないため Mutex を選択する。
pub(crate) type SharedSendFlowControl = Arc<StdMutex<WtSendFlowControl>>;

/// 共有状態を参照してストリーム開設を試行する
///
/// 上限超過時は [`crate::Error::FlowControlBlocked`] を返す。
pub(crate) fn try_open_stream(
    send_fc: &SharedSendFlowControl,
    bidirectional: bool,
) -> crate::Result<()> {
    send_fc
        .lock()
        .expect("mutex should not be poisoned")
        .try_open_stream(bidirectional)
}

/// 共有状態を参照してデータ送信を試行する
///
/// 上限超過時は [`crate::Error::FlowControlBlocked`] を返す。
fn try_send_data(send_fc: &SharedSendFlowControl, bytes: u64) -> crate::Result<()> {
    send_fc
        .lock()
        .expect("mutex should not be poisoned")
        .try_send_data(bytes)
}

/// ピアから受信したフロー制御カプセルを送信側フロー制御状態へ反映する
///
/// `Err` は WT_FLOW_CONTROL_ERROR でセッションを閉じるべき違反を表す
/// (draft-ietf-webtrans-http3-16 Section 5.6.2, 5.6.4)。呼び出し側は
/// [`flow_control_error_capsule`] を送信してセッションを終了すること。
pub(crate) fn apply_flow_control_capsule(
    send_fc: &SharedSendFlowControl,
    capsule: &Capsule,
) -> Result<(), CapsuleValidationError> {
    send_fc
        .lock()
        .expect("mutex should not be poisoned")
        .process_capsule(capsule)
}

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
            ConnectCommand::Reset { error_code } => {
                // H3 エラーコードは VarInt 値域内のため new は常に成功する
                let error = s2n_quic::application::Error::new(error_code)
                    .expect("H3 error code fits in VarInt range");
                let _ = send.reset(error);
            }
        }
    }
}

/// `H3_ID_ERROR` の QUIC アプリケーションエラーコードを返す
///
/// `H3_ID_ERROR` は 0x108 で、QUIC の 62 ビット VarInt 値域
/// (RFC 9000 Section 16) に収まる。
fn id_error_application_error() -> s2n_quic::application::Error {
    s2n_quic::application::Error::new(shiguredo_http3::ErrorCode::IdError.code())
        .expect("H3_ID_ERROR fits in 62-bit VarInt range (RFC 9000 Section 16)")
}

/// 接続を H3_ID_ERROR で閉じる
///
/// client-initiated bidirectional stream ID に対応しない session ID を受信した
/// 端点は H3_ID_ERROR で接続を閉じなければならない (MUST)
/// (draft-ietf-webtrans-http3-16 Section 4)。
pub(crate) fn close_connection_with_id_error(handle: &s2n_quic::connection::Handle) {
    handle.close(id_error_application_error());
}

/// WT 単方向ストリームヘッダーの解析結果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UniStreamHeader {
    /// WebTransport 単方向ストリーム (session ID が本セッションと一致する)
    WebTransport {
        /// ストリームタイプとセッション ID の後ろのデータ開始オフセット
        data_offset: usize,
    },
    /// WebTransport 以外の HTTP/3 単方向ストリーム
    Http3,
    /// session ID が client-initiated bidirectional stream ID ではない
    MalformedSessionId,
    /// session ID は形式として正しいが、本セッションのものではない
    ForeignSession,
}

/// WT 単方向ストリームヘッダーを解析する
///
/// `classify_uni_stream_checked` に session ID の一致検証を加えたもの。
/// ストリームタイプと session ID の varint が未完の場合は、追加データを待つことを
/// 表す `None` を返す。
///
/// 形式不正 (`MalformedSessionId`) は H3_ID_ERROR で接続を閉じる MUST がある
/// (draft-ietf-webtrans-http3-16 Section 4)。
///
/// `expected_session_id` が `Some` の場合は session ID の一致も検証する。
/// `None` は session ID が未確定 (サーバーが CONNECT ストリームを受理する前) の
/// 場合で、形式検証のみを行う。
///
/// `ForeignSession` は、本クレートが接続上の単方向ストリームアクセプターを
/// 単一の `WtSession` で保持しており他セッションへ配送できないことを表す
/// (本クレートの API では 1 接続 1 WebTransport セッションのみを扱う)。
pub(crate) fn classify_uni_stream_header(
    buf: &[u8],
    expected_session_id: Option<u64>,
) -> Option<UniStreamHeader> {
    match classify_uni_stream_checked(buf) {
        Ok(ClassifiedUniStream::WebTransport {
            session_id: received,
            data_offset,
        }) => match expected_session_id {
            Some(session_id) if received != session_id => Some(UniStreamHeader::ForeignSession),
            _ => Some(UniStreamHeader::WebTransport { data_offset }),
        },
        Ok(ClassifiedUniStream::Http3 { .. }) => Some(UniStreamHeader::Http3),
        Err(StreamHeaderDecodeError::BufferTooShort) => None,
        // 現状の `classify_uni_stream_checked` は InvalidFormat を返さない
        // (0x54 以外は Http3 として分類する) が、将来の変更に備えて
        // HTTP/3 ストリームとして扱い sans-I/O 層の検証に委ねる
        Err(StreamHeaderDecodeError::InvalidFormat) => Some(UniStreamHeader::Http3),
        Err(
            StreamHeaderDecodeError::InvalidSessionId
            | StreamHeaderDecodeError::SessionIdOutOfRange,
        ) => Some(UniStreamHeader::MalformedSessionId),
    }
}

/// WT 双方向ストリームヘッダーの解析結果
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BiStreamHeader {
    /// ヘッダー確定。`pending` はセッション ID の後ろに続くアプリケーションデータ
    Complete {
        /// ヘッダー解析後の残留データ
        pending: Vec<u8>,
    },
    /// Signal Value (0x41) がなく WebTransport 双方向ストリームではない
    NotWebTransport,
    /// session ID が client-initiated bidirectional stream ID ではない
    MalformedSessionId,
    /// session ID は形式として正しいが、本セッションのものではない
    ForeignSession,
}

/// WT 双方向ストリームヘッダーを解析する
///
/// `StreamHeader::decode_bidirectional_checked` に session ID の一致検証を加えたもの。
/// ヘッダーの varint が未完の場合は、追加データを待つことを表す `None` を返す。
///
/// 形式不正 (`MalformedSessionId`) は H3_ID_ERROR で接続を閉じる MUST がある
/// (draft-ietf-webtrans-http3-16 Section 4)。`NotWebTransport` は 0x41 以外の
/// 先頭バイトを持つストリーム (HTTP/3 リクエストストリーム等) を表す。
pub(crate) fn classify_bi_stream_header(buf: &[u8], session_id: u64) -> Option<BiStreamHeader> {
    match StreamHeader::decode_bidirectional_checked(buf) {
        Ok((header, consumed)) => {
            if header.session_id() == session_id {
                Some(BiStreamHeader::Complete {
                    pending: buf[consumed..].to_vec(),
                })
            } else {
                Some(BiStreamHeader::ForeignSession)
            }
        }
        Err(StreamHeaderDecodeError::BufferTooShort) => None,
        Err(StreamHeaderDecodeError::InvalidFormat) => Some(BiStreamHeader::NotWebTransport),
        Err(
            StreamHeaderDecodeError::InvalidSessionId
            | StreamHeaderDecodeError::SessionIdOutOfRange,
        ) => Some(BiStreamHeader::MalformedSessionId),
    }
}

/// WT_FLOW_CONTROL_ERROR を WS_CLOSE_SESSION の 32 ビットアプリケーション
/// エラーコードとして返す
///
/// WT_FLOW_CONTROL_ERROR は 0x045d4487 で 32 ビットに収まる
/// (draft-ietf-webtrans-http3-16 Section 9)。
fn flow_control_error_code() -> u32 {
    u32::try_from(WtErrorCode::FlowControlError as u64)
        .expect("WT_FLOW_CONTROL_ERROR fits in 32-bit application error code")
}

/// WT_FLOW_CONTROL_ERROR を示す WT_CLOSE_SESSION カプセルを作る
///
/// 単調性違反 (前回値以下) やストリーム数上限超過の検出時に送信する
/// (draft-ietf-webtrans-http3-16 Section 5.6.2, 5.6.4)。
pub(crate) fn flow_control_error_capsule() -> Capsule {
    Capsule::CloseSession {
        error_code: flow_control_error_code(),
        message: String::new(),
    }
}

/// カプセルを DATA フレームとして CONNECT ストリームへ送出し、FIN を送る
///
/// 送信完了は待たない (呼び出し側は受信タスクや `Drop` 経路で待てないため)。
pub(crate) fn send_capsule_and_finish(
    connect_tx: &mpsc::UnboundedSender<ConnectCommand>,
    capsule: Capsule,
) {
    // CONNECT ストリーム上のカプセルは HTTP/3 DATA フレームとして送出する
    // (RFC 9297 Section 3.1 / RFC 9114 Section 7.2.1)
    let mut buf = Vec::new();
    capsule.encode_as_data_frame(&mut buf);
    // WT_CLOSE_SESSION 送信後は即座に FIN を送る
    // (draft-ietf-webtrans-http3-16 Section 6)
    let (done_tx, _done_rx) = oneshot::channel();
    let _ = connect_tx.send(ConnectCommand::SendAndFinish {
        data: Bytes::from(buf),
        done: done_tx,
    });
}

/// WT_FLOW_CONTROL_ERROR でセッションを終了する
///
/// CONNECT ストリームへ WT_CLOSE_SESSION(WT_FLOW_CONTROL_ERROR) を送出し、
/// 直後に FIN を送る (draft-ietf-webtrans-http3-16 Section 6)。
/// フロー制御カプセルの単調性違反を検出した受信タスクが使用する
/// (draft-ietf-webtrans-http3-16 Section 5.6.2, 5.6.4)。
pub(crate) fn terminate_with_flow_control_error(
    connect_tx: &mpsc::UnboundedSender<ConnectCommand>,
) {
    send_capsule_and_finish(connect_tx, flow_control_error_capsule());
}

/// WT_FLOW_CONTROL_ERROR でセッションが終了したことをアプリへ通知する合成イベント
///
/// フロー制御違反は sans-I/O 層ではなく統合層が検出するため、
/// `WebTransportEvent::SessionClosed` を合成してアプリへ終端を通知する。
pub(crate) fn flow_control_error_session_closed(session_id: u64) -> WebTransportEvent {
    WebTransportEvent::SessionClosed {
        session_id,
        // 本クレートは WT データストリームを sans-I/O 層に登録しないため空
        reset_streams: Vec::new(),
        error_code: WtErrorCode::FlowControlError as u64,
        close_error_code: flow_control_error_code(),
        close_message: String::new(),
    }
}

/// WebTransport セッション
pub struct WtSession {
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
    /// (現時点で発火し得るのは `SessionClosed` / `SessionDraining` / `BufferedStreamRejected`。
    /// フロー制御違反で統合層が終了させた場合は合成 `SessionClosed` も届く)
    event_rx: mpsc::Receiver<WebTransportEvent>,
    /// 送信側フロー制御の共有状態
    ///
    /// CONNECT ストリーム受信タスクがフロー制御カプセルで更新する
    send_fc: SharedSendFlowControl,
    /// CONNECT ストリーム受信タスク (`WtSession` ドロップ時に abort する)
    ///
    /// タスクは CONNECT ストリームの受信データを sans-I/O 層へ流し、
    /// WT_CLOSE_SESSION カプセルの検知 / CONNECT ストリームの FIN / RESET_STREAM を
    /// `WebTransportEvent::SessionClosed` に変換して `event_rx` に届ける。
    recv_task: JoinHandle<()>,
}

impl WtSession {
    /// 新しいセッションを作成する
    #[expect(
        clippy::too_many_arguments,
        reason = "クライアント / サーバーの確立処理が保持する状態をそのまま引き継ぐため"
    )]
    pub(crate) fn new(
        session_id: u64,
        bidi_acceptor: BidirectionalStreamAcceptor,
        handle: s2n_quic::connection::Handle,
        connect_tx: mpsc::UnboundedSender<ConnectCommand>,
        uni_rx: mpsc::Receiver<WtRecvStream>,
        event_rx: mpsc::Receiver<WebTransportEvent>,
        send_fc: SharedSendFlowControl,
        recv_task: JoinHandle<()>,
    ) -> Self {
        Self {
            session_id,
            bidi_acceptor,
            handle,
            connect_tx,
            uni_rx,
            event_rx,
            send_fc,
            recv_task,
        }
    }

    /// セッション ID を取得する
    pub fn session_id(&self) -> u64 {
        self.session_id
    }

    /// WebTransport イベントを受信する
    ///
    /// 現時点で届き得るイベントは以下の 4 種類:
    /// - `SessionClosed`: CONNECT ストリーム上で WT_CLOSE_SESSION カプセル受信 /
    ///   クリーンな FIN 受信 / RESET_STREAM 受信のいずれかを検知した
    /// - `SessionDraining`: WT_DRAIN_SESSION カプセル受信を検知した
    /// - `BufferedStreamRejected`: セッション確立時のバッファリング済みストリームが
    ///   フロー制御違反で拒否された
    /// - `SessionClosed` (合成): 受信したフロー制御カプセルが単調性違反だったため
    ///   統合層が WT_FLOW_CONTROL_ERROR でセッションを終了させた
    ///   (draft-ietf-webtrans-http3-16 Section 5.6.2, 5.6.4)
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
    /// (draft-ietf-webtrans-http3-16 Section 4.3)。
    ///
    /// session ID が本セッションと一致しない場合は接続を H3_ID_ERROR で閉じる
    /// (draft-ietf-webtrans-http3-16 Section 4)。0x41 以外の先頭バイトを持つ
    /// ストリーム (HTTP/3 リクエストストリーム等) は本 API では処理できないため
    /// 接続は閉じずに `Error::InvalidState` を返す。
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
        // ヘッダー長は可変のため、確定するまで受信を繰り返す。
        let mut header_buf: Vec<u8> = Vec::new();
        let pending = loop {
            let data = recv
                .receive()
                .await
                .map_err(crate::Error::transport)?
                .ok_or(crate::Error::StreamClosed)?;
            header_buf.extend_from_slice(&data);
            match classify_bi_stream_header(&header_buf, self.session_id) {
                None => continue,
                Some(BiStreamHeader::Complete { pending }) => break pending,
                Some(BiStreamHeader::NotWebTransport) => {
                    // 0x41 以外の先頭バイトは WT ストリームではない。本 API は
                    // HTTP/3 リクエストストリームを扱わないため、接続は維持したまま
                    // ストリームをドロップする (STOP_SENDING が送出される)
                    return Err(crate::Error::InvalidState(
                        "received bidirectional stream is not a WebTransport stream".to_string(),
                    ));
                }
                Some(BiStreamHeader::MalformedSessionId | BiStreamHeader::ForeignSession) => {
                    // 形式不正は H3_ID_ERROR で接続を閉じる MUST
                    // (draft-ietf-webtrans-http3-16 Section 4)。
                    // 本セッション以外の session ID も、本クレートでは接続上の
                    // 双方向ストリームアクセプターを単一の WtSession が保持するため
                    // 他セッションへ配送できず、接続状態を維持したまま処理を続けられない
                    close_connection_with_id_error(&self.handle);
                    return Err(crate::Error::Http3(
                        shiguredo_http3::Error::ConnectionError(
                            shiguredo_http3::ErrorCode::IdError,
                        ),
                    ));
                }
            }
        };

        Ok(WtBiStream {
            stream_id,
            recv,
            send,
            pending,
            send_fc: Arc::clone(&self.send_fc),
        })
    }

    /// 新しい双方向ストリームを開く
    ///
    /// WT_STREAM ヘッダー (0x41 + session_id) を先頭に送信する
    /// (draft-ietf-webtrans-http3-16 Section 4.3)。
    ///
    /// ピアが広告した WT_MAX_STREAMS の上限に達している場合は
    /// `Error::FlowControlBlocked` を返し、ストリームを開かない
    /// (draft-ietf-webtrans-http3-16 Section 5.6.2)。
    pub async fn open_bi_stream(&mut self) -> crate::Result<WtBiStream> {
        // 上限超過の開設はピアに WT_FLOW_CONTROL_ERROR を発生させるため事前に検証する
        try_open_stream(&self.send_fc, true)?;

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

        Ok(WtBiStream {
            stream_id,
            recv,
            send,
            pending: Vec::new(),
            send_fc: Arc::clone(&self.send_fc),
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
    /// (draft-ietf-webtrans-http3-16 Section 4.2)。
    ///
    /// ピアが広告した WT_MAX_STREAMS の上限に達している場合は
    /// `Error::FlowControlBlocked` を返し、ストリームを開かない
    /// (draft-ietf-webtrans-http3-16 Section 5.6.2)。
    pub async fn open_uni_stream(&mut self) -> crate::Result<WtSendStream> {
        // 上限超過の開設はピアに WT_FLOW_CONTROL_ERROR を発生させるため事前に検証する
        try_open_stream(&self.send_fc, false)?;

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

        Ok(WtSendStream {
            stream_id,
            send,
            send_fc: Arc::clone(&self.send_fc),
        })
    }

    /// セッションをクローズする
    ///
    /// WT_CLOSE_SESSION カプセルを H3 DATA フレームに包んで CONNECT ストリームに送信し、
    /// 直後に FIN を送出する
    /// (draft-ietf-webtrans-http3-16 Section 6: WT_CLOSE_SESSION を送信したエンドポイントは
    /// CONNECT ストリームに即座に FIN を送らなければならない)。
    ///
    /// `reason` が 1024 バイト (WT_CLOSE_SESSION の Application Error Message の上限)
    /// を超える場合は、UTF-8 文字境界で 1024 バイトに切り詰められる
    /// (draft-ietf-webtrans-http3-16 Section 6)。切り詰めは sans-I/O 層の
    /// `Capsule::encode` が行うため、本メソッドでは切り詰めない。
    pub async fn close(&mut self, code: u32, reason: &str) -> crate::Result<()> {
        let capsule = Capsule::CloseSession {
            error_code: code,
            message: reason.to_string(),
        };
        // CONNECT ストリーム上のカプセルは HTTP/3 DATA フレーム (0x00 + varint 長 + ペイロード)
        // として送出する必要がある (RFC 9297 Section 3.1 / RFC 9114 Section 7.2.1)。
        let mut buf = Vec::new();
        capsule.encode_as_data_frame(&mut buf);
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

impl Drop for WtSession {
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
/// - `Capsule`: CONNECT ストリーム受信タスクが送信側フロー制御
///   ([`apply_flow_control_capsule`]) へ反映するため転送しない。反映の結果
///   単調性違反を検出した場合は合成 `SessionClosed` を別途転送する
/// - `Datagram` / `StreamReset` / `StreamStopSending`: tokio-s2n-quic が
///   これらの生成源となる sans-I/O 呼び出し (DATAGRAM フィード、WT データストリーム
///   RESET_STREAM / STOP_SENDING 経路) を wire していないため現時点で発火しない。
///   将来 wire する対応が入る際に本フィルタへ追加する。
pub(crate) fn is_forwardable_wt_event(event: &WebTransportEvent) -> bool {
    matches!(
        event,
        WebTransportEvent::SessionClosed { .. }
            | WebTransportEvent::SessionDraining { .. }
            | WebTransportEvent::BufferedStreamRejected { .. }
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
    /// 送信側フロー制御 (WT_MAX_DATA の検証用)
    send_fc: SharedSendFlowControl,
}

impl WtBiStream {
    /// データを送信する
    ///
    /// ピアが広告した WT_MAX_DATA の上限に達している場合は
    /// `Error::FlowControlBlocked` を返し、送信しない
    /// (draft-ietf-webtrans-http3-16 Section 5.6.4)。
    pub async fn send(&mut self, data: &[u8]) -> crate::Result<()> {
        try_send_data(&self.send_fc, data.len() as u64)?;
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
    /// 送信側フロー制御 (WT_MAX_DATA の検証用)
    send_fc: SharedSendFlowControl,
}

impl WtSendStream {
    /// データを送信する
    ///
    /// ピアが広告した WT_MAX_DATA の上限に達している場合は
    /// `Error::FlowControlBlocked` を返し、送信しない
    /// (draft-ietf-webtrans-http3-16 Section 5.6.4)。
    pub async fn send(&mut self, data: &[u8]) -> crate::Result<()> {
        try_send_data(&self.send_fc, data.len() as u64)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// テスト用: ストリームヘッダーをエンコードする
    fn encode_bi_header(session_id: u64) -> Vec<u8> {
        let mut buf = Vec::new();
        StreamHeader::new(session_id)
            .expect("テスト用の session ID が client-initiated bidi ID であること")
            .encode_bidirectional(&mut buf);
        buf
    }

    /// テスト用: 単方向ストリームヘッダーをエンコードする
    fn encode_uni_header(session_id: u64) -> Vec<u8> {
        let mut buf = Vec::new();
        StreamHeader::new(session_id)
            .expect("テスト用の session ID が client-initiated bidi ID であること")
            .encode_unidirectional(&mut buf);
        buf
    }

    #[test]
    fn test_id_error_application_error_is_h3_id_error() {
        // H3_ID_ERROR (0x108) が QUIC のアプリケーションエラーコードとして設定されること
        // (draft-ietf-webtrans-http3-16 Section 4)
        assert_eq!(
            *id_error_application_error(),
            0x108,
            "H3_ID_ERROR のコードが 0x108 であること"
        );
    }

    #[test]
    fn test_classify_uni_stream_header_webtransport() {
        // session ID が一致する WT 単方向ストリームは WebTransport と判定されること
        let buf = encode_uni_header(4);
        assert_eq!(
            classify_uni_stream_header(&buf, Some(4)),
            Some(UniStreamHeader::WebTransport {
                data_offset: buf.len()
            })
        );
    }

    #[test]
    fn test_classify_uni_stream_header_foreign_session() {
        // 形式は正しいが本セッションと異なる session ID は ForeignSession と判定されること
        let buf = encode_uni_header(8);
        assert_eq!(
            classify_uni_stream_header(&buf, Some(4)),
            Some(UniStreamHeader::ForeignSession)
        );
    }

    #[test]
    fn test_classify_uni_stream_header_malformed_session_id() {
        // session ID が client-initiated bidi ID (4 の倍数) でない場合は
        // MalformedSessionId と判定されること (H3_ID_ERROR で接続を閉じる)
        // (draft-ietf-webtrans-http3-16 Section 4)
        // ストリームタイプ 0x54 は 2 バイト varint (0x40 0x54)、session ID 1 は 1 バイト
        let buf = [0x40, 0x54, 0x01];
        assert_eq!(
            classify_uni_stream_header(&buf, Some(4)),
            Some(UniStreamHeader::MalformedSessionId)
        );
    }

    #[test]
    fn test_classify_uni_stream_header_http3() {
        // 0x54 以外のストリームタイプは HTTP/3 ストリームと判定されること
        assert_eq!(
            classify_uni_stream_header(&[0x00], Some(4)),
            Some(UniStreamHeader::Http3)
        );
    }

    #[test]
    fn test_classify_uni_stream_header_incomplete() {
        // ストリームタイプの varint が未完の場合は Incomplete と判定されること
        assert_eq!(
            classify_uni_stream_header(&[0x40], Some(4)),
            None,
            "varint が未完の場合は None (追加データ待ち) になること"
        );
    }

    #[test]
    fn test_classify_bi_stream_header_complete() {
        // session ID が一致する WT 双方向ストリームは Complete と判定され、
        // ヘッダー以降のデータが pending として返ること
        let mut buf = encode_bi_header(4);
        buf.extend_from_slice(b"hello");
        assert_eq!(
            classify_bi_stream_header(&buf, 4),
            Some(BiStreamHeader::Complete {
                pending: b"hello".to_vec()
            })
        );
    }

    #[test]
    fn test_classify_bi_stream_header_incomplete() {
        // ヘッダーが未完の場合は Incomplete と判定されること
        assert_eq!(classify_bi_stream_header(&[], 4), None);
        // Signal Value (0x41) のみで session ID が未完の場合も None
        assert_eq!(classify_bi_stream_header(&[0x41], 4), None);
    }

    #[test]
    fn test_classify_bi_stream_header_not_webtransport() {
        // 0x41 以外の先頭バイトは WT ストリームではないと判定されること
        // (HTTP/3 リクエストストリームの HEADERS フレームタイプ 0x01)
        assert_eq!(
            classify_bi_stream_header(&[0x01], 4),
            Some(BiStreamHeader::NotWebTransport)
        );
    }

    #[test]
    fn test_classify_bi_stream_header_malformed_session_id() {
        // session ID が client-initiated bidi ID でない場合は MalformedSessionId
        // (draft-ietf-webtrans-http3-16 Section 4)
        // Signal Value 0x41 は 2 バイト varint (0x40 0x41)、session ID 2 は 1 バイト
        assert_eq!(
            classify_bi_stream_header(&[0x40, 0x41, 0x02], 4),
            Some(BiStreamHeader::MalformedSessionId)
        );
    }

    #[test]
    fn test_classify_bi_stream_header_foreign_session() {
        // 形式は正しいが本セッションと異なる session ID は ForeignSession
        assert_eq!(
            classify_bi_stream_header(&[0x40, 0x41, 0x08], 4),
            Some(BiStreamHeader::ForeignSession)
        );
    }

    #[test]
    fn test_flow_control_error_capsule() {
        // WT_FLOW_CONTROL_ERROR の WT_CLOSE_SESSION カプセルが生成されること
        // (draft-ietf-webtrans-http3-16 Section 5.6.2, 5.6.4)
        match flow_control_error_capsule() {
            Capsule::CloseSession {
                error_code,
                message,
            } => {
                assert_eq!(
                    error_code, 0x045d4487,
                    "WT_FLOW_CONTROL_ERROR のコードが設定されること"
                );
                assert_eq!(message, "", "メッセージは空であること");
            }
            other => panic!("WT_CLOSE_SESSION 以外が生成された: {other:?}"),
        }
    }

    #[test]
    fn test_flow_control_error_session_closed() {
        // フロー制御違反で終了したことを通知する合成 SessionClosed の各フィールド
        match flow_control_error_session_closed(4) {
            WebTransportEvent::SessionClosed {
                session_id,
                reset_streams,
                error_code,
                close_error_code,
                close_message,
            } => {
                assert_eq!(session_id, 4, "セッション ID が設定されること");
                assert!(reset_streams.is_empty(), "リセット対象は空であること");
                assert_eq!(
                    error_code, 0x045d4487,
                    "RESET_STREAM 用のエラーコードが WT_FLOW_CONTROL_ERROR であること"
                );
                assert_eq!(
                    close_error_code, 0x045d4487,
                    "WT_CLOSE_SESSION のアプリケーションエラーコードが設定されること"
                );
                assert_eq!(close_message, "", "メッセージは空であること");
            }
            other => panic!("SessionClosed 以外が生成された: {other:?}"),
        }
    }
}
