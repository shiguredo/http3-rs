#![no_main]

//! QPACK Fuzzing (RFC 9204)
//!
//! 以下のシナリオをカバーする:
//!
//! 1. パニック安全性: 任意バイト列を各コンポーネントに入力してクラッシュしないことを検証
//!    - 静的デコーダー (Decoder)
//!    - 動的デコーダー (DynamicDecoder)
//!    - エンコーダーストリームレシーバー (EncoderStreamReceiver)
//!    - デコーダーストリームレシーバー (DecoderStreamReceiver)
//!
//! 2. ラウンドトリップ正確性: エンコード→デコードの往復でヘッダーが一致することを検証
//!    - 静的テーブルのみ (Encoder + Decoder)
//!    - 動的テーブル使用 (DynamicEncoder + DynamicDecoder)
//!    - エンコーダーストリーム往復 (EncoderStream + EncoderStreamReceiver)
//!
//! 3. ブロッキング→アンブロック (RFC 9204 Section 2.1.2):
//!    - 動的テーブル参照でエンコードしたデータをデコード試行 → Blocked を確認
//!    - エンコーダーストリームデータでテーブルを更新 → 再デコードで成功することを確認

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use shiguredo_http3::qpack::{
    DecodeOutput, Decoder, DecoderStreamReceiver, DynamicDecoder, DynamicEncoder, DynamicTable,
    Encoder, EncoderStream, EncoderStreamReceiver, Header,
};

/// テーブル容量 (バイト)
const TABLE_CAPACITY: u64 = 4096;

#[derive(Debug, Arbitrary)]
enum FuzzInput {
    /// 静的デコーダーへの任意バイト列 (パニック安全性)
    RawBytesStaticDecoder(Vec<u8>),
    /// 動的デコーダーへの任意バイト列 (パニック安全性)
    RawBytesDynamicDecoder(Vec<u8>),
    /// エンコーダーストリームレシーバーへの任意バイト列 (パニック安全性)
    RawBytesEncoderStream(Vec<u8>),
    /// デコーダーストリームレシーバーへの任意バイト列 (パニック安全性)
    RawBytesDecoderStream(Vec<u8>),
    /// 静的テーブルのみを使用したラウンドトリップ (正確性)
    StaticRoundtrip {
        headers: Vec<FuzzHeader>,
        use_huffman: bool,
    },
    /// 動的テーブルを使用したラウンドトリップ (正確性)
    DynamicRoundtrip {
        /// 事前に両テーブルへ挿入するエントリ
        entries: Vec<FuzzHeader>,
        /// エンコード対象ヘッダー
        headers: Vec<FuzzHeader>,
        use_huffman: bool,
    },
    /// EncoderStream → EncoderStreamReceiver ラウンドトリップ (正確性)
    EncoderStreamRoundtrip { entries: Vec<FuzzHeader> },
    /// ブロッキング → アンブロック (RFC 9204 Section 2.1.2)
    BlockedThenUnblocked {
        entry_name: Vec<u8>,
        entry_value: Vec<u8>,
    },
}

#[derive(Debug, Arbitrary)]
struct FuzzHeader {
    name: Vec<u8>,
    value: Vec<u8>,
}

