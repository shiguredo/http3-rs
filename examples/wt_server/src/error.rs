/// WebTransport サーバーのエラー型
#[derive(Debug)]
pub enum Error {
    /// WebTransport エラー
    WebTransport(String),
    /// I/O エラー
    Io(std::io::Error),
    /// その他
    Other(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::WebTransport(msg) => write!(f, "WebTransport: {msg}"),
            Error::Io(e) => write!(f, "I/O: {e}"),
            Error::Other(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<crate::webtransport::Error> for Error {
    fn from(e: crate::webtransport::Error) -> Self {
        Error::WebTransport(e.to_string())
    }
}
