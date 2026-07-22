//! HTTP/3 リクエストストリーム (RFC 9114 Section 6.1)
//!
//! 双方向ストリームで HTTP リクエスト/レスポンスを送受信。

use crate::error::{Error, ErrorCode};
use crate::frame::{self, DataPayload, Frame, HeadersPayload};
use crate::qpack::Header;

use super::{RecvBuffer, SendBuffer, StreamState};

/// リクエストストリーム送信状態
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestSendState {
    /// 初期状態
    Initial,
    /// 1xx 中間レスポンス送信済み (DATA/トレーラー禁止、次の HEADERS 待ち)
    InterimResponseSent,
    /// HEADERS 送信済み (最終レスポンスまたはリクエスト)
    HeadersSent,
    /// DATA 送信中
    SendingBody,
    /// トレーラー送信済み (以降の HEADERS / DATA は不正)
    TrailersSent,
    /// 完了
    Complete,
}

/// リクエストストリーム受信状態
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestRecvState {
    /// HEADERS 待ち
    WaitingHeaders,
    /// DATA 受信中
    ReceivingBody,
    /// トレーラー受信済み (これ以降の HEADERS / DATA は不正)
    TrailersReceived,
    /// 完了
    Complete,
}

/// リクエストストリーム
#[derive(Debug)]
pub struct RequestStream {
    /// ストリーム ID
    stream_id: u64,
    /// 送信バッファ
    send_buf: SendBuffer,
    /// 受信バッファ
    recv_buf: RecvBuffer,
    /// 送信状態
    send_state: RequestSendState,
    /// 受信状態
    recv_state: RequestRecvState,
    /// ストリーム状態
    state: StreamState,
    /// 受信済みヘッダー
    recv_headers: Vec<Header>,
    /// 受信済みボディ
    recv_body: Vec<u8>,
    /// HEAD リクエストかどうか (クライアントが HEAD を送信した場合 true)
    ///
    /// Content-Length 検証でレスポンスの body チェックをスキップするために使用する。
    is_head_request: bool,
    /// クライアントが plain CONNECT リクエストを送信したかどうか
    ///
    /// 2xx レスポンス受信時に is_connect を設定するために使用する。
    /// (RFC 9114 Section 4.4)
    is_connect_request: bool,
    /// 通常の CONNECT ストリームかどうか (Extended CONNECT を除く)
    ///
    /// CONNECT 完了後は DATA のみ許可し、HEADERS は H3_FRAME_UNEXPECTED とする。
    /// (RFC 9114 Section 4.4)
    is_connect: bool,
    /// QPACK ブロック中かどうか (nghttp3 の NGHTTP3_STREAM_FLAG_QPACK_DECODE_BLOCKED 相当)
    ///
    /// ブロック中はフレーム解析を停止し、受信データをバッファに保持する。
    qpack_blocked: bool,
    /// QPACK ブロック中の Required Insert Count
    ///
    /// `Connection::blocked_by_ricnt` での順序付けに使用する。
    qpack_ricnt: u64,
    /// QPACK ブロック中のエンコード済みヘッダー
    ///
    /// HEADERS/Trailers フレームのペイロード。ブロック解除時にデコードする。
    qpack_blocked_header: Option<(Vec<u8>, bool)>,
}

impl RequestStream {
    /// 新しいリクエストストリームを作成
    pub fn new(stream_id: u64) -> Self {
        Self {
            stream_id,
            send_buf: SendBuffer::new(),
            recv_buf: RecvBuffer::new(),
            send_state: RequestSendState::Initial,
            recv_state: RequestRecvState::WaitingHeaders,
            state: StreamState::Open,
            recv_headers: Vec::new(),
            recv_body: Vec::new(),
            is_head_request: false,
            is_connect_request: false,
            is_connect: false,
            qpack_blocked: false,
            qpack_ricnt: 0,
            qpack_blocked_header: None,
        }
    }

    /// ストリーム ID を取得
    pub fn stream_id(&self) -> u64 {
        self.stream_id
    }

    /// ストリーム状態を取得
    pub fn state(&self) -> StreamState {
        self.state
    }

    /// ストリーム状態への可変参照を取得
    pub fn state_mut(&mut self) -> &mut StreamState {
        &mut self.state
    }

