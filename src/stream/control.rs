//! HTTP/3 制御ストリーム (RFC 9114 Section 6.2.1)
//!
//! 制御ストリームは SETTINGS, GOAWAY などの制御フレームの送受信に使用。

use crate::error::{Error, ErrorCode};
use crate::frame::{self, Frame, GoawayPayload, SettingsPayload};
use crate::settings::Settings;
use crate::varint::VarInt;
use crate::webtransport::stream::is_wt_stream_frame_type;

use super::{RecvBuffer, SendBuffer};

/// 制御ストリーム送信状態
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ControlSendState {
    /// SETTINGS 未送信
    Initial,
    /// SETTINGS 送信済み
    Ready,
}

/// 制御ストリーム受信状態
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ControlRecvState {
    /// ストリームタイプ待ち
    WaitingType,
    /// SETTINGS 待ち
    WaitingSettings,
    /// 準備完了
    Ready,
}

/// 制御ストリーム (送信側)
#[derive(Debug)]
pub(crate) struct ControlStreamSend {
    /// ストリーム ID
    stream_id: Option<u64>,
    /// 送信バッファ
    send_buf: SendBuffer,
    /// 送信状態
    send_state: ControlSendState,
}

impl ControlStreamSend {
    /// 新しい制御ストリーム (送信側) を作成
    pub fn new() -> Self {
        Self {
            stream_id: None,
            send_buf: SendBuffer::new(),
            send_state: ControlSendState::Initial,
        }
    }

    /// ストリーム ID を設定
    pub fn set_stream_id(&mut self, stream_id: u64) {
        self.stream_id = Some(stream_id);
    }

    /// ストリーム ID を取得
    pub fn stream_id(&self) -> Option<u64> {
        self.stream_id
    }

    /// SETTINGS フレームを送信キューに追加
    pub fn send_settings(&mut self, settings: &Settings) {
        if self.send_state != ControlSendState::Initial {
            return;
        }

        // ストリームタイプ (0x00)
        let mut buf = vec![0x00];

        // SETTINGS フレーム
        let payload = SettingsPayload::from_settings(settings);
        let frame = Frame::Settings(payload);
        // SETTINGS の id / value はアプリ層が `Settings` 経由で構築するため、
        // 現時点では VarInt 範囲内であることをアプリ層責務として扱う。
        // 将来 `Setting` enum に昇格して型レベルに移す。
        let frame_len =
            frame::encoded_frame_len(&frame).expect("SETTINGS frame fields fit in VarInt");
        buf.resize(1 + frame_len, 0);
        let written =
            frame::encode_frame(&mut buf[1..], &frame).expect("encoded_frame_len validated above");
        debug_assert_eq!(written, frame_len);

        self.send_buf.push(&buf);
        self.send_state = ControlSendState::Ready;
    }

    /// GOAWAY フレームを送信キューに追加
    ///
    /// 制御ストリームが Ready 状態でなければエラーを返す (RFC 9114 Section 7.2.6)。
    /// `id` の値域は [`VarInt`] 型レベルで保証される (RFC 9000 Section 16)。
    pub fn send_goaway(&mut self, id: VarInt) -> Result<(), Error> {
        if self.send_state != ControlSendState::Ready {
            return Err(crate::error::Error::ConnectionError(
                crate::error::ErrorCode::ClosedCriticalStream,
            ));
        }

        let frame = Frame::Goaway(GoawayPayload::new(id));
        let frame_len = frame::encoded_frame_len(&frame)
            .expect("GOAWAY id is VarInt typed, payload always fits");
        let mut buf = vec![0u8; frame_len];
        let written =
            frame::encode_frame(&mut buf, &frame).expect("encoded_frame_len validated above");
        debug_assert_eq!(written, frame_len);

        self.send_buf.push(&buf);
        Ok(())
    }

    /// 送信データを取得
    pub fn get_data(&self) -> &[u8] {
        self.send_buf.peek()
    }

    /// 送信データを消費
    pub fn consume_data(&mut self, len: usize) {
        self.send_buf.consume(len);
    }

    /// 送信待ちデータがあるか
    pub fn has_pending(&self) -> bool {
        self.send_buf.has_pending()
    }
}

