//! Property-Based Testing for HTTP/3 メッセージ検証 (RFC 9114 Section 4.1.2)

use pbt::wire_header;
use proptest::prelude::*;
use shiguredo_http3::Header;
use shiguredo_http3::validation::{
    calculate_field_section_size, check_field_section_size, validate_content_length,
    validate_request_headers, validate_response_headers, validate_trailer_headers,
};

// =============================================================================
// Strategy ヘルパー
// =============================================================================

/// 有効なフィールド名を生成 (小文字 ASCII + tchar, 擬似ヘッダープレフィックス ':' を含まない)
fn valid_field_name() -> impl Strategy<Value = Vec<u8>> {
    // RFC 9110 Section 5.6.2 の tchar のうち小文字のみ (RFC 9114 Section 4.2)
    // 接続固有フィールドと "te" は除外する
    prop::collection::vec(
        prop::sample::select(vec![
            b'a', b'b', b'c', b'd', b'e', b'f', b'g', b'h', b'i', b'j', b'k', b'l', b'm', b'n',
            b'o', b'p', b'q', b'r', b's', b't', b'u', b'v', b'w', b'x', b'y', b'z', b'0', b'1',
            b'2', b'3', b'4', b'5', b'6', b'7', b'8', b'9', b'!', b'#', b'$', b'%', b'&', b'*',
            b'+', b'-', b'.', b'^', b'_', b'`', b'|', b'~',
        ]),
        1..=20,
    )
    .prop_filter("接続固有フィールドと te を除外", |name| {
        let forbidden: &[&[u8]] = &[
            b"connection",
            b"keep-alive",
            b"proxy-connection",
            b"transfer-encoding",
            b"upgrade",
            b"te",
        ];
        !forbidden.contains(&name.as_slice())
    })
}

/// 有効なフィールド値を生成 (field-vchar: 0x21-0x7e, 空も許可)
fn valid_field_value() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(0x21u8..=0x7e, 0..=100)
}

/// 有効な HTTP メソッドを生成
fn valid_method() -> impl Strategy<Value = Vec<u8>> {
    prop::sample::select(vec![
        b"GET".to_vec(),
        b"POST".to_vec(),
        b"PUT".to_vec(),
        b"DELETE".to_vec(),
        b"HEAD".to_vec(),
        b"OPTIONS".to_vec(),
        b"PATCH".to_vec(),
    ])
}

/// 有効な :status 値を生成 (100-599 の 3 桁 ASCII 文字列、ただし 101 は除外)
fn valid_status() -> impl Strategy<Value = Vec<u8>> {
    (100u32..600)
        .prop_filter("HTTP/3 は 101 をサポートしない", |s| *s != 101)
        .prop_map(|s| format!("{s:03}").into_bytes())
}

/// 有効な URI scheme を生成
fn valid_scheme() -> impl Strategy<Value = Vec<u8>> {
    prop::sample::select(vec![b"http".to_vec(), b"https".to_vec()])
}

/// 有効な :path を生成 ("/" + 0-20 文字の英数字)
fn valid_path() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(
        prop::sample::select(vec![
            b'a', b'b', b'c', b'd', b'e', b'f', b'g', b'h', b'i', b'j', b'k', b'l', b'm', b'n',
            b'o', b'p', b'q', b'r', b's', b't', b'u', b'v', b'w', b'x', b'y', b'z', b'0', b'1',
            b'2', b'3', b'4', b'5', b'6', b'7', b'8', b'9',
        ]),
        0..=20,
    )
    .prop_map(|mut v| {
        v.insert(0, b'/');
        v
    })
}

/// 有効な通常ヘッダーのリストを生成
fn valid_regular_headers() -> impl Strategy<Value = Vec<Header>> {
    prop::collection::vec(
        (valid_field_name(), valid_field_value())
            .prop_map(|(name, value)| Header::new(name, value).unwrap()),
        0..=5,
    )
}

// =============================================================================
// (a) field_section_size の計算式が RFC 準拠
// =============================================================================

