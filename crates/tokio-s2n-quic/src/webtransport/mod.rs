//! WebTransport サーバー/クライアント

pub mod client;
pub mod server;
pub mod session;

pub use client::WtClient;
pub use server::{WtServer, WtSessionRequest};
pub use session::{WtBiStream, WtRecvStream, WtSendStream, WtSession};

// アプリが `WtSession::recv_event` で受け取るイベント型 (sans-I/O 層で定義済み)
pub use shiguredo_http3::WebTransportEvent;
