#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use shiguredo_http3::Header;
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
/// `Header::new` の構築時検査をスキップして任意バイト列を `Header` 内に
/// 注入する `from_validated_parts` を使う。これにより wire 由来の不正バイト列
/// (decoder 経由) に対する `validate_*` の panic 耐性を fuzz でテストできる。
fn to_headers(raw: Vec<(Vec<u8>, Vec<u8>)>) -> Vec<Header> {
    raw.into_iter()
        .take(20)
        .map(|(n, v)| {
            Header::from_validated_parts(std::borrow::Cow::Owned(n), std::borrow::Cow::Owned(v))
        })
        .collect()
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