proptest! {
    /// Property: calculate_field_section_size は各ヘッダーの (name_len + value_len + 32) の合計と一致する
    #[test]
    fn prop_field_section_size_matches_rfc_formula(
        headers in prop::collection::vec(
            (valid_field_name(), valid_field_value()),
            0..=10,
        )
    ) {
        let header_list: Vec<Header> = headers
            .iter()
            .map(|(n, v)| Header::new(n.as_slice(), v.as_slice()).unwrap())
            .collect();

        // RFC 9114 Section 4.2.2: 各フィールドの name_len + value_len + 32 の合計
        let expected: u64 = headers
            .iter()
            .map(|(n, v)| n.len() as u64 + v.len() as u64 + 32)
            .sum();

        let actual = calculate_field_section_size(&header_list);
        prop_assert_eq!(actual, expected, "RFC 準拠の計算式と不一致");
    }
}

// =============================================================================
// (b) 有効なリクエストヘッダーは常に受理される
// =============================================================================

proptest! {
    /// Property: 正しい擬似ヘッダーと有効な通常ヘッダーを持つリクエストは受理される
    #[test]
    fn prop_valid_request_headers_accepted(
        method in valid_method(),
        scheme in valid_scheme(),
        path in valid_path(),
        regular_headers in valid_regular_headers(),
    ) {
        // CONNECT は :scheme, :path を持たないため除外
        prop_assume!(method != b"CONNECT");

        let mut headers = vec![
            Header::new(b":method", method.as_slice()).unwrap(),
            Header::new(b":scheme", scheme.as_slice()).unwrap(),
            Header::new(b":path", path.as_slice()).unwrap(),
            Header::new(b":authority", b"example.com").unwrap(),
        ];
        headers.extend(regular_headers);

        let result = validate_request_headers(&headers);
        prop_assert!(
            result.is_ok(),
            "有効なリクエストが拒否された: {:?}",
            result,
        );
    }
}

// =============================================================================
// (c) 有効なレスポンスヘッダーは常に受理される
// =============================================================================

proptest! {
    /// Property: 正しい :status と有効な通常ヘッダーを持つレスポンスは受理される
    #[test]
    fn prop_valid_response_headers_accepted(
        status in valid_status(),
        regular_headers in valid_regular_headers(),
    ) {
        let mut headers = vec![Header::new(b":status", status.as_slice()).unwrap()];
        // レスポンスでは "te" ヘッダーは禁止なのでフィルタする
        let filtered: Vec<_> = regular_headers
            .into_iter()
            .filter(|h| h.name() != b"te")
            .collect();
        headers.extend(filtered);

        let result = validate_response_headers(&headers);
        prop_assert!(
            result.is_ok(),
            "有効なレスポンスが拒否された: {:?}",
            result,
        );
    }
}

// =============================================================================
// (d) 擬似ヘッダーが通常ヘッダーの後に出現すると拒否
// =============================================================================

proptest! {
    /// Property: 擬似ヘッダーの後に通常ヘッダーが来てから再び擬似ヘッダーが出現すると MessageError
    #[test]
    fn prop_pseudo_header_after_regular_rejected(
        method in valid_method(),
        scheme in valid_scheme(),
        path in valid_path(),
    ) {
        prop_assume!(method != b"CONNECT");

        // 擬似ヘッダー → 通常ヘッダー → 擬似ヘッダー の順序にする
        let headers = vec![
            Header::new(b":method", method.as_slice()).unwrap(),
            Header::new(b":scheme", scheme.as_slice()).unwrap(),
            Header::new(b"x-test", b"value").unwrap(),
            // 通常ヘッダーの後に擬似ヘッダー (不正)
            Header::new(b":path", path.as_slice()).unwrap(),
        ];

        let result = validate_request_headers(&headers);
        prop_assert!(
            result.is_err(),
            "擬似ヘッダーが通常ヘッダーの後に来ても受理された"
        );
    }
}

// =============================================================================
// (e) 接続固有ヘッダーは必ず拒否
// =============================================================================