    /// エンコード済みヘッダーを送信
    ///
    /// Connection が QPACK エンコードを行い、エンコード済みデータを渡す。
    /// `is_interim` が true の場合は 1xx 中間レスポンスとして扱い、
    /// DATA/トレーラーの送信を禁止する (RFC 9114 Section 4.1)。
    pub fn send_encoded_headers(
        &mut self,
        encoded: &[u8],
        fin: bool,
        is_interim: bool,
    ) -> Result<(), Error> {
        if !self.state.can_send() {
            return Err(Error::StreamClosed(self.stream_id));
        }

        // 送信状態による HEADERS 送信可否の検証 (RFC 9114 Section 4.1, 4.4)
        match self.send_state {
            // 初期状態: 最初の HEADERS (リクエストまたはレスポンス)
            RequestSendState::Initial => {}
            // 1xx 送信後: 次の HEADERS (最終レスポンスまたは次の 1xx)
            RequestSendState::InterimResponseSent => {}
            // HEADERS / DATA 送信後: トレーラーとして送信可能
            // ただし CONNECT 完了後は HEADERS 禁止 (RFC 9114 Section 4.4)
            RequestSendState::HeadersSent | RequestSendState::SendingBody => {
                if self.is_connect {
                    return Err(Error::StreamError(ErrorCode::FrameUnexpected));
                }
                if is_interim {
                    // 最終レスポンス送信後に 1xx は不正
                    return Err(Error::StreamError(ErrorCode::FrameUnexpected));
                }
            }
            // トレーラー送信後 / 完了後: HEADERS は不正 (RFC 9114 Section 4.1)
            RequestSendState::TrailersSent | RequestSendState::Complete => {
                return Err(Error::StreamError(ErrorCode::FrameUnexpected));
            }
        }

        // 中間レスポンスで FIN は不正: 最終レスポンスが必要 (RFC 9114 Section 4.1)
        // バッファに追加する前にチェックする
        if is_interim && fin {
            return Err(Error::StreamError(ErrorCode::MessageError));
        }

        // HEADERS フレーム
        let frame = Frame::Headers(HeadersPayload::new(encoded.to_vec()));
        // HEADERS は内部で構築するペイロード長 (QPACK エンコード結果) しか扱わず、
        // 必ず VarInt 範囲内に収まる。将来型レベルに昇格予定。
        let frame_len =
            frame::encoded_frame_len(&frame).expect("HEADERS frame length fits in VarInt");
        let mut frame_buf = vec![0u8; frame_len];
        let written =
            frame::encode_frame(&mut frame_buf, &frame).expect("encoded_frame_len validated above");
        debug_assert_eq!(written, frame_len);

        self.send_buf.push(&frame_buf);

        // 状態遷移
        match self.send_state {
            RequestSendState::Initial | RequestSendState::InterimResponseSent => {
                if is_interim {
                    self.send_state = RequestSendState::InterimResponseSent;
                } else {
                    self.send_state = RequestSendState::HeadersSent;
                }
            }
            RequestSendState::HeadersSent | RequestSendState::SendingBody => {
                // トレーラー
                self.send_state = RequestSendState::TrailersSent;
            }
            _ => unreachable!(),
        }

        if fin {
            self.send_buf.set_fin();
            self.state.close_local();
            self.send_state = RequestSendState::Complete;
        }

        Ok(())
    }

    /// ボディを送信
    pub fn send_body(&mut self, data: &[u8], fin: bool) -> Result<(), Error> {
        if !self.state.can_send() {
            return Err(Error::StreamClosed(self.stream_id));
        }

        // DATA 送信可否の検証 (RFC 9114 Section 4.1, 4.4)
        // HEADERS 前、1xx 中間レスポンス後、トレーラー後の DATA は不正
        match self.send_state {
            RequestSendState::HeadersSent | RequestSendState::SendingBody => {}
            _ => {
                return Err(Error::StreamError(ErrorCode::FrameUnexpected));
            }
        }

        if !data.is_empty() {
            // DATA フレーム
            let frame = Frame::Data(DataPayload::new(data.to_vec()));
            // DATA フレームのペイロード長は呼び出し側スライス長と一致するため、必ず
            // VarInt 範囲内 (`usize` の上限値 < 2^62 - 1)。
            let frame_len =
                frame::encoded_frame_len(&frame).expect("DATA frame length fits in VarInt");
            let mut frame_buf = vec![0u8; frame_len];
            let written = frame::encode_frame(&mut frame_buf, &frame)
                .expect("encoded_frame_len validated above");
            debug_assert_eq!(written, frame_len);

            self.send_buf.push(&frame_buf);
            self.send_state = RequestSendState::SendingBody;
        }

        if fin {
            self.send_buf.set_fin();
            self.state.close_local();
            self.send_state = RequestSendState::Complete;
        }

        Ok(())
    }

