#![no_main]

use libfuzzer_sys::fuzz_target;
use shiguredo_http3::qpack::huffman;

fuzz_target!(|data: &[u8]| {
    // デコードテスト: 任意のバイト列に対してパニックしないことを検証
    let _ = huffman::decode(data);

    // encoded_len は任意のバイト列に対してパニックしない
    let _ = huffman::encoded_len(data);

    // ラウンドトリップテスト: 任意のバイト列をエンコード → デコード
    let encoded_len = huffman::encoded_len(data);
    let mut buf = vec![0u8; encoded_len];

    if let Some(actual_len) = huffman::encode(&mut buf, data) {
        assert_eq!(actual_len, encoded_len);

        // デコード
        if let Ok(decoded) = huffman::decode(&buf[..actual_len]) {
            // 元のデータと一致することを確認
            assert_eq!(decoded, data);
        }
    }
});
