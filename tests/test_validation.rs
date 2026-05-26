//! validation モジュールの単体テスト

use shiguredo_http3::qpack::{Decoder, Header};
use shiguredo_http3::validation::*;

/// QPACK 整数 (RFC 7541 Section 5.1) を指定 prefix bits で符号化する。
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

/// QPACK 文字列リテラル (H=0) を指定 prefix/prefix_bits で符号化する。
fn encode_qpack_literal(buf: &mut Vec<u8>, data: &[u8], prefix_bits: u8, prefix: u8) {
    encode_qpack_integer(buf, data.len() as u64, prefix_bits, prefix);
    buf.extend_from_slice(data);
}

/// QPACK 文字列リテラル (H=0, 7-bit prefix) を符号化する。
fn encode_qpack_string(buf: &mut Vec<u8>, data: &[u8]) {
    encode_qpack_literal(buf, data, 7, 0x00);
}

/// 検査なしの name/value を QPACK Literal Field Line with Literal Name
/// (RFC 9204 Section 4.5.6) として符号化し、Decoder でデコードして
/// Header を返す (wire 模擬)。
fn wire_header(name: &[u8], value: &[u8]) -> Header {
    let mut wire = Vec::new();
    // Required Insert Count = 0
    wire.push(0x00);
    // Delta Base = 0
    wire.push(0x00);
    // Literal Field Line with Literal Name: 001N=0, H=0
    encode_qpack_literal(&mut wire, name, 3, 0x20);
    // Value: H=0, 7-bit prefix
    encode_qpack_string(&mut wire, value);

    let decoder = Decoder::new();
    let headers = decoder
        .decode(&wire)
        .expect("infallible: wire_header produced invalid QPACK");
    headers
        .into_iter()
        .next()
        .expect("infallible: wire encoding produces exactly one header")
}

// =========================================================================
// リクエスト検証
// =========================================================================

#[test]
fn test_valid_get_request() {
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/"),
        wire_header(b":authority", b"example.com"),
    ];
    assert!(validate_request_headers(&headers).is_ok());
}

#[test]
fn test_valid_connect_request() {
    let headers = vec![
        wire_header(b":method", b"CONNECT"),
        wire_header(b":authority", b"example.com:443"),
    ];
    assert!(validate_request_headers(&headers).is_ok());
}