    /// データを受信
    pub fn receive(&mut self, data: &[u8], fin: bool) {
        self.recv_buf.push(data);
        if fin {
            self.recv_buf.set_fin();
        }
    }

    /// 受信フレームを処理 (生データを返す)
    ///
    /// Connection が QPACK デコードを行うため、エンコードされたヘッダーをそのまま返す。
    pub fn process_raw(&mut self) -> Result<Option<RawReceivedData>, Error> {
        loop {
            let data = self.recv_buf.peek();
            if data.is_empty() {
                if self.recv_buf.is_fin() && self.recv_state != RequestRecvState::Complete {
                    // HEADERS フレームを 1 件も受信しないまま FIN は malformed
                    // (RFC 9114 Section 4.1, 4.1.2)
                    if self.recv_state == RequestRecvState::WaitingHeaders {
                        return Err(Error::StreamError(ErrorCode::MessageError));
                    }
                    // plain CONNECT でも DATA なし FIN は許容する (RFC 9114 Section 4.4)
                    // TCP 側が即座に接続を閉じた場合に自然に発生し得る
                    self.state.close_remote();
                    self.recv_state = RequestRecvState::Complete;
                    return Ok(Some(RawReceivedData::StreamEnd));
                }
                return Ok(None);
            }

            // フレームヘッダーをチェック
            let header = match frame::decode_frame_header(data) {
                Ok(h) => h,
                Err(crate::error::FrameDecodeError::BufferTooShort) => {
                    // FIN 受信済みならフレームが切断されている (RFC 9114 Section 7.1)
                    if self.recv_buf.is_fin() {
                        return Err(Error::ConnectionError(ErrorCode::FrameError));
                    }
                    return Ok(None);
                }
                // HTTP/2 専用フレームは接続エラー (RFC 9114 Section 7.2.8)
                Err(crate::error::FrameDecodeError::Http2Frame(_)) => {
                    return Err(Error::ConnectionError(ErrorCode::FrameUnexpected));
                }
                Err(crate::error::FrameDecodeError::InvalidLength) => {
                    return Err(Error::ConnectionError(ErrorCode::FrameError));
                }
                Err(e) => return Err(Error::FrameDecode(e)),
            };

            // フレーム全体を受信できているか
            // total_len が None なら 32bit プラットフォームで usize に収まらない
            // → H3_FRAME_ERROR (RFC 9114 Section 7.1)
            let Some(total_len) = header.total_len() else {
                return Err(Error::ConnectionError(ErrorCode::FrameError));
            };
            if data.len() < total_len {
                // FIN 受信済みならフレームが切断されている (RFC 9114 Section 7.1)
                if self.recv_buf.is_fin() {
                    return Err(Error::ConnectionError(ErrorCode::FrameError));
                }
                return Ok(None);
            }

            // SETTINGS はリクエストストリームでは内容に関わらず接続エラー
            // (RFC 9114 Section 7.2.4: ペイロードデコード前に判定する)
            if header.frame_type().get() == 0x04 {
                return Err(Error::ConnectionError(ErrorCode::FrameUnexpected));
            }

            // フレームをデコード
            let (frame, consumed) = frame::decode_frame(data).map_err(|e| match e {
                // サーバープッシュ関連フレームは接続エラー
                // PUSH_PROMISE は本来 request stream で受信可能だが、
                // サーバープッシュ非対応のため H3_FRAME_UNEXPECTED で拒否する
                // (詳細は frame/decoder.rs のコメントを参照)
                crate::error::FrameDecodeError::ServerPushNotSupported(_) => {
                    Error::ConnectionError(ErrorCode::FrameUnexpected)
                }
                other => Error::FrameDecode(other),
            })?;
            self.recv_buf.consume(consumed);

            match frame {
                Frame::Headers(payload) => {
                    // トレーラー後の HEADERS は不正 (RFC 9114 Section 4.1)
                    if self.recv_state == RequestRecvState::TrailersReceived {
                        return Err(Error::ConnectionError(ErrorCode::FrameUnexpected));
                    }
                    if self.recv_state == RequestRecvState::ReceivingBody {
                        // CONNECT 完了後は HEADERS 禁止 (RFC 9114 Section 4.4)
                        if self.is_connect {
                            return Err(Error::ConnectionError(ErrorCode::FrameUnexpected));
                        }
                        // 2 回目の HEADERS はトレーラー (RFC 9114 Section 4.1)
                        self.recv_state = RequestRecvState::TrailersReceived;
                        return Ok(Some(RawReceivedData::Trailers(
                            payload.into_encoded_field_section(),
                        )));
                    } else {
                        // 最初の HEADERS
                        self.recv_state = RequestRecvState::ReceivingBody;
                    }
                    return Ok(Some(RawReceivedData::Headers(
                        payload.into_encoded_field_section(),
                    )));
                }
                Frame::Data(payload) => {
                    // HEADERS 前の DATA は接続エラー (RFC 9114 Section 4.1)
                    // トレーラー後の DATA も接続エラー (RFC 9114 Section 4.1)
                    if self.recv_state == RequestRecvState::WaitingHeaders
                        || self.recv_state == RequestRecvState::TrailersReceived
                    {
                        return Err(Error::ConnectionError(ErrorCode::FrameUnexpected));
                    }
                    let data = payload.into_data();
                    self.recv_body.extend_from_slice(&data);
                    return Ok(Some(RawReceivedData::Data(data)));
                }
                Frame::Unknown(_) => {
                    // RFC 9114 Section 9 / Section 7.2.8: 不明なフレームは無視する
                    // (Reserved Frame Types: 0x1f * N + 0x21)
                    // ループを継続して次のフレームを処理
                }
                // SETTINGS/GOAWAY/MAX_PUSH_ID はリクエストストリームで受信した場合は接続エラー
                // (RFC 9114 Section 7.2.4, 7.2.6, 7.2.7)
                Frame::Settings(_) | Frame::Goaway(_) | Frame::MaxPushId(_) => {
                    return Err(Error::ConnectionError(ErrorCode::FrameUnexpected));
                }
            }
        }
    }