impl Default for ControlStreamSend {
    fn default() -> Self {
        Self::new()
    }
}

/// 制御ストリーム (受信側)
#[derive(Debug)]
pub(crate) struct ControlStreamRecv {
    /// ストリーム ID
    stream_id: Option<u64>,
    /// 受信バッファ
    recv_buf: RecvBuffer,
    /// 受信状態
    recv_state: ControlRecvState,
}

impl ControlStreamRecv {
    /// 新しい制御ストリーム (受信側) を作成
    pub fn new() -> Self {
        Self {
            stream_id: None,
            recv_buf: RecvBuffer::new(),
            recv_state: ControlRecvState::WaitingType,
        }
    }

    /// ストリーム ID を設定
    pub fn set_stream_id(&mut self, stream_id: u64) {
        self.stream_id = Some(stream_id);
    }

    /// ストリーム ID を取得
    pub fn stream_id(&self) -> Option<u64> {
        self.stream_id
    }

    /// ストリームタイプを既にデコード済みとしてスキップ
    ///
    /// Connection 側で varint デコード済みの場合に WaitingSettings 状態へ遷移する。
    pub fn skip_stream_type(&mut self) {
        if self.recv_state == ControlRecvState::WaitingType {
            self.recv_state = ControlRecvState::WaitingSettings;
        }
    }

    /// データを受信
    pub fn receive(&mut self, data: &[u8]) {
        self.recv_buf.push(data);
    }

    /// 受信バッファに未処理データが残っているかを返す
    ///
    /// FIN 受信時にこれが true の場合、フレームが切り詰められていることを意味する
    /// (RFC 9114 Section 7.1: H3_FRAME_ERROR)
    pub fn has_pending_data(&self) -> bool {
        !self.recv_buf.peek().is_empty()
    }

    /// フレームを処理
    pub fn process(&mut self) -> Result<Option<Frame>, Error> {
        loop {
            let data = self.recv_buf.peek();
            if data.is_empty() {
                return Ok(None);
            }

            match self.recv_state {
                ControlRecvState::WaitingType => {
                    // ストリームタイプ (1バイト varint)
                    if data.is_empty() {
                        return Ok(None);
                    }
                    if data[0] != 0x00 {
                        return Err(Error::ConnectionError(ErrorCode::StreamCreationError));
                    }
                    self.recv_buf.consume(1);
                    self.recv_state = ControlRecvState::WaitingSettings;
                }
                ControlRecvState::WaitingSettings | ControlRecvState::Ready => {
                    // 制御ストリームの WT_STREAM (0x41) は length を持たないため、
                    // フレームタイプが確定した時点で判定する
                    // (draft-ietf-webtrans-http3-17 Section 4.3。将来のドラフトで
                    //  変更される可能性がある)。
                    // SETTINGS 受信前は RFC 9114 Section 6.2.1 の
                    // H3_MISSING_SETTINGS を優先する (最初のフレームが SETTINGS 以外
                    // なら接続エラー)。
                    if is_wt_stream_frame_type(data) {
                        let error_code = if self.recv_state == ControlRecvState::WaitingSettings {
                            ErrorCode::MissingSettings
                        } else {
                            ErrorCode::FrameUnexpected
                        };
                        return Err(Error::ConnectionError(error_code));
                    }

                    // フレームヘッダーをチェック
                    let header = match frame::decode_frame_header(data) {
                        Ok(h) => h,
                        Err(crate::error::FrameDecodeError::BufferTooShort) => return Ok(None),
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
                        return Ok(None);
                    }

                    // フレームをデコード
                    let (frame, consumed) = frame::decode_frame(data).map_err(|e| match e {
                        // サーバープッシュ関連フレームは接続エラー
                        // CANCEL_PUSH / MAX_PUSH_ID は本来 control stream で受信可能だが、
                        // サーバープッシュ非対応のため H3_FRAME_UNEXPECTED で拒否する
                        // (詳細は frame/decoder.rs のコメントを参照)
                        crate::error::FrameDecodeError::ServerPushNotSupported(_) => {
                            Error::ConnectionError(ErrorCode::FrameUnexpected)
                        }
                        // SETTINGS パラメータの構築時検査エラー (重複 / HTTP/2 専用 / 予約 / bool 値域外)
                        // は H3_SETTINGS_ERROR (RFC 9114 §7.2.4 / §7.2.4.1)
                        crate::error::FrameDecodeError::InvalidSetting(_) => {
                            Error::ConnectionError(ErrorCode::SettingsError)
                        }
                        // payload 途中切れはフレームエラー (RFC 9114 Section 7.1)
                        crate::error::FrameDecodeError::InvalidLength => {
                            Error::ConnectionError(ErrorCode::FrameError)
                        }
                        other => Error::FrameDecode(other),
                    })?;
                    self.recv_buf.consume(consumed);

                    // SETTINGS が最初のフレームである必要がある (RFC 9114 Section 6.2.1)
                    if self.recv_state == ControlRecvState::WaitingSettings {
                        if matches!(frame, Frame::Settings(_)) {
                            // SETTINGS の内容検証は ControlStreamRecv では行わず、
                            // 接続層 (Connection) 側の process_control_stream から
                            // Settings::from_payload で検証する (二重管理を避ける)
                            self.recv_state = ControlRecvState::Ready;
                        } else {
                            // SETTINGS 以外の最初のフレームは H3_MISSING_SETTINGS
                            // (RFC 9114 Section 6.2.1)。WT_STREAM (0x41) は
                            // フレームタイプ確定時点の判定で先に処理される。
                            return Err(Error::ConnectionError(ErrorCode::MissingSettings));
                        }
                    } else {
                        // Ready 状態でのフレーム検証
                        match &frame {
                            // 2 回目以降の SETTINGS は接続エラー (RFC 9114 Section 7.2.4)
                            Frame::Settings(_) => {
                                return Err(Error::ConnectionError(ErrorCode::FrameUnexpected));
                            }
                            // DATA/HEADERS は制御ストリームでは無効 (RFC 9114 Section 7.2)
                            Frame::Data(_) | Frame::Headers(_) => {
                                return Err(Error::ConnectionError(ErrorCode::FrameUnexpected));
                            }
                            _ => {}
                        }
                    }

                    return Ok(Some(frame));
                }
            }
        }
    }