proptest! {
    /// Property: connection, keep-alive, proxy-connection, transfer-encoding, upgrade を含むリクエストは拒否
    #[test]
    fn prop_connection_specific_headers_rejected_in_request(
        conn_header in prop::sample::select(vec![
            b"connection".to_vec(),
            b"keep-alive".to_vec(),
            b"proxy-connection".to_vec(),
            b"transfer-encoding".to_vec(),
            b"upgrade".to_vec(),
        ]),
    ) {
        let headers = vec![
            Header::new(b":method", b"GET").unwrap(),
            Header::new(b":scheme", b"https").unwrap(),
            Header::new(b":path", b"/").unwrap(),
            Header::new(b":authority", b"example.com").unwrap(),
            Header::new(conn_header.as_slice(), b"some-value").unwrap(),
        ];

        let result = validate_request_headers(&headers);
        prop_assert!(
            result.is_err(),
            "接続固有ヘッダー {:?} がリクエストで受理された",
            String::from_utf8_lossy(&conn_header),
        );
    }

    /// Property: 接続固有ヘッダーを含むレスポンスは拒否
    #[test]
    fn prop_connection_specific_headers_rejected_in_response(
        conn_header in prop::sample::select(vec![
            b"connection".to_vec(),
            b"keep-alive".to_vec(),
            b"proxy-connection".to_vec(),
            b"transfer-encoding".to_vec(),
            b"upgrade".to_vec(),
        ]),
    ) {
        let headers = vec![
            Header::new(b":status", b"200").unwrap(),
            Header::new(conn_header.as_slice(), b"some-value").unwrap(),
        ];

        let result = validate_response_headers(&headers);
        prop_assert!(
            result.is_err(),
            "接続固有ヘッダー {:?} がレスポンスで受理された",
            String::from_utf8_lossy(&conn_header),
        );
    }
}

// =============================================================================
// (f) content-length の一致/不一致
// =============================================================================

proptest! {
    /// Property: content-length と body_size が一致すれば Ok
    #[test]
    fn prop_content_length_match_ok(body_size in 0u64..10000) {
        let cl_str = body_size.to_string();
        let headers = vec![
            Header::new(b":method", b"GET").unwrap(),
            Header::new(b"content-length", cl_str.as_bytes()).unwrap(),
        ];

        let result = validate_content_length(&headers, body_size, false);
        prop_assert!(result.is_ok(), "一致する content-length が拒否された");
    }

    /// Property: content-length と body_size が不一致なら Err
    #[test]
    fn prop_content_length_mismatch_err(
        expected in 0u64..10000,
        actual in 0u64..10000,
    ) {
        prop_assume!(expected != actual);

        let cl_str = expected.to_string();
        let headers = vec![
            Header::new(b":method", b"GET").unwrap(),
            Header::new(b"content-length", cl_str.as_bytes()).unwrap(),
        ];

        let result = validate_content_length(&headers, actual, false);
        prop_assert!(result.is_err(), "不一致な content-length が受理された");
    }
}

// =============================================================================
// (g) field_section_size の上限チェック
// =============================================================================

proptest! {
    /// Property: field_section_size が peer_max 以下なら Ok、超過なら Err
    #[test]
    fn prop_field_section_size_limit(
        headers in prop::collection::vec(
            (valid_field_name(), valid_field_value()),
            1..=5,
        ),
        peer_max in 0u64..5000,
    ) {
        let header_list: Vec<Header> = headers
            .iter()
            .map(|(n, v)| Header::new(n.as_slice(), v.as_slice()).unwrap())
            .collect();

        let size = calculate_field_section_size(&header_list);
        let result = check_field_section_size(&header_list, Some(peer_max));

        if size <= peer_max {
            prop_assert!(result.is_ok(), "上限以下なのに拒否された: size={}, max={}", size, peer_max);
        } else {
            prop_assert!(result.is_err(), "上限超過なのに受理された: size={}, max={}", size, peer_max);
        }
    }

    /// Property: peer_max が None の場合は常に Ok
    #[test]
    fn prop_field_section_size_no_limit(
        headers in prop::collection::vec(
            (valid_field_name(), valid_field_value()),
            0..=5,
        ),
    ) {
        let header_list: Vec<Header> = headers
            .iter()
            .map(|(n, v)| Header::new(n.as_slice(), v.as_slice()).unwrap())
            .collect();

        let result = check_field_section_size(&header_list, None);
        prop_assert!(result.is_ok(), "peer_max=None なのに拒否された");
    }
}

// =============================================================================
// (h) トレーラーに擬似ヘッダーが含まれると拒否
// =============================================================================