    /// 1xx 中間レスポンスを受信したことを通知する (Connection から呼び出し)
    ///
    /// 1xx 受信後に次の HEADERS (最終レスポンス) を受け入れられるよう
    /// recv_state を WaitingHeaders に戻す。
    /// RFC 9114 Section 4.1: 1xx はボディとトレーラーを持たない。
    pub fn notify_informational(&mut self) {
        if self.recv_state == RequestRecvState::ReceivingBody {
            self.recv_state = RequestRecvState::WaitingHeaders;
        }
    }

    /// デコード済みヘッダーを設定 (Connection から呼び出し)
    pub fn set_recv_headers(&mut self, headers: Vec<Header>) {
        self.recv_headers = headers;
    }

    /// 送信データを取得
    pub fn get_send_data(&self) -> (&[u8], bool) {
        (
            self.send_buf.peek(),
            self.send_buf.is_fin() && !self.send_buf.has_pending(),
        )
    }

    /// 送信データを消費
    pub fn consume_send_data(&mut self, len: usize) {
        self.send_buf.consume(len);
    }

    /// FIN 送信完了をマークする
    pub fn mark_fin_sent(&mut self) {
        self.send_buf.mark_fin_sent();
    }

    /// 送信待ちデータがあるか (FIN-only も含む)
    pub fn has_pending_send(&self) -> bool {
        self.send_buf.has_pending()
    }

    /// 受信済みヘッダーを取得
    pub fn received_headers(&self) -> &[Header] {
        &self.recv_headers
    }

    /// 受信済みボディを取得
    pub fn received_body(&self) -> &[u8] {
        &self.recv_body
    }

    /// HEAD リクエストかどうかを設定 (Content-Length 検証用)
    pub fn set_is_head_request(&mut self, v: bool) {
        self.is_head_request = v;
    }

