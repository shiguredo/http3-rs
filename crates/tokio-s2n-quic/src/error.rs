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
    /// WebTransport 送信側フロー制御の上限に達した
    ///
    /// ピアが SETTINGS / WT_MAX_STREAMS / WT_MAX_DATA で広告した送信上限を超える
    /// 操作を行おうとした。上限を超えて送信するとピアは WT_FLOW_CONTROL_ERROR で
    /// セッションを閉じるため、統合層が事前に拒否する
    /// (draft-ietf-webtrans-http3-16 Section 5.6.2, 5.6.4)。
    FlowControlBlocked {
        /// 上限に達した資源
        resource: FlowControlResource,
    },
}

/// WebTransport 送信側フロー制御で上限に達する資源
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowControlResource {
    /// 双方向ストリーム (WT_MAX_STREAMS の双方向上限)
    BidirectionalStream,
    /// 単方向ストリーム (WT_MAX_STREAMS の単方向上限)
    UnidirectionalStream,
    /// セッション全体の送信データ量 (WT_MAX_DATA)
    Data,
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
            Self::FlowControlBlocked { resource } => match resource {
                FlowControlResource::BidirectionalStream => {
                    write!(f, "webtransport bidirectional stream limit reached")
                }
                FlowControlResource::UnidirectionalStream => {
                    write!(f, "webtransport unidirectional stream limit reached")
                }
                FlowControlResource::Data => write!(f, "webtransport session data limit reached"),
            },
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flow_control_blocked_display() {
        // フロー制御上限超過のエラー表示が資源ごとに区別できること
        // (draft-ietf-webtrans-http3-16 Section 5.6.2, 5.6.4)
        let cases = [
            (
                FlowControlResource::BidirectionalStream,
                "webtransport bidirectional stream limit reached",
            ),
            (
                FlowControlResource::UnidirectionalStream,
                "webtransport unidirectional stream limit reached",
            ),
            (
                FlowControlResource::Data,
                "webtransport session data limit reached",
            ),
        ];
        for (resource, expected) in cases {
            let error = Error::FlowControlBlocked { resource };
            assert_eq!(error.to_string(), expected, "資源ごとの表示が一致すること");
        }
    }
}
