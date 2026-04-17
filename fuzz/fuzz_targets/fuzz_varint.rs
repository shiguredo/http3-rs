#![no_main]

use libfuzzer_sys::fuzz_target;
use shiguredo_http3::varint;

fuzz_target!(|data: &[u8]| {
    // 任意バイト列に対してパニックしないことを検証
    let _ = varint::decode(data);
    let _ = varint::peek_len(data);
});
