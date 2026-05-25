#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use shiguredo_http3::Header;
use shiguredo_http3::qpack::Decoder;
use shiguredo_http3::validation::{
    calculate_field_section_size, check_field_section_size, validate_content_length,
    validate_request_headers, validate_response_headers, validate_trailer_headers,
};

#[derive(Debug, Arbitrary)]
enum FuzzInput {
    /// 任意のヘッダーでリクエスト検証
    RequestHeaders {
        headers: Vec<(Vec<u8>, Vec<u8>)>,
    },
    /// 任意のヘッダーでレスポンス検証
    ResponseHeaders {
        headers: Vec<(Vec<u8>, Vec<u8>)>,
    },
    /// 任意のヘッダーでトレーラー検証
    TrailerHeaders {
        headers: Vec<(Vec<u8>, Vec<u8>)>,
    },
    /// content-length 検証
    ContentLength {
        headers: Vec<(Vec<u8>, Vec<u8>)>,
        body_size: u64,
        skip_body_check: bool,
    },
    /// フィールドセクションサイズ検証
    FieldSectionSize {
        headers: Vec<(Vec<u8>, Vec<u8>)>,
        peer_max: Option<u64>,
    },
}

/// 生のヘッダータプルを `Header` に変換する (最大 20 件に制限)
///
/// 検証対象は `validate_*_headers` 等の組合せ検査がパニックしないことなので、
/// QPACK Literal Field Line with Literal Name として符号化し Decoder で
/// デコードすることで、構築時検査をバイパスして任意バイト列を `Header` に
/// 注入する (wire 模擬)。
fn to_headers(raw: Vec<(Vec<u8>, Vec<u8>)>) -> Vec<Header> {
    raw.into_iter()
        .take(20)
        .map(|(n, v)| wire_header(&n, &v))
        .collect()
}

/// QPACK wire 模擬: 任意 name/value を QPACK Literal Field Line with
/// Literal Name (RFC 9204 Section 4.5.6) として符号化し Decoder で復号する。
fn wire_header(name: &[u8], value: &[u8]) -> Header {
    let mut wire = Vec::new();
    wire.push(0x00); // Required Insert Count = 0
    wire.push(0x00); // Delta Base = 0
    encode_qpack_literal(&mut wire, name, 3, 0x20);
    encode_qpack_string(&mut wire, value);
    let decoder = Decoder::new().max_field_section_size(u64::MAX);
    let headers = decoder.decode(&wire).expect("infallible: wire encoding");
    headers.into_iter().next().expect("infallible: one header")
}

fn encode_qpack_integer(buf: &mut Vec<u8>, value: u64, prefix_bits: u8, prefix: u8) {
    let max_prefix = (1u64 << prefix_bits) - 1;
    if value < max_prefix {
        buf.push(prefix | value as u8);
    } else {
        buf.push(prefix | max_prefix as u8);
        let mut remaining = value - max_prefix;
        while remaining >= 128 {
            buf.push(0x80 | (remaining & 0x7f) as u8);
            remaining >>= 7;
        }
        buf.push(remaining as u8);
    }
}

fn encode_qpack_literal(buf: &mut Vec<u8>, data: &[u8], prefix_bits: u8, prefix: u8) {
    encode_qpack_integer(buf, data.len() as u64, prefix_bits, prefix);
    buf.extend_from_slice(data);
}

fn encode_qpack_string(buf: &mut Vec<u8>, data: &[u8]) {
    encode_qpack_literal(buf, data, 7, 0x00);
}

fuzz_target!(|input: FuzzInput| {
    match input {
        FuzzInput::RequestHeaders { headers } => {
            let headers = to_headers(headers);
            // 任意入力でパニックしないことを検証
            let _ = validate_request_headers(&headers);
        }
        FuzzInput::ResponseHeaders { headers } => {
            let headers = to_headers(headers);
            let _ = validate_response_headers(&headers);
        }
        FuzzInput::TrailerHeaders { headers } => {
            let headers = to_headers(headers);
            let _ = validate_trailer_headers(&headers);
        }
        FuzzInput::ContentLength {
            headers,
            body_size,
            skip_body_check,
        } => {
            let headers = to_headers(headers);
            let _ = validate_content_length(&headers, body_size, skip_body_check);
        }
        FuzzInput::FieldSectionSize { headers, peer_max } => {
            let headers = to_headers(headers);
            // サイズ計算がオーバーフローしないことを検証
            let _ = calculate_field_section_size(&headers);
            let _ = check_field_section_size(&headers, peer_max);
        }
    }
});
