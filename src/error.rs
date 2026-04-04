//! HTTP/3 エラー型
//!
//! RFC 9114 Section 8 で定義されるエラーコードと内部エラー型を提供。

use core::fmt;

/// HTTP/3 エラーコード (RFC 9114 Section 8.1)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum ErrorCode {
    /// 正常終了 (0x100)
    NoError = 0x100,
    /// プロトコル全般のエラー (0x101)
    GeneralProtocolError = 0x101,
    /// 内部エラー (0x102)
    InternalError = 0x102,
    /// ストリーム作成エラー (0x103)
    StreamCreationError = 0x103,
    /// クローズドクリティカルストリーム (0x104)
    ClosedCriticalStream = 0x104,
    /// フレーム不正 (0x105)
    FrameUnexpected = 0x105,
    /// フレームエラー (0x106)
    FrameError = 0x106,
    /// 過剰な負荷 (0x107)
    ExcessiveLoad = 0x107,
    /// ID エラー (0x108)
    IdError = 0x108,
    /// 設定エラー (0x109)
    SettingsError = 0x109,
    /// リクエスト不完全 (0x10a)
    MissingSettings = 0x10a,
    /// リクエスト拒否 (0x10b)
    ///
    /// サーバーがリクエストを処理せずに拒否する場合に使用 (RFC 9114 Section 4.1.1)。
    /// このコードで拒否されたリクエストはクライアントが安全に再送できる。
    RequestRejected = 0x10b,
    /// リクエストキャンセル (0x10c)
    RequestCancelled = 0x10c,
    /// リクエスト不完全 (0x10d)
    RequestIncomplete = 0x10d,
    /// メッセージエラー (0x10e)
    MessageError = 0x10e,
    /// 接続クローズ (0x10f)
    ConnectError = 0x10f,
    /// バージョンフォールバック (0x110)
    VersionFallback = 0x110,
    /// QPACK デコンプレッションエラー (0x200)
    QpackDecompressionFailed = 0x200,
    /// QPACK エンコーダーストリームエラー (0x201)
    QpackEncoderStreamError = 0x201,
    /// QPACK デコーダーストリームエラー (0x202)
    QpackDecoderStreamError = 0x202,
    /// HTTP Datagram エラー (0x33)
    ///
    /// RFC 9297 Section 5: Quarter Stream ID が不正な HTTP Datagram を
    /// 受信した場合に接続を閉じるためのエラーコード。
    H3DatagramError = 0x33,
}

impl ErrorCode {
    /// エラーコードから `ErrorCode` を作成
    pub fn from_code(code: u64) -> Option<Self> {
        match code {
            0x100 => Some(Self::NoError),
            0x101 => Some(Self::GeneralProtocolError),
            0x102 => Some(Self::InternalError),
            0x103 => Some(Self::StreamCreationError),
            0x104 => Some(Self::ClosedCriticalStream),
            0x105 => Some(Self::FrameUnexpected),
            0x106 => Some(Self::FrameError),
            0x107 => Some(Self::ExcessiveLoad),
            0x108 => Some(Self::IdError),
            0x109 => Some(Self::SettingsError),
            0x10a => Some(Self::MissingSettings),
            0x10b => Some(Self::RequestRejected),
            0x10c => Some(Self::RequestCancelled),
            0x10d => Some(Self::RequestIncomplete),
            0x10e => Some(Self::MessageError),
            0x10f => Some(Self::ConnectError),
            0x110 => Some(Self::VersionFallback),
            0x200 => Some(Self::QpackDecompressionFailed),
            0x201 => Some(Self::QpackEncoderStreamError),
            0x202 => Some(Self::QpackDecoderStreamError),
            0x33 => Some(Self::H3DatagramError),
            _ => None,
        }
    }