    /// 準備完了かどうか
    #[cfg(test)]
    pub fn is_ready(&self) -> bool {
        self.recv_state == ControlRecvState::Ready
    }
}

impl Default for ControlStreamRecv {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_control_stream_send() {
        use crate::varint::VarInt;
        let mut stream = ControlStreamSend::new();
        let settings = Settings::new().max_field_section_size(VarInt::from_static(16384));

        stream.send_settings(&settings);
        assert!(stream.has_pending());

        let data = stream.get_data();
        assert!(!data.is_empty());
        assert_eq!(data[0], 0x00); // Control stream type
    }

    #[test]
    fn test_control_stream_recv() {
        let mut stream = ControlStreamRecv::new();

        // ストリームタイプ + SETTINGS フレーム
        // Type=0x00, Frame: type=0x04, len=0x00
        let data = [0x00, 0x04, 0x00];
        stream.receive(&data);

        let frame = stream
            .process()
            .expect("テスト用の処理に成功すること")
            .expect("テスト用の処理に成功すること");
        assert!(matches!(frame, Frame::Settings(_)));
        assert!(stream.is_ready());
    }

    #[test]
    fn test_control_stream_recv_duplicate_settings() {
        let mut stream = ControlStreamRecv::new();

        // ストリームタイプ + SETTINGS フレーム (1 回目)
        let data = [0x00, 0x04, 0x00];
        stream.receive(&data);
        stream
            .process()
            .expect("テスト用の処理に成功すること")
            .expect("テスト用の処理に成功すること");
        assert!(stream.is_ready());

        // 2 回目の SETTINGS フレームは接続エラー (RFC 9114 Section 7.2.4)
        let data = [0x04, 0x00];
        stream.receive(&data);
        let result = stream.process();
        assert!(
            matches!(
                result,
                Err(Error::ConnectionError(ErrorCode::FrameUnexpected))
            ),
            "H3_FRAME_UNEXPECTED になること: {result:?}"
        );
    }

