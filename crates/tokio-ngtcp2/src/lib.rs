//! crates.io の shiguredo_ngtcp2_tokio と shiguredo_http3 を組み合わせた
//! 非同期 HTTP/3 / WebTransport クライアント / サーバー
//!
//! QUIC トランスポートは [shiguredo_ngtcp2_tokio](https://crates.io/crates/shiguredo_ngtcp2_tokio)
//! が提供するイベントベースの API を使用し、HTTP/3 の状態機械は
//! `shiguredo_http3` が担う。このクレートは両者をつなぐドライバだけを提供する。

mod client;
mod h3;
mod server;
mod webtransport;

pub use client::Client;
pub use server::Server;
pub use webtransport::{ClientWebTransportSession, ServerWebTransportSession};

use std::fmt;

/// このクレートのエラー
#[derive(Debug)]
pub enum Error {
    /// QUIC 層 (shiguredo_ngtcp2_tokio) のエラー
    Quic(shiguredo_ngtcp2_tokio::Error),
    /// HTTP/3 層 (shiguredo_http3) のエラー
    ///
    /// `shiguredo_http3::Error::StreamError` は該当ストリームのみの問題であり、
    /// 接続を維持したまま呼び出し元に返される。
    Http3(shiguredo_http3::Error),
    /// 操作がタイムアウトした
    Timeout,
    /// 引数が不正
    InvalidArgument(String),
    /// 役割に対して許可されない操作、または前提条件を満たしていない
    InvalidState(&'static str),
    /// WebTransport セッションが終了した
    WebTransportClosed {
        /// アプリケーションエラーコード
        error_code: u32,
        /// エラーメッセージ
        message: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Quic(e) => write!(f, "QUIC error: {e}"),
            Self::Http3(e) => write!(f, "HTTP/3 error: {e}"),
            Self::Timeout => write!(f, "operation timed out"),
            Self::InvalidArgument(msg) => write!(f, "invalid argument: {msg}"),
            Self::InvalidState(msg) => write!(f, "invalid state: {msg}"),
            Self::WebTransportClosed {
                error_code,
                message,
            } => write!(
                f,
                "WebTransport session closed: error_code={error_code} message={message}"
            ),
        }
    }
}

impl std::error::Error for Error {}

impl From<shiguredo_ngtcp2_tokio::Error> for Error {
    fn from(value: shiguredo_ngtcp2_tokio::Error) -> Self {
        Self::Quic(value)
    }
}

impl From<shiguredo_http3::Error> for Error {
    fn from(value: shiguredo_http3::Error) -> Self {
        Self::Http3(value)
    }
}

impl Error {
    /// HTTP/3 のストリームエラーかどうか
    ///
    /// ストリームエラーは該当ストリームだけの問題であり、接続を閉じる根拠に
    /// ならない (RFC 9114 Section 8.1)。接続エラー (`ConnectionError`) や QUIC 層の
    /// エラーは接続が回復不能なため `false` を返す。
    pub(crate) fn is_stream_error(&self) -> bool {
        matches!(self, Self::Http3(shiguredo_http3::Error::StreamError(_)))
    }
}

/// このクレートの結果型
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_stream_error() {
        // HTTP/3 のストリームエラーは接続を閉じない対象であること
        let stream_error = Error::Http3(shiguredo_http3::Error::StreamError(
            shiguredo_http3::ErrorCode::MessageError,
        ));
        assert!(
            stream_error.is_stream_error(),
            "StreamError は接続を閉じない対象であること"
        );

        // HTTP/3 の接続エラーは接続を閉じる対象であること
        let connection_error = Error::Http3(shiguredo_http3::Error::ConnectionError(
            shiguredo_http3::ErrorCode::FrameUnexpected,
        ));
        assert!(
            !connection_error.is_stream_error(),
            "ConnectionError は接続を閉じる対象であること"
        );

        // タイムアウト等の HTTP/3 以外のエラーはストリームエラーではないこと
        assert!(
            !Error::Timeout.is_stream_error(),
            "Timeout は接続を閉じる対象であること"
        );
    }
}
