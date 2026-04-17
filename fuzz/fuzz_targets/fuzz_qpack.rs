#![no_main]

//! QPACK Fuzzing (RFC 9204)
//!
//! QPACK の各コンポーネントに任意バイト列を入力してパニックしないことを検証する。
//! - 静的デコーダー (Decoder)
//! - 動的デコーダー (DynamicDecoder)
//! - エンコーダーストリームレシーバー (EncoderStreamReceiver)
//! - デコーダーストリームレシーバー (DecoderStreamReceiver)
//!
//! エンコード/デコードのラウンドトリップやブロッキングの状態遷移は `prop_qpack.rs`
//! でカバーする (fuzz はパニック安全性のみ)。

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use shiguredo_http3::qpack::{
    Decoder, DecoderStreamReceiver, DynamicDecoder, DynamicTable, EncoderStreamReceiver,
};

/// テーブル容量 (バイト)
const TABLE_CAPACITY: u64 = 4096;

#[derive(Debug, Arbitrary)]
enum FuzzInput {
    StaticDecoder(Vec<u8>),
    DynamicDecoder(Vec<u8>),
    EncoderStream(Vec<u8>),
    DecoderStream(Vec<u8>),
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
    }
});