    /// HEAD リクエストかどうかを取得
    pub fn is_head_request(&self) -> bool {
        self.is_head_request
    }

    /// plain CONNECT リクエストを送信したことをマークする (クライアント専用)
    ///
    /// 2xx レスポンス受信時に is_connect を設定するために使用する。
    pub fn set_connect_request(&mut self) {
        self.is_connect_request = true;
    }

    /// plain CONNECT リクエストを送信したかどうかを取得する
    pub fn is_connect_request(&self) -> bool {
        self.is_connect_request
    }

    /// CONNECT ストリームとしてマークする (RFC 9114 Section 4.4)
    ///
    /// CONNECT 完了後は DATA のみ許可し、HEADERS (トレーラー) は H3_FRAME_UNEXPECTED とする。
    /// plain CONNECT および WebTransport CONNECT の両方で設定する。
    pub fn set_connect(&mut self) {
        self.is_connect = true;
    }

    /// QPACK ブロック中かどうかを取得する
    pub fn is_qpack_blocked(&self) -> bool {
        self.qpack_blocked
    }

    /// QPACK ブロック状態を設定する
    pub fn set_qpack_blocked(
        &mut self,
        blocked: bool,
        ricnt: u64,
        blocked_header: Option<(Vec<u8>, bool)>,
    ) {
        self.qpack_blocked = blocked;
        self.qpack_ricnt = ricnt;
        self.qpack_blocked_header = blocked_header;
    }

    /// QPACK ブロック中の Required Insert Count を取得する
    pub fn qpack_ricnt(&self) -> u64 {
        self.qpack_ricnt
    }

    /// QPACK ブロック中のエンコード済みヘッダーを取り出す
    pub fn take_qpack_blocked_header(&mut self) -> Option<(Vec<u8>, bool)> {
        self.qpack_blocked_header.take()
    }
}

/// 受信データの種類 (生データ)
///
/// QPACK デコード前のデータを返す。Connection が動的テーブルを使用してデコードする。
#[derive(Debug, Clone)]
pub enum RawReceivedData {
    /// エンコードされたフィールドセクション (ヘッダーセクション)
    Headers(Vec<u8>),
    /// エンコードされたトレーラーセクション (RFC 9114 Section 4.1)
    Trailers(Vec<u8>),
    /// ボディデータ
    Data(Vec<u8>),
    /// ストリーム終了
    StreamEnd,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::qpack::{Encoder as QpackEncoder, Header};

    #[test]
    fn test_request_stream_send_encoded_headers() {
        let mut stream = RequestStream::new(0);
        let headers = vec![
            Header::new(b":method", b"GET").expect("test must succeed"),
            Header::new(b":path", b"/").expect("test must succeed"),
            Header::new(b":scheme", b"https").expect("test must succeed"),
            Header::new(b":authority", b"example.com").expect("test must succeed"),
        ];

        // QPACK エンコード
        let mut encoder = QpackEncoder::new();
        let mut qpack_buf = vec![0u8; 4096];
        let qpack_len = encoder
            .encode(&mut qpack_buf, &headers, 0)
            .expect("test must succeed");
        qpack_buf.truncate(qpack_len);

        stream
            .send_encoded_headers(&qpack_buf, false, false)
            .expect("test must succeed");
        assert!(stream.has_pending_send());

        let (data, fin) = stream.get_send_data();
        assert!(!data.is_empty());
        assert!(!fin);
    }

    #[test]
    fn test_request_stream_send_body() {
        let mut stream = RequestStream::new(0);
        let headers = vec![Header::new(b":method", b"POST").expect("test must succeed")];

        // QPACK エンコード
        let mut encoder = QpackEncoder::new();
        let mut qpack_buf = vec![0u8; 4096];
        let qpack_len = encoder
            .encode(&mut qpack_buf, &headers, 0)
            .expect("test must succeed");
        qpack_buf.truncate(qpack_len);

        stream
            .send_encoded_headers(&qpack_buf, false, false)
            .expect("test must succeed");
        stream.send_body(b"hello", true).expect("test must succeed");

        let (data, _fin) = stream.get_send_data();
        assert!(!data.is_empty());
        // FIN は最後のデータ消費後に true になる
    }

