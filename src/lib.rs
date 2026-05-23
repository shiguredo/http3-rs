//! shiguredo_http3 - Sans I/O HTTP/3 ライブラリ
//!
//! QUIC 非依存の HTTP/3 実装。任意の QUIC 実装と組み合わせて使用可能。
//!
//! ## 機能
//!
//! - Sans I/O 設計: QUIC トランスポートに依存しない
//! - HTTP/3 フレームのエンコード/デコード
//! - QPACK ヘッダー圧縮 (静的テーブル + 動的テーブル)
//! - クライアント/サーバー接続管理
//! - WebTransport サポート
//!
//! ## 使用例
//!
//! ```rust,ignore
//! use shiguredo_http3::{ClientConnection, Header, Event, Settings};
//!
//! // クライアント接続を作成
//! let mut conn = ClientConnection::new(Settings::default());
//!
//! // 制御ストリーム ID を設定
//! conn.set_control_stream_id(2).unwrap();
//!
//! // リクエストを送信
//! let stream_id = conn.send_request(&[
//!     Header::new(b":method", b"GET").unwrap(),
//!     Header::new(b":path", b"/").unwrap(),
//!     Header::new(b":scheme", b"https").unwrap(),
//!     Header::new(b":authority", b"example.com").unwrap(),
//! ], true).unwrap();
//!
//! // QUIC からデータを受信
//! conn.feed_stream(stream_id, &response_data, fin).unwrap();
//!
//! // イベントを処理
//! while let Some(event) = conn.poll_event() {
//!     match event {
//!         Event::HeadersEnd { stream_id } => println!("Headers received"),
//!         Event::Data { stream_id, data } => println!("Data: {:?}", data),
//!         _ => {}
//!     }
//! }
//! ```

pub mod connection;
pub mod error;
pub mod event;
pub mod frame;
pub mod limits;
pub mod qpack;
pub mod settings;
pub mod stream;
pub mod validation;
/// QUIC 可変長整数エンコーディング
pub mod varint;
pub mod webtransport;

// 公開 API
pub use connection::{ClientConnection, Connection, H3InitData, Role, ServerConnection};
pub use error::{Error, ErrorCode, FrameDecodeError, QpackError};
pub use event::Event;
pub use frame::{
    DataPayload, Frame, FrameHeader, FrameType, GoawayPayload, HeadersPayload, SettingsPayload,
};
pub use limits::Limits;
pub use qpack::{
    Decoder as QpackDecoder, DecoderInstruction, DecoderStream, DecoderStreamReceiver,
    DynamicDecoder, DynamicEncoder, DynamicEntry, DynamicTable, Encoder as QpackEncoder,
    EncoderInstruction, EncoderStream, EncoderStreamReceiver, Header, HeaderError,
};
pub use settings::{Settings, SettingsId};
pub use stream::{RequestStream, StreamKind, StreamState, UniStreamType};
pub use varint::{VarInt, VarIntError};
