//! QPACK ヘッダー圧縮 (RFC 9204)
//!
//! HTTP/3 で使用される QPACK ヘッダー圧縮のエンコード/デコード機能を提供。
//!
//! ## 機能
//!
//! - 静的テーブル (99 エントリ) を使用したヘッダー圧縮
//! - 動的テーブルを使用したヘッダー圧縮
//! - ハフマン符号化による文字列圧縮
//! - エンコーダー/デコーダーストリームの処理
//!
//! ## 使用例
//!
//! ```rust
//! use shiguredo_http3::qpack::{Encoder, Decoder, Header};
//!
//! // エンコード
//! let encoder = Encoder::new();
//! let headers = vec![
//!     Header::new(b":method", b"GET").unwrap(),
//!     Header::new(b":path", b"/").unwrap(),
//! ];
//! let mut buf = vec![0u8; 128];
//! let len = encoder.encode(&mut buf, &headers).unwrap();
//!
//! // デコード
//! let decoder = Decoder::new();
//! let decoded = decoder.decode(&buf[..len]).unwrap();
//! assert_eq!(decoded[0].name(), b":method");
//! ```

mod decoder;
pub mod decoder_stream;
pub mod dynamic_table;
mod encoder;
pub mod encoder_stream;
mod header;
pub mod huffman;
pub mod integer;
pub mod table;

pub use decoder::{DecodeOutput, Decoder, DynamicDecoder};
pub use decoder_stream::{DecoderInstruction, DecoderStream, DecoderStreamReceiver};
pub use dynamic_table::{DynamicEntry, DynamicTable};
pub use encoder::{DynamicEncoder, Encoder, estimate_encoded_size};
pub use encoder_stream::{EncoderInstruction, EncoderStream, EncoderStreamReceiver};
pub use header::{Header, HeaderError};
pub use table::{STATIC_TABLE, STATIC_TABLE_LEN, find_static_entry, get_static_entry};