    #[test]
    fn test_request_stream_http2_frame_is_connection_error() {
        let mut stream = RequestStream::new(0);
        // HTTP/2 PRIORITY フレーム (0x02) はリクエストストリームで接続エラー
        // (RFC 9114 Section 7.2.8)
        let data = [0x02, 0x05, 0x00, 0x00, 0x00, 0x00, 0x00];
        stream.receive(&data, false);
        let result = stream.process_raw();
        assert!(
            matches!(
                result,
                Err(Error::ConnectionError(ErrorCode::FrameUnexpected))
            ),
            "expected ConnectionError(FrameUnexpected), got {result:?}"
        );
    }

    #[test]
    fn test_request_stream_settings_frame_is_connection_error() {
        let mut stream = RequestStream::new(0);
        // SETTINGS フレーム (0x04) はリクエストストリームで接続エラー (RFC 9114 Section 7.2.4)
        let data = [0x04, 0x00];
        stream.receive(&data, false);
        let result = stream.process_raw();
        assert!(
            matches!(
                result,
                Err(Error::ConnectionError(ErrorCode::FrameUnexpected))
            ),
            "expected ConnectionError(FrameUnexpected), got {result:?}"
        );
    }

    #[test]
    fn test_request_stream_settings_with_invalid_payload_is_connection_error() {
        // 不正ペイロード (HTTP/2 専用設定 ID = 0x02) を持つ SETTINGS でも
        // ペイロード内容に関わらず ConnectionError(FrameUnexpected) を返す
        // (RFC 9114 Section 7.2.4)
        let mut stream = RequestStream::new(0);
        // SETTINGS フレーム: type=0x04, len=2, payload=[0x02, 0x01]
        // 0x02 は ENABLE_PUSH (HTTP/2 専用設定 ID)
        let data = [0x04, 0x02, 0x02, 0x01];
        stream.receive(&data, false);
        let result = stream.process_raw();
        assert!(
            matches!(
                result,
                Err(Error::ConnectionError(ErrorCode::FrameUnexpected))
            ),
            "expected ConnectionError(FrameUnexpected), got {result:?}"
        );
    }

    #[test]
    fn test_request_stream_goaway_frame_is_connection_error() {
        let mut stream = RequestStream::new(0);
        // GOAWAY フレーム (0x07) はリクエストストリームで接続エラー (RFC 9114 Section 7.2.6)
        let data = [0x07, 0x01, 0x00];
        stream.receive(&data, false);
        let result = stream.process_raw();
        assert!(
            matches!(
                result,
                Err(Error::ConnectionError(ErrorCode::FrameUnexpected))
            ),
            "expected ConnectionError(FrameUnexpected), got {result:?}"
        );
    }

    #[test]
    fn test_request_stream_fin_without_headers_is_message_error() {
        // HEADERS なしで FIN は malformed (RFC 9114 Section 4.1, 4.1.2)
        let mut stream = RequestStream::new(0);
        stream.receive(&[], true);
        let result = stream.process_raw();
        assert!(
            matches!(result, Err(Error::StreamError(ErrorCode::MessageError))),
            "expected StreamError(MessageError), got {result:?}"
        );
    }

    #[test]
    fn test_request_stream_fin_after_headers_is_stream_end() {
        // HEADERS 受信後の FIN は正常終了
        let mut stream = RequestStream::new(0);
        // HEADERS フレーム (QPACK: RIC=0, DeltaBase=0, Indexed :method GET)
        let data = [0x01, 0x03, 0x00, 0x00, 0xd1];
        stream.receive(&data, false);
        let _ = stream.process_raw().expect("test must succeed"); // Headers
        stream.receive(&[], true);
        let result = stream.process_raw().expect("test must succeed");
        assert!(
            matches!(result, Some(RawReceivedData::StreamEnd)),
            "expected StreamEnd, got {result:?}"
        );
    }

    #[test]
    fn test_request_stream_receive_raw() {
        let mut stream = RequestStream::new(0);

        // HEADERS フレーム (QPACK: RIC=0, DeltaBase=0, Indexed :method GET)
        let data = [0x01, 0x03, 0x00, 0x00, 0xd1];
        stream.receive(&data, false);

        let result = stream
            .process_raw()
            .expect("test must succeed")
            .expect("test must succeed");
        if let RawReceivedData::Headers(encoded) = result {
            // エンコードされたフィールドセクションが返される
            assert_eq!(encoded, vec![0x00, 0x00, 0xd1]);
        } else {
            panic!("expected Headers");
        }
    }

