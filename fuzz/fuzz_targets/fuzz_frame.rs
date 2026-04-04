#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use shiguredo_http3::frame::{
    DataPayload, Frame, GoawayPayload, HeadersPayload, SettingsPayload, decode_frame,
    decode_frame_header, encode_frame, encoded_frame_len,
};

#[derive(Debug, Arbitrary)]
enum FuzzInput {
    // 任意のバイト列でのデコードテスト
    RawBytes(Vec<u8>),
    // 構造化された入力でのラウンドトリップテスト
    Data { data: Vec<u8> },
    Headers { encoded_field_section: Vec<u8> },
    Settings { entries: Vec<(u32, u32)> },
    Goaway { id: u32 },
}

fuzz_target!(|input: FuzzInput| {
    match input {
        FuzzInput::RawBytes(data) => {
            // デコードテスト: 任意のバイト列に対してパニックしないことを検証
            let _ = decode_frame_header(&data);
            let _ = decode_frame(&data);

            // ラウンドトリップテスト
            if let Ok((frame, _consumed)) = decode_frame(&data) {
                // エンコードバッファを確保
                let required_len = encoded_frame_len(&frame);
                let mut buf = vec![0u8; required_len];

                // エンコード
                if let Some(encoded_len) = encode_frame(&mut buf, &frame) {
                    assert_eq!(encoded_len, required_len);

                    // 再デコード
                    if let Ok((decoded_frame, decoded_consumed)) = decode_frame(&buf) {
                        // フレームが一致することを確認
                        assert_eq!(frame, decoded_frame);
                        assert_eq!(encoded_len, decoded_consumed);
                    }
                }
            }
        }
        _ => {
            // 構造化された入力でのラウンドトリップテスト
            let frame = match input {
                FuzzInput::Data { data } => Frame::Data(DataPayload::new(data)),
                FuzzInput::Headers {
                    encoded_field_section,
                } => Frame::Headers(HeadersPayload::new(encoded_field_section)),
                FuzzInput::Settings { entries } => {
                    let mut payload = SettingsPayload::new();
                    for (id, value) in entries {
                        // HTTP/2 専用設定 ID を避ける
                        let safe_id = if matches!(id, 0x02 | 0x03 | 0x04 | 0x05) {
                            0x06 + (id as u64)
                        } else {
                            id as u64
                        };
                        payload.add(safe_id, value as u64);
                    }
                    Frame::Settings(payload)
                }
                FuzzInput::Goaway { id } => Frame::Goaway(GoawayPayload::new(id as u64)),
                FuzzInput::RawBytes(_) => unreachable!(),
            };

            // エンコード
            let required_len = encoded_frame_len(&frame);
            let mut buf = vec![0u8; required_len];

            if let Some(encoded_len) = encode_frame(&mut buf, &frame) {
                assert_eq!(encoded_len, required_len);

                // デコード
                if let Ok((decoded_frame, decoded_consumed)) = decode_frame(&buf) {
                    assert_eq!(frame, decoded_frame);
                    assert_eq!(encoded_len, decoded_consumed);
                }
            }
        }
    }
});
