//! tokio-s2n-quic エラー型

use std::fmt;

/// tokio-s2n-quic エラー型
#[derive(Debug)]
pub enum Error {
    /// s2n-quic トランスポートエラー
    Transport(Box<dyn std::error::Error + Send + Sync>),
    /// HTTP/3 プロトコルエラー
    Http3(shiguredo_http3::Error),
    /// 接続がクローズ済み
    ConnectionClosed,
    /// ストリームがクローズ済み
    StreamClosed,
    /// 無効な状態
    InvalidState(String),
    /// 内部エラー
    Internal(String),
}

impl Error {
    /// 任意のエラー型から Transport バリアントを生成する
    ///
    /// 具象エラー型でも Box<dyn Error + Send + Sync> でも受け取れる
    pub fn transport(e: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> Self {
        Self::Transport(e.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(e) => write!(f, "transport error: {e}"),
            Self::Http3(e) => write!(f, "http3 error: {e}"),
            Self::ConnectionClosed => write!(f, "connection closed"),
            Self::StreamClosed => write!(f, "stream closed"),
            Self::InvalidState(msg) => write!(f, "invalid state: {msg}"),
            Self::Internal(msg) => write!(f, "internal error: {msg}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<shiguredo_http3::Error> for Error {
    fn from(e: shiguredo_http3::Error) -> Self {
        Self::Http3(e)
    }
}

impl From<shiguredo_http3::HeaderError> for Error {
    fn from(e: shiguredo_http3::HeaderError) -> Self {
        Self::InvalidState(format!("invalid header: {e}"))
    }
}

impl From<s2n_quic::connection::Error> for Error {
    fn from(e: s2n_quic::connection::Error) -> Self {
        Self::transport(e)
    }
}

impl From<s2n_quic::stream::Error> for Error {
    fn from(e: s2n_quic::stream::Error) -> Self {
        Self::transport(e)
    }
}

/// tokio-s2n-quic の Result 型
pub type Result<T> = std::result::Result<T, Error>;