    /// 数値コードを取得
    pub fn code(self) -> u64 {
        self as u64
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoError => write!(f, "H3_NO_ERROR"),
            Self::GeneralProtocolError => write!(f, "H3_GENERAL_PROTOCOL_ERROR"),
            Self::InternalError => write!(f, "H3_INTERNAL_ERROR"),
            Self::StreamCreationError => write!(f, "H3_STREAM_CREATION_ERROR"),
            Self::ClosedCriticalStream => write!(f, "H3_CLOSED_CRITICAL_STREAM"),
            Self::FrameUnexpected => write!(f, "H3_FRAME_UNEXPECTED"),
            Self::FrameError => write!(f, "H3_FRAME_ERROR"),
            Self::ExcessiveLoad => write!(f, "H3_EXCESSIVE_LOAD"),
            Self::IdError => write!(f, "H3_ID_ERROR"),
            Self::SettingsError => write!(f, "H3_SETTINGS_ERROR"),
            Self::MissingSettings => write!(f, "H3_MISSING_SETTINGS"),
            Self::RequestRejected => write!(f, "H3_REQUEST_REJECTED"),
            Self::RequestCancelled => write!(f, "H3_REQUEST_CANCELLED"),
            Self::RequestIncomplete => write!(f, "H3_REQUEST_INCOMPLETE"),
            Self::MessageError => write!(f, "H3_MESSAGE_ERROR"),
            Self::ConnectError => write!(f, "H3_CONNECT_ERROR"),
            Self::VersionFallback => write!(f, "H3_VERSION_FALLBACK"),
            Self::QpackDecompressionFailed => write!(f, "QPACK_DECOMPRESSION_FAILED"),
            Self::QpackEncoderStreamError => write!(f, "QPACK_ENCODER_STREAM_ERROR"),
            Self::QpackDecoderStreamError => write!(f, "QPACK_DECODER_STREAM_ERROR"),
            Self::H3DatagramError => write!(f, "H3_DATAGRAM_ERROR"),
        }
    }
}

/// HTTP/3 エラー
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// 接続エラー (接続をクローズすべき)
    ConnectionError(ErrorCode),
    /// ストリームエラー (ストリームをリセットすべき)
    StreamError(ErrorCode),
    /// バッファ不足 (追加データが必要)
    BufferTooShort,
    /// 無効なストリーム ID
    InvalidStreamId(u64),
    /// ストリームが見つからない
    StreamNotFound(u64),
    /// ストリームがクローズ済み
    StreamClosed(u64),
    /// 可変長整数デコードエラー
    VarintDecode,
    /// フレームデコードエラー
    FrameDecode(FrameDecodeError),
    /// QPACK エラー
    Qpack(QpackError),
    /// WebTransport セッションが draining 状態
    ///
    /// (draft-ietf-webtrans-http3-15 Section 6)
    /// `WT_DRAIN_SESSION` 受信後、または GOAWAY 受信によりセッションが
    /// グレースフルシャットダウン中の場合に、新規ストリーム/データグラムの
    /// 送信を試みると返される。
    WtSessionDraining(u64),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConnectionError(code) => write!(f, "connection error: {code}"),
            Self::StreamError(code) => write!(f, "stream error: {code}"),
            Self::BufferTooShort => write!(f, "buffer too short"),
            Self::InvalidStreamId(id) => write!(f, "invalid stream id: {id}"),
            Self::StreamNotFound(id) => write!(f, "stream not found: {id}"),
            Self::StreamClosed(id) => write!(f, "stream closed: {id}"),
            Self::VarintDecode => write!(f, "varint decode error"),
            Self::FrameDecode(e) => write!(f, "frame decode error: {e}"),
            Self::Qpack(e) => write!(f, "qpack error: {e}"),
            Self::WtSessionDraining(id) => write!(f, "webtransport session draining: {id}"),
        }
    }
}

impl core::error::Error for Error {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::FrameDecode(e) => Some(e),
            Self::Qpack(e) => Some(e),
            _ => None,
        }
    }
}

