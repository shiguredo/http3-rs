#![no_main]

use libfuzzer_sys::fuzz_target;
use shiguredo_http3::varint;

fuzz_target!(|data: &[u8]| {
    // デコードテスト: 任意のバイト列に対してパニックしないことを検証
    let _ = varint::decode(data);
    let _ = varint::peek_len(data);

    // ラウンドトリップテスト
    if let Ok((value, _len)) = varint::decode(data) {
        // 値が MAX_VALUE 以下であることを確認 (デコーダーが返す値は常に範囲内)
        assert!(value <= varint::MAX_VALUE);

        // encode → decode でラウンドトリップ
        let mut buf = [0u8; 8];
        if let Ok(encoded_len) = varint::encode(&mut buf, value) {
            // 再デコード
            let (decoded_value, decoded_len) = varint::decode(&buf[..encoded_len]).unwrap();
            assert_eq!(value, decoded_value);
            assert_eq!(encoded_len, decoded_len);

            // 再エンコードして一致を確認
            let mut buf2 = [0u8; 8];
            let encoded_len2 = varint::encode(&mut buf2, decoded_value).unwrap();
            assert_eq!(buf[..encoded_len], buf2[..encoded_len2]);
        }
    }
});