proptest! {
    /// Property: 擬似ヘッダーを含むトレーラーは拒否される
    #[test]
    fn prop_trailer_with_pseudo_header_rejected(
        pseudo in prop::sample::select(vec![
            b":method".to_vec(),
            b":scheme".to_vec(),
            b":path".to_vec(),
            b":authority".to_vec(),
            b":status".to_vec(),
            b":protocol".to_vec(),
        ]),
        regular_headers in valid_regular_headers(),
    ) {
        // 有効な通常ヘッダーのみのトレーラーは受理される
        let filtered: Vec<_> = regular_headers
            .iter()
            .filter(|h| h.name() != b"te")
            .cloned()
            .collect();
        if !filtered.is_empty() {
            let result = validate_trailer_headers(&filtered);
            prop_assert!(result.is_ok(), "有効なトレーラーが拒否された: {:?}", result);
        }

        // 擬似ヘッダーを混入
        //
        // `:status` の値 `"value"` は構築時検査 (3DIGIT) に違反するため
        // `Header::new` では構築できない。トレーラーで疑似ヘッダーが拒否される
        // ことの確認が主目的なので、wire 模擬で直接構築する。
        let mut bad_trailer = filtered;
        bad_trailer.push(wire_header(&pseudo, b"value"));

        let result = validate_trailer_headers(&bad_trailer);
        prop_assert!(
            result.is_err(),
            "擬似ヘッダー {:?} を含むトレーラーが受理された",
            String::from_utf8_lossy(&pseudo),
        );
    }
}

// =============================================================================
// `:protocol` 値の構築時検査 (`Header::new`) と validation 側 (`is_valid_protocol`)
// の同値性 (RFC 8441 Section 4 / RFC 9110 Section 7.8)
//
// `qpack::header::check_upgrade_token` と `validation::is_valid_protocol` の
// 挙動が乖離していないことを保証する。
// =============================================================================

proptest! {
    /// Property: `Header::new(b":protocol", v).is_ok()` ⇔ Extended CONNECT で
    /// 同じ `v` を渡したリクエストが `validate_request_headers` を通る
    #[test]
    fn prop_protocol_check_consistency(
        value in prop::collection::vec(any::<u8>(), 0..=32),
    ) {
        let build_ok = Header::new(b":protocol", &value).is_ok();

        // wire 模擬で Header を組み立て、validation 側の判定だけを取り出す
        let headers = vec![
            Header::new(b":method", b"CONNECT").unwrap(),
            wire_header(b":protocol", &value),
            Header::new(b":scheme", b"https").unwrap(),
            Header::new(b":path", b"/wt").unwrap(),
            Header::new(b":authority", b"example.com").unwrap(),
        ];
        let validate_ok = validate_request_headers(&headers).is_ok();

        prop_assert_eq!(
            build_ok, validate_ok,
            "Header::new と validation::is_valid_protocol の判定が乖離: value={:?}",
            value,
        );
    }
}

/// authority 候補を生成する (host[:port] 構文の有効・無効を混在させる)。
///
/// 文字集合はすべて有効な field-value 文字に限定するため、Host 経路の
/// field-value 検査では落ちず、authority 構文検査 (is_valid_authority) の
/// 結果だけが Ok/Err を分ける。
fn authority_candidate() -> impl Strategy<Value = Vec<u8>> {
    proptest::collection::vec(prop::sample::select(b"ab01.:-[]".to_vec()), 1..16)
}

proptest! {
    /// :authority 経路と Host 単独経路で authority 構文検証の結果が一致すること。
    ///
    /// Host は :authority の代替として同じ uri-host[:port] 構文で検証される (RFC 9110
    /// Section 7.2)。charset は field-value として有効かつ userinfo (@) や空値を含まないため、
    /// 両経路で挙動が分かれる差分 (userinfo 拒否 / 空値 / CONNECT) を踏まず、
    /// is_valid_authority の結果だけが Ok/Err を分ける。それらの差分は単体テストで固定する。
    #[test]
    fn prop_host_only_matches_authority_validation(value in authority_candidate()) {
        let with_authority = vec![
            Header::new(b":method", b"GET").unwrap(),
            Header::new(b":scheme", b"https").unwrap(),
            Header::new(b":path", b"/").unwrap(),
            Header::new(b":authority", &value).unwrap(),
        ];
        let with_host = vec![
            Header::new(b":method", b"GET").unwrap(),
            Header::new(b":scheme", b"https").unwrap(),
            Header::new(b":path", b"/").unwrap(),
            Header::new(b"host", &value).unwrap(),
        ];
        prop_assert_eq!(
            validate_request_headers(&with_authority).is_ok(),
            validate_request_headers(&with_host).is_ok(),
            "value={:?}",
            value,
        );
    }
}
