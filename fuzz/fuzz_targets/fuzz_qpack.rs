#![no_main]

//! QPACK Fuzzing (RFC 9204)
//!
//! QPACK の各コンポーネントに任意バイト列を入力してパニックしないことを検証する。
//! - 静的デコーダー (Decoder)
//! - 動的デコーダー (DynamicDecoder)
//! - エンコーダーストリームレシーバー (EncoderStreamReceiver)
//! - デコーダーストリームレシーバー (DecoderStreamReceiver)
//! - 動的エンコーダー (DynamicEncoder)
//! - 整数エンコード (encode_integer / encode_integer_to_vec)
//! - 整数デコード (decode_integer)
//!
//! エンコード/デコードのラウンドトリップやブロッキングの状態遷移は `prop_qpack.rs`
//! でカバーする (fuzz はパニック安全性のみ)。

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use shiguredo_http3::qpack::{
    Decoder, DecoderStreamReceiver, DynamicDecoder, DynamicEncoder, DynamicTable,
    EncoderStreamReceiver, Header,
};
use shiguredo_http3::qpack::integer;

/// テーブル容量 (バイト)
const TABLE_CAPACITY: u64 = 4096;

#[derive(Debug, Arbitrary)]
enum FuzzInput {
    StaticDecoder(Vec<u8>),
    DynamicDecoder(Vec<u8>),
    EncoderStream(Vec<u8>),
    DecoderStream(Vec<u8>),
    /// 動的エンコーダーに任意のヘッダーリストをエンコード
    DynamicEncoder {
        headers: Vec<(Vec<u8>, Vec<u8>)>,
        blocked_streams_count: u8,
        use_huffman: bool,
    },
    /// QPACK 整数デコード (RFC 9204 Section 4.1.1 / RFC 7541 Section 5.1)
    IntegerDecode {
        data: Vec<u8>,
        /// 0..=255 全範囲。シフトオーバーフローガードを検証する
        prefix_bits: u8,
    },
    /// QPACK 整数エンコード (RFC 9204 Section 4.1.1 / RFC 7541 Section 5.1)
    IntegerEncode {
        value: u64,
        /// 0..=255 全範囲。シフトオーバーフローガードを検証する
        prefix_bits: u8,
        prefix: u8,
    },
}

fuzz_target!(|input: FuzzInput| {
    match input {
        FuzzInput::StaticDecoder(data) => {
            let decoder = Decoder::new();
            let _ = decoder.decode(&data);
        }
        FuzzInput::DynamicDecoder(data) => {
            let mut decoder = DynamicDecoder::new();
            decoder.set_max_table_capacity(TABLE_CAPACITY);
            decoder.set_table_capacity(TABLE_CAPACITY);
            let _ = decoder.decode(&data);
        }
        FuzzInput::EncoderStream(data) => {
            let mut receiver = EncoderStreamReceiver::new();
            receiver.set_max_table_capacity(TABLE_CAPACITY);
            receiver.receive(&data);
            let mut table = DynamicTable::with_capacity(TABLE_CAPACITY);
            loop {
                match receiver.process(&mut table) {
                    Ok(None) | Err(_) => break,
                    Ok(Some(_)) => {}
                }
            }
        }
        FuzzInput::DecoderStream(data) => {
            let mut receiver = DecoderStreamReceiver::new();
            receiver.receive(&data);
            loop {
                match receiver.process(0) {
                    Ok(None) | Err(_) => break,
                    Ok(Some(_)) => {}
                }
            }
        }
        FuzzInput::DynamicEncoder {
            headers,
            blocked_streams_count,
            use_huffman,
        } => {
            let mut encoder = DynamicEncoder::new().use_huffman(use_huffman);
            encoder.set_max_table_capacity(TABLE_CAPACITY);
            encoder.set_table_capacity(TABLE_CAPACITY);
            encoder.set_peer_max_blocked_streams(100);
            // 動的テーブル path を通すためエントリを 1 件投入
            let _ = encoder.insert(b"a".to_vec(), b"b".to_vec());
            let valid: Vec<Header> = headers
                .into_iter()
                .filter_map(|(n, v)| Header::new(n, v).ok())
                .collect();
            if valid.is_empty() {
                return;
            }
            // 十分なマージンでバッファを確保
            let buf_size = valid.len() * 32 + 128;
            let mut buf = vec![0u8; buf_size];
            let _ = encoder.encode(&mut buf, &valid, blocked_streams_count as usize);
        }
        FuzzInput::IntegerDecode { data, prefix_bits } => {
            let _ = integer::decode_integer(&data, prefix_bits);
        }
        FuzzInput::IntegerEncode {
            value,
            prefix_bits,
            prefix,
        } => {
            let mut buf = [0u8; 32];
            let _ = integer::encode_integer(&mut buf, value, prefix_bits, prefix);
            let mut vec = Vec::new();
            integer::encode_integer_to_vec(&mut vec, value, prefix_bits, prefix);
        }
    }
});