#[test]
fn test_valid_extended_connect_request() {
    // WebTransport 等の Extended CONNECT (RFC 8441, RFC 9220)
    let headers = vec![
        wire_header(b":method", b"CONNECT"),
        wire_header(b":protocol", b"webtransport-h3"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/webtransport"),
        wire_header(b":authority", b"example.com"),
    ];
    assert!(validate_request_headers(&headers).is_ok());
}

#[test]
fn test_extended_connect_missing_scheme_is_malformed() {
    let headers = vec![
        wire_header(b":method", b"CONNECT"),
        wire_header(b":protocol", b"webtransport-h3"),
        wire_header(b":path", b"/"),
        wire_header(b":authority", b"example.com"),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_extended_connect_missing_path_is_malformed() {
    let headers = vec![
        wire_header(b":method", b"CONNECT"),
        wire_header(b":protocol", b"webtransport-h3"),
        wire_header(b":scheme", b"https"),
        wire_header(b":authority", b"example.com"),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_protocol_on_non_connect_is_malformed() {
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/"),
        wire_header(b":protocol", b"webtransport-h3"),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_connect_with_scheme_is_malformed() {
    let headers = vec![
        wire_header(b":method", b"CONNECT"),
        wire_header(b":scheme", b"https"),
        wire_header(b":authority", b"example.com:443"),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_connect_with_path_is_malformed() {
    let headers = vec![
        wire_header(b":method", b"CONNECT"),
        wire_header(b":path", b"/"),
        wire_header(b":authority", b"example.com:443"),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_connect_without_authority_is_malformed() {
    let headers = vec![wire_header(b":method", b"CONNECT")];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_missing_method_is_malformed() {
    let headers = vec![
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/"),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_missing_scheme_is_malformed() {
    let headers = vec![wire_header(b":method", b"GET"), wire_header(b":path", b"/")];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_missing_path_is_malformed() {
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"https"),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_empty_path_is_malformed() {
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b""),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_pseudo_after_regular_is_malformed() {
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b"accept", b"*/*"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/"),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_uppercase_field_name_is_malformed() {
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/"),
        wire_header(b"Content-Type", b"text/html"),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_connection_field_is_malformed() {
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/"),
        wire_header(b"connection", b"keep-alive"),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_te_trailers_is_allowed() {
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/"),
        wire_header(b":authority", b"example.com"),
        wire_header(b"te", b"trailers"),
    ];
    assert!(validate_request_headers(&headers).is_ok());
}

#[test]
fn test_te_non_trailers_is_malformed() {
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/"),
        wire_header(b":authority", b"example.com"),
        wire_header(b"te", b"gzip"),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_https_without_authority_or_host_is_malformed() {
    // https scheme では :authority または Host が必須 (RFC 9114 Section 4.3.1)
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/"),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_http_without_authority_or_host_is_malformed() {
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"http"),
        wire_header(b":path", b"/"),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_https_with_host_only_is_valid() {
    // :authority がなくても Host があれば有効
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/"),
        wire_header(b"host", b"example.com"),
    ];
    assert!(validate_request_headers(&headers).is_ok());
}

#[test]
fn test_authority_host_mismatch_is_malformed() {
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/"),
        wire_header(b":authority", b"example.com"),
        wire_header(b"host", b"other.com"),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_duplicate_method_is_malformed() {
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":method", b"POST"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/"),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_status_in_request_is_malformed() {
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":status", b"200"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/"),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_unknown_pseudo_header_is_malformed() {
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/"),
        wire_header(b":unknown", b"value"),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

// =========================================================================
// レスポンス検証
// =========================================================================

#[test]
fn test_valid_response() {
    let headers = vec![
        wire_header(b":status", b"200"),
        wire_header(b"content-type", b"text/html"),
    ];
    assert!(validate_response_headers(&headers).is_ok());
}

#[test]
fn test_status_101_is_rejected() {
    // HTTP/3 は 101 (Switching Protocols) をサポートしない (RFC 9114 Section 4.5)
    let headers = vec![wire_header(b":status", b"101")];
    assert!(validate_response_headers(&headers).is_err());
}

#[test]
fn test_status_100_is_valid() {
    // 100 (Continue) は有効な中間レスポンス
    let headers = vec![wire_header(b":status", b"100")];
    assert!(validate_response_headers(&headers).is_ok());
}

#[test]
fn test_status_non_digit_is_malformed() {
    // :status の値が非数字は malformed (RFC 9114 Section 4.1.2, 4.3.2)
    let headers = vec![wire_header(b":status", b"abc")];
    assert!(validate_response_headers(&headers).is_err());
}

#[test]
fn test_status_two_digits_is_malformed() {
    let headers = vec![wire_header(b":status", b"20")];
    assert!(validate_response_headers(&headers).is_err());
}

#[test]
fn test_status_four_digits_is_malformed() {
    let headers = vec![wire_header(b":status", b"2000")];
    assert!(validate_response_headers(&headers).is_err());
}

#[test]
fn test_missing_status_is_malformed() {
    let headers = vec![wire_header(b"content-type", b"text/html")];
    assert!(validate_response_headers(&headers).is_err());
}

#[test]
fn test_method_in_response_is_malformed() {
    let headers = vec![
        wire_header(b":status", b"200"),
        wire_header(b":method", b"GET"),
    ];
    assert!(validate_response_headers(&headers).is_err());
}

#[test]
fn test_response_uppercase_field_name_is_malformed() {
    let headers = vec![
        wire_header(b":status", b"200"),
        wire_header(b"Content-Type", b"text/html"),
    ];
    assert!(validate_response_headers(&headers).is_err());
}

#[test]
fn test_response_connection_field_is_malformed() {
    let headers = vec![
        wire_header(b":status", b"200"),
        wire_header(b"transfer-encoding", b"chunked"),
    ];
    assert!(validate_response_headers(&headers).is_err());
}

// =========================================================================
// トレーラー検証
// =========================================================================

#[test]
fn test_valid_trailer() {
    // 疑似ヘッダーなし・通常フィールドのみのトレーラーは正当
    let headers = vec![wire_header(b"x-checksum", b"abc123")];
    assert!(validate_trailer_headers(&headers).is_ok());
}

#[test]
fn test_empty_trailer_is_valid() {
    // フィールドなしのトレーラーも正当
    let headers: Vec<Header> = vec![];
    assert!(validate_trailer_headers(&headers).is_ok());
}

#[test]
fn test_trailer_with_status_is_malformed() {
    // トレーラーに :status は禁止 (RFC 9114 Section 4.3)
    let headers = vec![wire_header(b":status", b"200")];
    assert!(validate_trailer_headers(&headers).is_err());
}

#[test]
fn test_trailer_with_method_is_malformed() {
    // トレーラーにリクエスト疑似ヘッダーも禁止 (RFC 9114 Section 4.3)
    let headers = vec![wire_header(b":method", b"GET")];
    assert!(validate_trailer_headers(&headers).is_err());
}

#[test]
fn test_trailer_uppercase_field_name_is_malformed() {
    let headers = vec![wire_header(b"X-Checksum", b"abc123")];
    assert!(validate_trailer_headers(&headers).is_err());
}

#[test]
fn test_trailer_connection_field_is_malformed() {
    let headers = vec![wire_header(b"transfer-encoding", b"chunked")];
    assert!(validate_trailer_headers(&headers).is_err());
}

// =========================================================================
// 0024: TE ヘッダーのレスポンス/トレーラー拒否 (RFC 9114 Section 4.2)
// =========================================================================

#[test]
fn test_te_in_response_is_malformed() {
    // TE はリクエストのみ許可 (RFC 9114 Section 4.2)
    let headers = vec![
        wire_header(b":status", b"200"),
        wire_header(b"te", b"trailers"),
    ];
    assert!(validate_response_headers(&headers).is_err());
}

#[test]
fn test_te_in_trailer_is_malformed() {
    // TE はリクエストのみ許可 (RFC 9114 Section 4.2)
    let headers = vec![wire_header(b"te", b"trailers")];
    assert!(validate_trailer_headers(&headers).is_err());
}

// =========================================================================
// 0025: field-value の NUL / CR / LF 拒否 (RFC 9114 Section 10.3)
// =========================================================================

#[test]
fn test_request_field_value_with_nul_is_malformed() {
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/"),
        wire_header(b":authority", b"example.com"),
        wire_header(b"x-header", b"val\x00ue"),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_request_field_value_with_cr_is_malformed() {
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/"),
        wire_header(b":authority", b"example.com"),
        wire_header(b"x-header", b"val\x0due"),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_request_field_value_with_lf_is_malformed() {
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/"),
        wire_header(b":authority", b"example.com"),
        wire_header(b"x-header", b"val\x0aue"),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_response_field_value_with_nul_is_malformed() {
    let headers = vec![
        wire_header(b":status", b"200"),
        wire_header(b"x-header", b"val\x00ue"),
    ];
    assert!(validate_response_headers(&headers).is_err());
}

#[test]
fn test_response_field_value_with_cr_is_malformed() {
    let headers = vec![
        wire_header(b":status", b"200"),
        wire_header(b"x-header", b"val\x0due"),
    ];
    assert!(validate_response_headers(&headers).is_err());
}

#[test]
fn test_response_field_value_with_lf_is_malformed() {
    let headers = vec![
        wire_header(b":status", b"200"),
        wire_header(b"x-header", b"val\x0aue"),
    ];
    assert!(validate_response_headers(&headers).is_err());
}

#[test]
fn test_trailer_field_value_with_nul_is_malformed() {
    let headers = vec![wire_header(b"x-checksum", b"abc\x00def")];
    assert!(validate_trailer_headers(&headers).is_err());
}

#[test]
fn test_trailer_field_value_with_cr_is_malformed() {
    let headers = vec![wire_header(b"x-checksum", b"abc\x0ddef")];
    assert!(validate_trailer_headers(&headers).is_err());
}

#[test]
fn test_trailer_field_value_with_lf_is_malformed() {
    let headers = vec![wire_header(b"x-checksum", b"abc\x0adef")];
    assert!(validate_trailer_headers(&headers).is_err());
}

// =========================================================================
// 0027: content-length と DATA フレームの整合性検証 (RFC 9114 Section 4.1.2)
// =========================================================================

#[test]
fn test_content_length_absent_is_ok() {
    // content-length なし: 検証不要
    let headers = vec![wire_header(b":status", b"200")];
    assert!(validate_content_length(&headers, 0, false).is_ok());
    assert!(validate_content_length(&headers, 100, false).is_ok());
}

#[test]
fn test_content_length_zero_with_empty_body_is_ok() {
    // content-length: 0 かつ body 空は正当
    let headers = vec![wire_header(b"content-length", b"0")];
    assert!(validate_content_length(&headers, 0, false).is_ok());
}

#[test]
fn test_content_length_matches_body_size_is_ok() {
    // content-length の値と body サイズが一致
    let headers = vec![wire_header(b"content-length", b"10")];
    assert!(validate_content_length(&headers, 10, false).is_ok());
}

#[test]
fn test_duplicate_content_length_is_malformed() {
    // content-length が 2 個: malformed
    let headers = vec![
        wire_header(b"content-length", b"10"),
        wire_header(b"content-length", b"10"),
    ];
    assert!(validate_content_length(&headers, 10, false).is_err());
}

#[test]
fn test_content_length_too_large_is_malformed() {
    // content-length の値が body サイズより大きい
    let headers = vec![wire_header(b"content-length", b"10")];
    assert!(validate_content_length(&headers, 5, false).is_err());
}

#[test]
fn test_content_length_too_small_is_malformed() {
    // content-length の値が body サイズより小さい
    let headers = vec![wire_header(b"content-length", b"5")];
    assert!(validate_content_length(&headers, 10, false).is_err());
}

#[test]
fn test_content_length_non_numeric_is_malformed() {
    // content-length が非数値: malformed
    let headers = vec![wire_header(b"content-length", b"abc")];
    assert!(validate_content_length(&headers, 0, false).is_err());
}

#[test]
fn test_content_length_skip_body_check_is_ok() {
    // skip_body_check = true の場合は body サイズ不一致でも Ok (HEAD/1xx/204/304)
    let headers = vec![wire_header(b"content-length", b"100")];
    assert!(validate_content_length(&headers, 0, true).is_ok());
}

// =========================================================================
// 0026: Extended CONNECT の :authority 検証 (RFC 9114 Section 4.3.1, RFC 8441)
// =========================================================================

#[test]
fn test_extended_connect_https_without_authority_is_malformed() {
    // https scheme では :authority が必須 (RFC 9114 Section 4.3.1, RFC 8441 Section 4)
    let headers = vec![
        wire_header(b":method", b"CONNECT"),
        wire_header(b":protocol", b"webtransport-h3"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/webtransport"),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_extended_connect_https_with_empty_authority_is_malformed() {
    let headers = vec![
        wire_header(b":method", b"CONNECT"),
        wire_header(b":protocol", b"webtransport-h3"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/webtransport"),
        wire_header(b":authority", b""),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_extended_connect_https_with_host_is_valid() {
    // Host ヘッダーで代替可能 (RFC 9114 Section 4.3.1)
    let headers = vec![
        wire_header(b":method", b"CONNECT"),
        wire_header(b":protocol", b"webtransport-h3"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/webtransport"),
        wire_header(b"host", b"example.com"),
    ];
    assert!(validate_request_headers(&headers).is_ok());
}

#[test]
fn test_extended_connect_authority_host_mismatch_is_malformed() {
    let headers = vec![
        wire_header(b":method", b"CONNECT"),
        wire_header(b":protocol", b"webtransport-h3"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/webtransport"),
        wire_header(b":authority", b"example.com"),
        wire_header(b"host", b"other.com"),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

// =========================================================================
// 0045: フィールド名の不正文字検証 (RFC 9110 Section 5.1, RFC 9114 Section 10.3)
// =========================================================================

#[test]
fn test_field_name_with_space_is_malformed() {
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/"),
        wire_header(b":authority", b"example.com"),
        wire_header(b"invalid name", b"value"),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_field_name_with_control_char_is_malformed() {
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/"),
        wire_header(b":authority", b"example.com"),
        wire_header(b"x-hdr\x01", b"value"),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_field_name_with_slash_is_malformed() {
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/"),
        wire_header(b":authority", b"example.com"),
        wire_header(b"x/hdr", b"value"),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_field_name_with_colon_is_malformed() {
    // 通常ヘッダーでコロンを含む名前は不正 (token に含まれない)
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/"),
        wire_header(b":authority", b"example.com"),
        wire_header(b"x:hdr", b"value"),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_field_name_with_at_sign_is_malformed() {
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/"),
        wire_header(b":authority", b"example.com"),
        wire_header(b"x@hdr", b"value"),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_field_name_with_del_is_malformed() {
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/"),
        wire_header(b":authority", b"example.com"),
        wire_header(b"x-hdr\x7f", b"value"),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_valid_field_name_with_tchar() {
    // tchar に含まれる記号を含むフィールド名は正当
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/"),
        wire_header(b":authority", b"example.com"),
        wire_header(b"x-hdr_name.v1+2", b"value"),
    ];
    assert!(validate_request_headers(&headers).is_ok());
}

#[test]
fn test_response_field_name_with_space_is_malformed() {
    let headers = vec![
        wire_header(b":status", b"200"),
        wire_header(b"invalid name", b"value"),
    ];
    assert!(validate_response_headers(&headers).is_err());
}

#[test]
fn test_trailer_field_name_with_space_is_malformed() {
    let headers = vec![wire_header(b"invalid name", b"value")];
    assert!(validate_trailer_headers(&headers).is_err());
}

// =========================================================================
// 0046: フィールド値の不正文字検証 (RFC 9110 Section 5.5, RFC 9114 Section 10.3)
// =========================================================================

#[test]
fn test_field_value_with_del_is_malformed() {
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/"),
        wire_header(b":authority", b"example.com"),
        wire_header(b"x-hdr", b"val\x7fue"),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_field_value_with_control_0x01_is_malformed() {
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/"),
        wire_header(b":authority", b"example.com"),
        wire_header(b"x-hdr", b"val\x01ue"),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_field_value_with_control_0x1f_is_malformed() {
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/"),
        wire_header(b":authority", b"example.com"),
        wire_header(b"x-hdr", b"val\x1fue"),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_field_value_with_leading_space_is_malformed() {
    // field-content ABNF: 先頭は field-vchar でなければならない
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/"),
        wire_header(b":authority", b"example.com"),
        wire_header(b"x-hdr", b" value"),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_field_value_with_trailing_space_is_malformed() {
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/"),
        wire_header(b":authority", b"example.com"),
        wire_header(b"x-hdr", b"value "),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_field_value_with_leading_htab_is_malformed() {
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/"),
        wire_header(b":authority", b"example.com"),
        wire_header(b"x-hdr", b"\tvalue"),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_field_value_with_middle_space_is_valid() {
    // 途中の SP は field-content で許可
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/"),
        wire_header(b":authority", b"example.com"),
        wire_header(b"x-hdr", b"val ue"),
    ];
    assert!(validate_request_headers(&headers).is_ok());
}

#[test]
fn test_field_value_with_middle_htab_is_valid() {
    // 途中の HTAB は field-content で許可
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/"),
        wire_header(b":authority", b"example.com"),
        wire_header(b"x-hdr", b"val\tue"),
    ];
    assert!(validate_request_headers(&headers).is_ok());
}

#[test]
fn test_field_value_with_obs_text_is_valid() {
    // obs-text (0x80-0xFF) は許可
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/"),
        wire_header(b":authority", b"example.com"),
        wire_header(b"x-hdr", b"\x80\xff"),
    ];
    assert!(validate_request_headers(&headers).is_ok());
}

#[test]
fn test_empty_field_value_is_valid() {
    // 空のフィールド値は field-value = *field-content で許可
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/"),
        wire_header(b":authority", b"example.com"),
        wire_header(b"x-hdr", b""),
    ];
    assert!(validate_request_headers(&headers).is_ok());
}

#[test]
fn test_response_field_value_with_del_is_malformed() {
    let headers = vec![
        wire_header(b":status", b"200"),
        wire_header(b"x-hdr", b"val\x7fue"),
    ];
    assert!(validate_response_headers(&headers).is_err());
}

#[test]
fn test_trailer_field_value_with_del_is_malformed() {
    let headers = vec![wire_header(b"x-checksum", b"abc\x7fdef")];
    assert!(validate_trailer_headers(&headers).is_err());
}

// =========================================================================
// 0051: :authority の userinfo 拒否 (RFC 9114 Section 4.3.1)
// =========================================================================

#[test]
fn test_authority_with_userinfo_is_malformed() {
    // http/https scheme で :authority に userinfo を含むのは不正
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/"),
        wire_header(b":authority", b"user:pass@example.com"),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_authority_with_userinfo_http_is_malformed() {
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"http"),
        wire_header(b":path", b"/"),
        wire_header(b":authority", b"user@example.com"),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_authority_without_userinfo_is_valid() {
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/"),
        wire_header(b":authority", b"example.com:443"),
    ];
    assert!(validate_request_headers(&headers).is_ok());
}

#[test]
fn test_connect_authority_with_at_sign_is_malformed() {
    // authority-form は uri-host ":" port であり userinfo を含まない
    // (RFC 9110 Section 7.1) ため '@' は不正
    let headers = vec![
        wire_header(b":method", b"CONNECT"),
        wire_header(b":authority", b"user@example.com:443"),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_extended_connect_authority_with_userinfo_is_malformed() {
    // Extended CONNECT で https scheme の場合は userinfo チェック適用
    let headers = vec![
        wire_header(b":method", b"CONNECT"),
        wire_header(b":protocol", b"webtransport-h3"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/webtransport"),
        wire_header(b":authority", b"user@example.com"),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_non_http_scheme_with_authority_is_valid() {
    // 非 http/https スキームでも :authority を許可する
    // RFC 9114 Section 4.3.1 の MUST NOT は「scheme が mandatory authority を
    // 持たず、かつリクエストターゲットに authority がない」場合のみ適用される。
    // ライブラリはスキーム固有の authority 要件を判断しない。
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"ftp"),
        wire_header(b":path", b"/"),
        wire_header(b":authority", b"example.com"),
    ];
    assert!(validate_request_headers(&headers).is_ok());
}

#[test]
fn test_non_http_scheme_with_host_is_valid() {
    // 非 http/https スキームでも Host を許可する
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"ftp"),
        wire_header(b":path", b"/"),
        wire_header(b"host", b"example.com"),
    ];
    assert!(validate_request_headers(&headers).is_ok());
}

#[test]
fn test_non_http_scheme_without_authority_is_valid() {
    // 非 http/https スキームで :authority も Host もない場合も正当
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"ftp"),
        wire_header(b":path", b"/files"),
    ];
    assert!(validate_request_headers(&headers).is_ok());
}

// P1: :method の token 検証
#[test]
fn test_method_with_space_is_malformed() {
    // method は token (RFC 9110 Section 9.1) なので空白を含めない
    let headers = vec![
        wire_header(b":method", b"GE T"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/"),
        wire_header(b":authority", b"example.com"),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_method_with_control_char_is_malformed() {
    let headers = vec![
        wire_header(b":method", b"GET\x01"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/"),
        wire_header(b":authority", b"example.com"),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_method_with_slash_is_malformed() {
    // '/' は tchar ではない
    let headers = vec![
        wire_header(b":method", b"GE/T"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/"),
        wire_header(b":authority", b"example.com"),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

// P1: :scheme の文法検証
#[test]
fn test_scheme_starting_with_digit_is_malformed() {
    // scheme は ALPHA で始まる (RFC 3986 Section 3.1)
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"1bad"),
        wire_header(b":path", b"/"),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_scheme_with_space_is_malformed() {
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"ht tps"),
        wire_header(b":path", b"/"),
        wire_header(b":authority", b"example.com"),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_scheme_with_valid_special_chars() {
    // scheme = ALPHA *( ALPHA / DIGIT / "+" / "-" / "." )
    // "coap+tcp" は妥当
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"coap+tcp"),
        wire_header(b":path", b"/resource"),
    ];
    assert!(validate_request_headers(&headers).is_ok());
}

// P1: :path の検証
#[test]
fn test_path_not_starting_with_slash_is_malformed() {
    // http/https では path-absolute ("/" 始まり) が必須
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"abc"),
        wire_header(b":authority", b"example.com"),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_options_asterisk_path_is_valid() {
    // OPTIONS の場合は "*" が許可される
    let headers = vec![
        wire_header(b":method", b"OPTIONS"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"*"),
        wire_header(b":authority", b"example.com"),
    ];
    assert!(validate_request_headers(&headers).is_ok());
}

#[test]
fn test_non_options_asterisk_path_is_malformed() {
    // OPTIONS 以外で "*" は不正
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"*"),
        wire_header(b":authority", b"example.com"),
    ];
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_path_with_query_is_valid() {
    let headers = vec![
        wire_header(b":method", b"GET"),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/search?q=hello"),
        wire_header(b":authority", b"example.com"),
    ];
    assert!(validate_request_headers(&headers).is_ok());
}

// P2: Extended CONNECT でも authority 不要 scheme のチェック
#[test]
fn test_extended_connect_non_http_scheme_with_authority_is_valid() {
    // 非 http/https スキームでも :authority を許可する
    let headers = vec![
        wire_header(b":method", b"CONNECT"),
        wire_header(b":protocol", b"websocket"),
        wire_header(b":scheme", b"ftp"),
        wire_header(b":path", b"/ws"),
        wire_header(b":authority", b"example.com"),
    ];
    assert!(validate_request_headers(&headers).is_ok());
}

#[test]
fn test_extended_connect_non_http_scheme_without_authority_is_valid() {
    let headers = vec![
        wire_header(b":method", b"CONNECT"),
        wire_header(b":protocol", b"websocket"),
        wire_header(b":scheme", b"ftp"),
        wire_header(b":path", b"/ws"),
    ];
    assert!(validate_request_headers(&headers).is_ok());
}

// =========================================================================
// `:protocol` の HTTP Upgrade Token 検査 (RFC 9110 Section 7.8)
// =========================================================================

fn ext_connect_headers(protocol: &[u8]) -> Vec<Header> {
    vec![
        wire_header(b":method", b"CONNECT"),
        wire_header(b":protocol", protocol),
        wire_header(b":scheme", b"https"),
        wire_header(b":path", b"/wt"),
        wire_header(b":authority", b"example.com"),
    ]
}

#[test]
fn test_protocol_empty_is_malformed() {
    let headers = ext_connect_headers(b"");
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_protocol_token_only_is_valid() {
    let headers = ext_connect_headers(b"webtransport-h3");
    assert!(validate_request_headers(&headers).is_ok());
}

#[test]
fn test_protocol_with_version_is_valid() {
    let headers = ext_connect_headers(b"h3/1.0");
    assert!(validate_request_headers(&headers).is_ok());
}

#[test]
fn test_protocol_trailing_slash_is_malformed() {
    let headers = ext_connect_headers(b"h3/");
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_protocol_leading_slash_is_malformed() {
    let headers = ext_connect_headers(b"/h3");
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_protocol_double_slash_is_malformed() {
    let headers = ext_connect_headers(b"a/b/c");
    assert!(validate_request_headers(&headers).is_err());
}

#[test]
fn test_protocol_space_is_malformed() {
    let headers = ext_connect_headers(b"web transport");
    assert!(validate_request_headers(&headers).is_err());
}