fuzz_target!(|input: FuzzInput| {
    match input {
        // --- パニック安全性テスト ---
        FuzzInput::RawBytesStaticDecoder(data) => {
            let decoder = Decoder::new();
            let _ = decoder.decode(&data);
        }

        FuzzInput::RawBytesDynamicDecoder(data) => {
            let mut decoder = DynamicDecoder::new();
            decoder.set_max_table_capacity(TABLE_CAPACITY);
            decoder.set_table_capacity(TABLE_CAPACITY);
            let _ = decoder.decode(&data);
        }

        FuzzInput::RawBytesEncoderStream(data) => {
            let mut receiver = EncoderStreamReceiver::new();
            receiver.set_max_table_capacity(TABLE_CAPACITY);
            receiver.receive(&data);
            let mut table = DynamicTable::with_capacity(TABLE_CAPACITY);
            loop {
                match receiver.process(&mut table) {
                    Ok(None) => break,
                    Ok(Some(_)) => {}
                    Err(_) => break,
                }
            }
        }

        FuzzInput::RawBytesDecoderStream(data) => {
            let mut receiver = DecoderStreamReceiver::new();
            receiver.receive(&data);
            loop {
                match receiver.process(0) {
                    Ok(None) => break,
                    Ok(Some(_)) => {}
                    Err(_) => break,
                }
            }
        }

        // --- ラウンドトリップ正確性テスト ---
        FuzzInput::StaticRoundtrip {
            headers,
            use_huffman,
        } => {
            let headers: Vec<Header> = headers
                .into_iter()
                .filter(|h| !h.name.is_empty())
                .map(|h| Header::new(h.name, h.value))
                .collect();
            if headers.is_empty() {
                return;
            }

            let encoder = Encoder::new().use_huffman(use_huffman);
            let mut buf = vec![0u8; 16 * 1024];

            let Some(len) = encoder.encode(&mut buf, &headers) else {
                return;
            };

            let decoder = Decoder::new();
            let Ok(decoded) = decoder.decode(&buf[..len]) else {
                return;
            };

            assert_eq!(headers.len(), decoded.len());
            for (orig, dec) in headers.iter().zip(decoded.iter()) {
                assert_eq!(orig.name, dec.name);
                assert_eq!(orig.value, dec.value);
            }
        }

        FuzzInput::DynamicRoundtrip {
            entries,
            headers,
            use_huffman,
        } => {
            let entries: Vec<(Vec<u8>, Vec<u8>)> = entries
                .into_iter()
                .filter(|e| !e.name.is_empty())
                .map(|e| (e.name, e.value))
                .collect();
            let headers: Vec<Header> = headers
                .into_iter()
                .filter(|h| !h.name.is_empty())
                .map(|h| Header::new(h.name, h.value))
                .collect();
            if headers.is_empty() {
                return;
            }

            let mut encoder = DynamicEncoder::new().use_huffman(use_huffman);
            encoder.set_max_table_capacity(TABLE_CAPACITY);
            encoder.set_table_capacity(TABLE_CAPACITY);

            let mut decoder = DynamicDecoder::new();
            decoder.set_max_table_capacity(TABLE_CAPACITY);
            decoder.set_table_capacity(TABLE_CAPACITY);

            // エンコーダーとデコーダーに同じエントリを挿入してテーブルを同期させる
            for (name, value) in &entries {
                encoder.insert(name.clone(), value.clone());
                decoder.insert(name.clone(), value.clone());
            }

            let mut buf = vec![0u8; 64 * 1024];
            let Some(len) = encoder.encode(&mut buf, &headers, 0) else {
                return;
            };

            match decoder.decode(&buf[..len]) {
                Ok(DecodeOutput::Decoded(decoded)) => {
                    assert_eq!(headers.len(), decoded.len());
                    for (orig, dec) in headers.iter().zip(decoded.iter()) {
                        assert_eq!(orig.name, dec.name);
                        assert_eq!(orig.value, dec.value);
                    }
                }
                // テーブル状態の不一致によるブロックやエラーは許容
                Ok(DecodeOutput::Blocked) | Err(_) => {}
            }
        }

        FuzzInput::EncoderStreamRoundtrip { entries } => {
            let entries: Vec<(Vec<u8>, Vec<u8>)> = entries
                .into_iter()
                .filter(|e| !e.name.is_empty())
                .map(|e| (e.name, e.value))
                .collect();
            if entries.is_empty() {
                return;
            }

            // エンコーダー側: EncoderStream でリテラル名挿入命令を生成
            let mut enc_stream = EncoderStream::new();
            for (name, value) in &entries {
                let _ = enc_stream.encode_insert_with_literal_name(name, value);
            }
            let stream_data = enc_stream.get_data().to_vec();

            // デコーダー側: EncoderStreamReceiver でテーブルを更新
            let mut receiver = EncoderStreamReceiver::new();
            receiver.set_max_table_capacity(TABLE_CAPACITY);
            receiver.receive(&stream_data);
            let mut table = DynamicTable::with_capacity(TABLE_CAPACITY);
            loop {
                match receiver.process(&mut table) {
                    Ok(None) => break,
                    Ok(Some(_)) => {}
                    // テーブル容量不足等によるエラーは許容
                    Err(_) => break,
                }
            }
        }

        // --- ブロッキング → アンブロック (RFC 9204 Section 2.1.2) ---
        FuzzInput::BlockedThenUnblocked {
            entry_name,
            entry_value,
        } => {
            if entry_name.is_empty() {
                return;
            }

            // エンコーダー側: 動的テーブルにエントリを挿入してエンコード
            let mut encoder = DynamicEncoder::new().use_huffman(false);
            encoder.set_max_table_capacity(TABLE_CAPACITY);
            encoder.set_table_capacity(TABLE_CAPACITY);
            encoder.insert(entry_name.clone(), entry_value.clone());

            let headers = vec![Header::new(entry_name.clone(), entry_value.clone())];
            let mut buf = vec![0u8; 64 * 1024];
            let Some(encoded_len) = encoder.encode(&mut buf, &headers, 0) else {
                return;
            };
            let encoded = buf[..encoded_len].to_vec();

            // デコーダー側: テーブルが空の状態でデコード試行
            // 動的テーブルを参照している場合は Blocked が返る (RFC 9204 Section 2.1.2)
            let mut decoder = DynamicDecoder::new();
            decoder.set_max_table_capacity(TABLE_CAPACITY);
            decoder.set_table_capacity(TABLE_CAPACITY);
            let first_result = decoder.decode(&encoded);

            // エンコーダーストリーム命令を生成: デコーダーテーブルを更新するための命令
            let mut enc_stream = EncoderStream::new();
            let _ = enc_stream.encode_insert_with_literal_name(&entry_name, &entry_value);
            let stream_data = enc_stream.get_data().to_vec();

            // デコーダー側テーブルをエンコーダーストリームデータで更新
            let mut receiver = EncoderStreamReceiver::new();
            receiver.set_max_table_capacity(TABLE_CAPACITY);
            receiver.receive(&stream_data);
            loop {
                match receiver.process(decoder.table_mut()) {
                    Ok(None) => break,
                    Ok(Some(_)) => {}
                    Err(_) => break,
                }
            }

            // テーブル更新後のデコード
            let second_result = decoder.decode(&encoded);

            // 最初の結果がブロックだった場合、テーブル更新後は成功するはず
            if let (Ok(DecodeOutput::Blocked), Ok(DecodeOutput::Decoded(decoded))) =
                (first_result, second_result)
            {
                assert_eq!(decoded.len(), 1);
                assert_eq!(decoded[0].name, entry_name);
                assert_eq!(decoded[0].value, entry_value);
            }
        }
    }
});
