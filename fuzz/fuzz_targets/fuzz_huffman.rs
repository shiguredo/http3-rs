#![no_main]

use libfuzzer_sys::fuzz_target;
use shiguredo_http3::qpack::huffman;

fuzz_target!(|data: &[u8]| {
    // 任意バイト列に対してパニックしないことを検証
    let _ = huffman::decode(data);
    let _ = huffman::encoded_len(data);

    // encode も任意バイト列を受けるのでパニック安全性を検証
    let len = huffman::encoded_len(data);
    let mut buf = vec![0u8; len];
    let _ = huffman::encode(&mut buf, data);
});