    #[test]
    fn test_control_stream_recv_data_frame_is_error() {
        let mut stream = ControlStreamRecv::new();

        // ストリームタイプ + SETTINGS フレーム
        let data = [0x00, 0x04, 0x00];
        stream.receive(&data);
        stream
            .process()
            .expect("テスト用の処理に成功すること")
            .expect("テスト用の処理に成功すること");

        // DATA フレームは制御ストリームで接続エラー (RFC 9114 Section 7.2)
        let data = [0x00, 0x03, 0x01, 0x02, 0x03];
        stream.receive(&data);
        let result = stream.process();
        assert!(
            matches!(
                result,
                Err(Error::ConnectionError(ErrorCode::FrameUnexpected))
            ),
            "H3_FRAME_UNEXPECTED になること: {result:?}"
        );
    }

    #[test]
    fn test_control_stream_recv_http2_frame_is_error() {
        let mut stream = ControlStreamRecv::new();

        // ストリームタイプ + SETTINGS フレーム
        let data = [0x00, 0x04, 0x00];
        stream.receive(&data);
        stream
            .process()
            .expect("テスト用の処理に成功すること")
            .expect("テスト用の処理に成功すること");

        // HTTP/2 PING フレーム (0x06) は接続エラー (RFC 9114 Section 7.2.8)
        let data = [0x06, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        stream.receive(&data);
        let result = stream.process();
        assert!(
            matches!(
                result,
                Err(Error::ConnectionError(ErrorCode::FrameUnexpected))
            ),
            "H3_FRAME_UNEXPECTED になること: {result:?}"
        );
    }

    #[test]
    fn test_control_stream_recv_http2_settings_id() {
        let mut stream = ControlStreamRecv::new();

        // ストリームタイプ + HTTP/2 専用設定 ID を含む SETTINGS フレーム
        // ENABLE_PUSH (0x02) = 1 → H3_SETTINGS_ERROR (RFC 9114 Section 7.2.4)
        let data = [0x00, 0x04, 0x02, 0x02, 0x01];
        stream.receive(&data);
        let result = stream.process();
        assert!(
            matches!(
                result,
                Err(Error::ConnectionError(ErrorCode::SettingsError))
            ),
            "expected ConnectionError(SettingsError), got {result:?}"
        );
    }

    #[test]
    fn test_control_stream_recv_wt_stream_without_length_is_frame_unexpected() {
        // WT_STREAM は length を持たないため、フレームタイプの可変長整数だけで
        // 不正位置と判定できること (draft-ietf-webtrans-http3-17 Section 4.3)
        let mut stream = ControlStreamRecv::new();

        // ストリームタイプ + SETTINGS フレーム
        let data = [0x00, 0x04, 0x00];
        stream.receive(&data);
        stream
            .process()
            .expect("テスト用の処理に成功すること")
            .expect("テスト用の処理に成功すること");
        assert!(stream.is_ready());

        // length を伴わない WT_STREAM 型 (0x41) だけを投入する
        let data = [0x40, 0x41];
        stream.receive(&data);
        let result = stream.process();
        assert!(
            matches!(
                result,
                Err(Error::ConnectionError(ErrorCode::FrameUnexpected))
            ),
            "H3_FRAME_UNEXPECTED になること: {result:?}"
        );
    }

    #[test]
    fn test_control_stream_recv_wt_stream_split_varint_is_frame_unexpected() {
        // WT_STREAM 型の可変長整数が 2 チャンクに分割されて到着しても、
        // 型が確定した時点で不正位置と判定できること
        // (draft-ietf-webtrans-http3-17 Section 4.3)
        let mut stream = ControlStreamRecv::new();

        // ストリームタイプ + SETTINGS フレーム
        let data = [0x00, 0x04, 0x00];
        stream.receive(&data);
        stream
            .process()
            .expect("テスト用の処理に成功すること")
            .expect("テスト用の処理に成功すること");
        assert!(stream.is_ready());

        // type=0x41 の 1 バイト目だけでは型を確定できないため待機する
        let chunk1 = [0x40];
        stream.receive(&chunk1);
        let result = stream.process().expect("テスト用の処理に成功すること");
        assert!(result.is_none(), "型が未確定の間は待機すること: {result:?}");

        // 2 バイト目で型が 0x41 に確定するため、length の到着を待たずに接続エラー
        let chunk2 = [0x41];
        stream.receive(&chunk2);
        let result = stream.process();
        assert!(
            matches!(
                result,
                Err(Error::ConnectionError(ErrorCode::FrameUnexpected))
            ),
            "H3_FRAME_UNEXPECTED になること: {result:?}"
        );
    }

    #[test]
    fn test_control_stream_recv_wt_stream_before_settings_is_missing_settings() {
        let mut stream = ControlStreamRecv::new();

        // ストリームタイプ (0x00) を消費して WaitingSettings 状態にする
        let stream_type = [0x00];
        stream.receive(&stream_type);
        let result = stream.process().expect("テスト用の処理に成功すること");
        assert!(
            result.is_none(),
            "ストリームタイプを消費すること: {result:?}"
        );

        // SETTINGS 受信前 (WaitingSettings 状態) の WT_STREAM (0x41) は
        // H3_MISSING_SETTINGS になる (RFC 9114 Section 6.2.1 の SETTINGS 先頭必須が、
        // draft-ietf-webtrans-http3-17 Section 4.3 の不正位置の WT_STREAM に対する
        // H3_FRAME_UNEXPECTED より優先する)。WT_STREAM は length を持たないため、
        // フレームタイプの可変長整数だけで判定できる。
        for data in [
            // フレームタイプだけで後続バイトがない
            &[0x40u8, 0x41][..],
            // 後続の varint が 4 (session ID) でも length として待たない
            &[0x40, 0x41, 0x04][..],
        ] {
            let mut stream = ControlStreamRecv::new();
            stream.receive(&[0x00]);
            let result = stream.process().expect("テスト用の処理に成功すること");
            assert!(
                result.is_none(),
                "ストリームタイプを消費すること: {result:?}"
            );

            stream.receive(data);
            let result = stream.process();
            assert!(
                matches!(
                    result,
                    Err(Error::ConnectionError(ErrorCode::MissingSettings))
                ),
                "SETTINGS 前の WT_STREAM は H3_MISSING_SETTINGS になること: {result:?}"
            );
        }
    }

    #[test]
    fn test_control_stream_recv_wt_stream_split_varint_before_settings_is_missing_settings() {
        // SETTINGS 受信前に WT_STREAM 型の可変長整数が分割されて到着しても、
        // 型が確定した時点で H3_MISSING_SETTINGS になること
        // (RFC 9114 Section 6.2.1, draft-ietf-webtrans-http3-17 Section 4.3)
        let mut stream = ControlStreamRecv::new();
        stream.receive(&[0x00]);
        stream.process().expect("テスト用の処理に成功すること");

        // 1 バイト目だけでは型を確定できないため待機する
        stream.receive(&[0x40]);
        let result = stream.process().expect("テスト用の処理に成功すること");
        assert!(result.is_none(), "型が未確定の間は待機すること: {result:?}");

        // 2 バイト目で型が確定するため、length を待たずに接続エラーになる
        stream.receive(&[0x41]);
        let result = stream.process();
        assert!(
            matches!(
                result,
                Err(Error::ConnectionError(ErrorCode::MissingSettings))
            ),
            "型の確定で H3_MISSING_SETTINGS になること: {result:?}"
        );
    }

    #[test]
    fn test_control_stream_recv_unknown_frame_is_returned() {
        // 0x41 以外の未知フレームはデコーダーが Frame::Unknown として返す
        // (接続層はこれを無視する。RFC 9114 Section 9 / Section 7.2.8)
        let mut stream = ControlStreamRecv::new();

        // ストリームタイプ + SETTINGS フレーム
        let data = [0x00, 0x04, 0x00];
        stream.receive(&data);
        stream
            .process()
            .expect("テスト用の処理に成功すること")
            .expect("テスト用の処理に成功すること");
        assert!(stream.is_ready());

        // 予約フレーム (0x21) は未知フレームとして返される (RFC 9114 Section 7.2.8)
        let data = [0x21, 0x00];
        stream.receive(&data);
        let result = stream
            .process()
            .expect("テスト用の処理に成功すること")
            .expect("テスト用の処理に成功すること");
        match result {
            Frame::Unknown(unknown) => assert_eq!(unknown.frame_type().get(), 0x21),
            other => panic!("未知フレームが返ること: {other:?}"),
        }
    }
}