/// フレームデコードエラー
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameDecodeError {
    /// バッファ不足
    BufferTooShort,
    /// 不明なフレームタイプ
    UnknownFrameType(u64),
    /// 無効なフレーム長
    InvalidLength,
    /// 無効な設定 ID
    InvalidSettingsId(u64),
    /// HTTP/2 専用フレームの検出
    Http2Frame(u64),
    /// サーバープッシュはサポートしない (CANCEL_PUSH, PUSH_PROMISE, MAX_PUSH_ID)
    ServerPushNotSupported(u64),
}

impl fmt::Display for FrameDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BufferTooShort => write!(f, "buffer too short"),
            Self::UnknownFrameType(t) => write!(f, "unknown frame type: {t:#x}"),
            Self::InvalidLength => write!(f, "invalid frame length"),
            Self::InvalidSettingsId(id) => write!(f, "invalid settings id: {id:#x}"),
            Self::Http2Frame(t) => write!(f, "http/2 frame not allowed: {t:#x}"),
            Self::ServerPushNotSupported(t) => {
                write!(f, "server push not supported: {t:#x}")
            }
        }
    }
}

impl core::error::Error for FrameDecodeError {}

/// QPACK エラー
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QpackError {
    /// バッファ不足
    BufferTooShort,
    /// 無効なインデックス
    InvalidIndex(u64),
    /// 無効なハフマン符号
    InvalidHuffman,
    /// 文字列が長すぎる
    StringTooLong,
    /// デコード失敗
    DecodeFailed,
    /// 動的テーブルが無効 (最大容量 0)
    ///
    /// RFC 9204 Section 3.2.3: 最大テーブル容量が 0 の間、
    /// エンコーダーは動的テーブルへの挿入も encoder stream 命令の送信も禁止される。
    DynamicTableDisabled,
    /// 容量が上限を超過
    ///
    /// RFC 9204 Section 3.2.3: エンコーダーは最大テーブル容量を超える
    /// 動的テーブル容量を設定してはならない。
    CapacityExceeded,
}

impl fmt::Display for QpackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BufferTooShort => write!(f, "buffer too short"),
            Self::InvalidIndex(idx) => write!(f, "invalid index: {idx}"),
            Self::InvalidHuffman => write!(f, "invalid huffman encoding"),
            Self::StringTooLong => write!(f, "string too long"),
            Self::DecodeFailed => write!(f, "decode failed"),
            Self::DynamicTableDisabled => {
                write!(f, "dynamic table disabled (max capacity is zero)")
            }
            Self::CapacityExceeded => write!(f, "capacity exceeds maximum table capacity"),
        }
    }
}

impl core::error::Error for QpackError {}

impl From<FrameDecodeError> for Error {
    fn from(e: FrameDecodeError) -> Self {
        Self::FrameDecode(e)
    }
}

impl From<QpackError> for Error {
    fn from(e: QpackError) -> Self {
        Self::Qpack(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_code_from_code() {
        assert_eq!(ErrorCode::from_code(0x100), Some(ErrorCode::NoError));
        assert_eq!(
            ErrorCode::from_code(0x101),
            Some(ErrorCode::GeneralProtocolError)
        );
        assert_eq!(
            ErrorCode::from_code(0x200),
            Some(ErrorCode::QpackDecompressionFailed)
        );
        assert_eq!(
            ErrorCode::from_code(0x10b),
            Some(ErrorCode::RequestRejected)
        );
        assert_eq!(ErrorCode::from_code(0x999), None);
    }

    #[test]
    fn test_error_code_display() {
        assert_eq!(format!("{}", ErrorCode::NoError), "H3_NO_ERROR");
        assert_eq!(format!("{}", ErrorCode::FrameError), "H3_FRAME_ERROR");
        assert_eq!(
            format!("{}", ErrorCode::RequestRejected),
            "H3_REQUEST_REJECTED"
        );
    }

    #[test]
    fn test_error_display() {
        let e = Error::ConnectionError(ErrorCode::FrameError);
        assert_eq!(format!("{e}"), "connection error: H3_FRAME_ERROR");

        let e = Error::BufferTooShort;
        assert_eq!(format!("{e}"), "buffer too short");
    }
}
