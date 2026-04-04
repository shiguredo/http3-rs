//! tokio-s2n-quic - s2n-quic ベースの async HTTP/3 + WebTransport
//!
//! s2n-quic の tokio ネイティブ設計を活かし、
//! 高レベル async/await HTTP/3 + WebTransport API を提供する。

pub mod config;
pub mod error;
pub mod h3;
mod internal;
pub mod webtransport;

pub use config::{ClientConfig, ServerConfig};
pub use error::{Error, Result};
pub use h3::{
    H3Client, H3ClientRequest, H3ClientResponse, H3Request, H3Response, H3Server,
    H3ServerConnection,
};
pub use shiguredo_http3::webtransport::DraftVersion;
pub use webtransport::{
    WtBiStream, WtClient, WtRecvStream, WtSendStream, WtServer, WtSession, WtSessionRequest,
};
