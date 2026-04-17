#![no_main]

use libfuzzer_sys::fuzz_target;
use shiguredo_http3::frame::{decode_frame, decode_frame_header};

fuzz_target!(|data: &[u8]| {
    // 任意バイト列に対してパニックしないことを検証
    let _ = decode_frame_header(data);
    let _ = decode_frame(data);
});