    #[test]
    fn test_request_stream_second_headers_is_trailers() {
        // ヘッダー → DATA → 2 回目 HEADERS はトレーラーとして Trailers バリアントを返す
        // (RFC 9114 Section 4.1)
        let mut stream = RequestStream::new(0);

        // 1 回目 HEADERS フレーム
        let headers_frame = [0x01, 0x03, 0x00, 0x00, 0xd1];
        stream.receive(&headers_frame, false);
        let result = stream
            .process_raw()
            .expect("test must succeed")
            .expect("test must succeed");
        assert!(
            matches!(result, RawReceivedData::Headers(_)),
            "expected Headers, got {result:?}"
        );

        // DATA フレーム
        let data_frame = [0x00, 0x05, b'h', b'e', b'l', b'l', b'o'];
        stream.receive(&data_frame, false);
        let result = stream
            .process_raw()
            .expect("test must succeed")
            .expect("test must succeed");
        assert!(
            matches!(result, RawReceivedData::Data(_)),
            "expected Data, got {result:?}"
        );

        // 2 回目 HEADERS フレーム (トレーラー)
        stream.receive(&headers_frame, false);
        let result = stream
            .process_raw()
            .expect("test must succeed")
            .expect("test must succeed");
        assert!(
            matches!(result, RawReceivedData::Trailers(_)),
            "expected Trailers, got {result:?}"
        );
    }

    #[test]
    fn test_request_stream_notify_informational_resets_state() {
        // notify_informational() で ReceivingBody → WaitingHeaders に戻る
        // (1xx 受信後に最終レスポンス HEADERS を受け入れられるようにする)
        let mut stream = RequestStream::new(0);

        // 1 回目 HEADERS (1xx として扱う)
        let headers_frame = [0x01, 0x03, 0x00, 0x00, 0xd1];
        stream.receive(&headers_frame, false);
        let result = stream
            .process_raw()
            .expect("test must succeed")
            .expect("test must succeed");
        assert!(matches!(result, RawReceivedData::Headers(_)));

        // 1xx 通知 (状態を WaitingHeaders に戻す)
        stream.notify_informational();

        // 2 回目 HEADERS は最終レスポンスとして Headers を返す (Trailers ではない)
        stream.receive(&headers_frame, false);
        let result = stream
            .process_raw()
            .expect("test must succeed")
            .expect("test must succeed");
        assert!(
            matches!(result, RawReceivedData::Headers(_)),
            "expected Headers for final response after 1xx, got {result:?}"
        );

        // DATA も正常に受信できる
        let data_frame = [0x00, 0x03, b'f', b'o', b'o'];
        stream.receive(&data_frame, false);
        let result = stream
            .process_raw()
            .expect("test must succeed")
            .expect("test must succeed");
        assert!(
            matches!(result, RawReceivedData::Data(_)),
            "expected Data after final response, got {result:?}"
        );
    }

    #[test]
    fn test_request_stream_headers_after_trailers_is_error() {
        // トレーラー受信後の HEADERS は接続エラー (RFC 9114 Section 4.1)
        let mut stream = RequestStream::new(0);

        // 1 回目 HEADERS
        let headers_frame = [0x01, 0x03, 0x00, 0x00, 0xd1];
        stream.receive(&headers_frame, false);
        let _ = stream
            .process_raw()
            .expect("test must succeed")
            .expect("test must succeed"); // Headers

        // 2 回目 HEADERS (トレーラー)
        stream.receive(&headers_frame, false);
        let _ = stream
            .process_raw()
            .expect("test must succeed")
            .expect("test must succeed"); // Trailers

        // 3 回目 HEADERS は接続エラー
        stream.receive(&headers_frame, false);
        let result = stream.process_raw();
        assert!(
            matches!(
                result,
                Err(Error::ConnectionError(ErrorCode::FrameUnexpected))
            ),
            "expected ConnectionError(FrameUnexpected), got {result:?}"
        );
    }
}
