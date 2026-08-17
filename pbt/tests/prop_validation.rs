//! Property-Based Testing for HTTP/3 メッセージ検証 (RFC 9114 Section 4.1.2)

use pbt::wire_header;
use shiguredo_http3::Header;
use shiguredo_http3::validation::{
    calculate_field_section_size, check_field_section_size, validate_content_length,
    validate_request_headers, validate_response_headers, validate_trailer_headers,
};

// =============================================================================
// 生成ヘルパー
// =============================================================================

/// 有効なフィールド名を生成 (小文字 ASCII + tchar, 擬似ヘッダープレフィックス ':' を含まない)
fn valid_field_name(ctx: &mut noprop::TestCaseContext) -> Vec<u8> {
    // RFC 9110 Section 5.6.2 の tchar のうち小文字のみ (RFC 9114 Section 4.2)
    // 接続固有フィールドと "te" は除外する
    noprop::sample_with_rejection(ctx, 64, |ctx| {
        let len = noprop::sample_usize_in(ctx, 1..=20);
        let mut name = Vec::new();
        for _ in 0..len {
            name.push(noprop::sample_choice(
                ctx,
                b"abcdefghijklmnopqrstuvwxyz0123456789!#$%&*+-.^_`|~",
            ));
        }
        // 接続固有フィールドと te を除外
        let forbidden: &[&[u8]] = &[
            b"connection",
            b"keep-alive",
            b"proxy-connection",
            b"transfer-encoding",
            b"upgrade",
            b"te",
        ];
        (!forbidden.contains(&name.as_slice())).then_some(name)
    })
}

/// 有効なフィールド値を生成 (field-vchar: 0x21-0x7e, 空も許可)
fn valid_field_value(ctx: &mut noprop::TestCaseContext) -> Vec<u8> {
    let len = noprop::sample_usize_in(ctx, 0..=100);
    let mut value = Vec::new();
    for _ in 0..len {
        value.push(0x21 + noprop::sample_usize_in(ctx, 0..=0x5d) as u8);
    }
    value
}

/// 有効な HTTP メソッドを生成
fn valid_method(ctx: &mut noprop::TestCaseContext) -> Vec<u8> {
    noprop::sample_choice(
        ctx,
        &[
            b"GET".to_vec(),
            b"POST".to_vec(),
            b"PUT".to_vec(),
            b"DELETE".to_vec(),
            b"HEAD".to_vec(),
            b"OPTIONS".to_vec(),
            b"PATCH".to_vec(),
        ],
    )
}

/// 有効な :status 値を生成 (100-599 の 3 桁 ASCII 文字列、ただし 101 は除外)
fn valid_status(ctx: &mut noprop::TestCaseContext) -> Vec<u8> {
    noprop::sample_with_rejection(ctx, 64, |ctx| {
        let s = noprop::sample_u64_in(ctx, 100..600);
        // HTTP/3 は 101 をサポートしない
        (s != 101).then_some(format!("{s:03}").into_bytes())
    })
}

/// 有効な URI scheme を生成
fn valid_scheme(ctx: &mut noprop::TestCaseContext) -> Vec<u8> {
    noprop::sample_choice(ctx, &[b"http".to_vec(), b"https".to_vec()])
}

/// 有効な :path を生成 ("/" + 0-20 文字の英数字)
fn valid_path(ctx: &mut noprop::TestCaseContext) -> Vec<u8> {
    let len = noprop::sample_usize_in(ctx, 0..=20);
    let mut path = vec![b'/'];
    for _ in 0..len {
        path.push(noprop::sample_choice(
            ctx,
            b"abcdefghijklmnopqrstuvwxyz0123456789",
        ));
    }
    path
}

/// 有効な通常ヘッダーのリストを生成
fn valid_regular_headers(ctx: &mut noprop::TestCaseContext) -> Vec<Header> {
    let len = noprop::sample_usize_in(ctx, 0..=5);
    let mut headers = Vec::new();
    for _ in 0..len {
        let name = valid_field_name(ctx);
        let value = valid_field_value(ctx);
        headers.push(Header::new(name, value).expect("test must succeed"));
    }
    headers
}

// =============================================================================
// (a) field_section_size の計算式が RFC 準拠
// =============================================================================

/// Property: calculate_field_section_size は各ヘッダーの (name_len + value_len + 32) の合計と一致する
#[test]
fn prop_field_section_size_matches_rfc_formula() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_VALIDATION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let count = noprop::sample_usize_in(ctx, 0..=10);
        let mut headers = Vec::new();
        let mut expected = 0u64;
        for _ in 0..count {
            let name = valid_field_name(ctx);
            let value = valid_field_value(ctx);
            // RFC 9114 Section 4.2.2: 各フィールドの name_len + value_len + 32 の合計
            expected += name.len() as u64 + value.len() as u64 + 32;
            headers.push(Header::new(name, value).expect("test must succeed"));
        }

        let actual = calculate_field_section_size(&headers);
        assert_eq!(actual, expected, "RFC 準拠の計算式と不一致");
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// (b) 有効なリクエストヘッダーは常に受理される
// =============================================================================

/// Property: 正しい擬似ヘッダーと有効な通常ヘッダーを持つリクエストは受理される
#[test]
fn prop_valid_request_headers_accepted() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_VALIDATION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let method = valid_method(ctx);
        let scheme = valid_scheme(ctx);
        let path = valid_path(ctx);
        let regular_headers = valid_regular_headers(ctx);
        // 生成されるメソッドに CONNECT は含まれないため、CONNECT 除外は不要

        let mut headers = vec![
            Header::new(b":method", method.as_slice()).expect("test must succeed"),
            Header::new(b":scheme", scheme.as_slice()).expect("test must succeed"),
            Header::new(b":path", path.as_slice()).expect("test must succeed"),
            Header::new(b":authority", b"example.com").expect("test must succeed"),
        ];
        headers.extend(regular_headers);

        let result = validate_request_headers(&headers);
        assert!(result.is_ok(), "有効なリクエストが拒否された: {:?}", result,);
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// (c) 有効なレスポンスヘッダーは常に受理される
// =============================================================================

/// Property: 正しい :status と有効な通常ヘッダーを持つレスポンスは受理される
#[test]
fn prop_valid_response_headers_accepted() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_VALIDATION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let status = valid_status(ctx);
        let regular_headers = valid_regular_headers(ctx);
        let mut headers =
            vec![Header::new(b":status", status.as_slice()).expect("test must succeed")];
        // レスポンスでは "te" ヘッダーは禁止なのでフィルタする
        let filtered: Vec<_> = regular_headers
            .into_iter()
            .filter(|h| h.name() != b"te")
            .collect();
        headers.extend(filtered);

        let result = validate_response_headers(&headers);
        assert!(result.is_ok(), "有効なレスポンスが拒否された: {:?}", result,);
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// (d) 擬似ヘッダーが通常ヘッダーの後に出現すると拒否
// =============================================================================

/// Property: 擬似ヘッダーの後に通常ヘッダーが来てから再び擬似ヘッダーが出現すると MessageError
#[test]
fn prop_pseudo_header_after_regular_rejected() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_VALIDATION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let method = valid_method(ctx);
        let scheme = valid_scheme(ctx);
        let path = valid_path(ctx);
        // 生成されるメソッドに CONNECT は含まれないため、CONNECT 除外は不要

        // 擬似ヘッダー → 通常ヘッダー → 擬似ヘッダー の順序にする
        let headers = vec![
            Header::new(b":method", method.as_slice()).expect("test must succeed"),
            Header::new(b":scheme", scheme.as_slice()).expect("test must succeed"),
            Header::new(b"x-test", b"value").expect("test must succeed"),
            // 通常ヘッダーの後に擬似ヘッダー (不正)
            Header::new(b":path", path.as_slice()).expect("test must succeed"),
        ];

        let result = validate_request_headers(&headers);
        assert!(
            result.is_err(),
            "擬似ヘッダーが通常ヘッダーの後に来ても受理された"
        );
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// (e) 接続固有ヘッダーは必ず拒否
// =============================================================================

/// Property: connection, keep-alive, proxy-connection, transfer-encoding, upgrade を含むリクエストは拒否
#[test]
fn prop_connection_specific_headers_rejected_in_request() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_VALIDATION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let conn_header = noprop::sample_choice(
            ctx,
            &[
                b"connection".to_vec(),
                b"keep-alive".to_vec(),
                b"proxy-connection".to_vec(),
                b"transfer-encoding".to_vec(),
                b"upgrade".to_vec(),
            ],
        );
        let headers = vec![
            Header::new(b":method", b"GET").expect("test must succeed"),
            Header::new(b":scheme", b"https").expect("test must succeed"),
            Header::new(b":path", b"/").expect("test must succeed"),
            Header::new(b":authority", b"example.com").expect("test must succeed"),
            Header::new(conn_header.as_slice(), b"some-value").expect("test must succeed"),
        ];

        let result = validate_request_headers(&headers);
        assert!(
            result.is_err(),
            "接続固有ヘッダー {:?} がリクエストで受理された",
            String::from_utf8_lossy(&conn_header),
        );
        Ok(())
    })?;
    Ok(())
}

/// Property: 接続固有ヘッダーを含むレスポンスは拒否
#[test]
fn prop_connection_specific_headers_rejected_in_response() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_VALIDATION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let conn_header = noprop::sample_choice(
            ctx,
            &[
                b"connection".to_vec(),
                b"keep-alive".to_vec(),
                b"proxy-connection".to_vec(),
                b"transfer-encoding".to_vec(),
                b"upgrade".to_vec(),
            ],
        );
        let headers = vec![
            Header::new(b":status", b"200").expect("test must succeed"),
            Header::new(conn_header.as_slice(), b"some-value").expect("test must succeed"),
        ];

        let result = validate_response_headers(&headers);
        assert!(
            result.is_err(),
            "接続固有ヘッダー {:?} がレスポンスで受理された",
            String::from_utf8_lossy(&conn_header),
        );
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// (f) content-length の一致/不一致
// =============================================================================

/// Property: content-length と body_size が一致すれば Ok
#[test]
fn prop_content_length_match_ok() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_VALIDATION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let body_size = noprop::sample_u64_in(ctx, 0..10000);
        let cl_str = body_size.to_string();
        let headers = vec![
            Header::new(b":method", b"GET").expect("test must succeed"),
            Header::new(b"content-length", cl_str.as_bytes()).expect("test must succeed"),
        ];

        let result = validate_content_length(&headers, body_size, false);
        assert!(result.is_ok(), "一致する content-length が拒否された");
        Ok(())
    })?;
    Ok(())
}

/// Property: content-length と body_size が不一致なら Err
#[test]
fn prop_content_length_mismatch_err() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_VALIDATION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let (expected, actual) = noprop::sample_with_rejection(ctx, 64, |ctx| {
            let e = noprop::sample_u64_in(ctx, 0..10000);
            let a = noprop::sample_u64_in(ctx, 0..10000);
            (e != a).then_some((e, a))
        });

        let cl_str = expected.to_string();
        let headers = vec![
            Header::new(b":method", b"GET").expect("test must succeed"),
            Header::new(b"content-length", cl_str.as_bytes()).expect("test must succeed"),
        ];

        let result = validate_content_length(&headers, actual, false);
        assert!(result.is_err(), "不一致な content-length が受理された");
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// (g) field_section_size の上限チェック
// =============================================================================

/// Property: field_section_size が peer_max 以下なら Ok、超過なら Err
#[test]
fn prop_field_section_size_limit() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_VALIDATION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let count = noprop::sample_usize_in(ctx, 1..=5);
        let mut header_list = Vec::new();
        for _ in 0..count {
            let name = valid_field_name(ctx);
            let value = valid_field_value(ctx);
            header_list.push(Header::new(name, value).expect("test must succeed"));
        }
        let peer_max = noprop::sample_u64_in(ctx, 0..5000);

        let size = calculate_field_section_size(&header_list);
        let result = check_field_section_size(&header_list, Some(peer_max));

        if size <= peer_max {
            assert!(
                result.is_ok(),
                "上限以下なのに拒否された: size={}, max={}",
                size,
                peer_max
            );
        } else {
            assert!(
                result.is_err(),
                "上限超過なのに受理された: size={}, max={}",
                size,
                peer_max
            );
        }
        Ok(())
    })?;
    Ok(())
}

/// Property: peer_max が None の場合は常に Ok
#[test]
fn prop_field_section_size_no_limit() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_VALIDATION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let count = noprop::sample_usize_in(ctx, 0..=5);
        let mut header_list = Vec::new();
        for _ in 0..count {
            let name = valid_field_name(ctx);
            let value = valid_field_value(ctx);
            header_list.push(Header::new(name, value).expect("test must succeed"));
        }

        let result = check_field_section_size(&header_list, None);
        assert!(result.is_ok(), "peer_max=None なのに拒否された");
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// (h) トレーラーに擬似ヘッダーが含まれると拒否
// =============================================================================

/// Property: 擬似ヘッダーを含むトレーラーは拒否される
#[test]
fn prop_trailer_with_pseudo_header_rejected() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_VALIDATION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let pseudo = noprop::sample_choice(
            ctx,
            &[
                b":method".to_vec(),
                b":scheme".to_vec(),
                b":path".to_vec(),
                b":authority".to_vec(),
                b":status".to_vec(),
                b":protocol".to_vec(),
            ],
        );
        let regular_headers = valid_regular_headers(ctx);

        // 有効な通常ヘッダーのみのトレーラーは受理される
        let filtered: Vec<_> = regular_headers
            .iter()
            .filter(|h| h.name() != b"te")
            .cloned()
            .collect();
        if !filtered.is_empty() {
            let result = validate_trailer_headers(&filtered);
            assert!(result.is_ok(), "有効なトレーラーが拒否された: {:?}", result);
        }

        // 擬似ヘッダーを混入
        //
        // `:status` の値 `"value"` は構築時検査 (3DIGIT) に違反するため
        // `Header::new` では構築できない。トレーラーで疑似ヘッダーが拒否される
        // ことの確認が主目的なので、wire 模擬で直接構築する。
        let mut bad_trailer = filtered;
        bad_trailer.push(wire_header(&pseudo, b"value"));

        let result = validate_trailer_headers(&bad_trailer);
        assert!(
            result.is_err(),
            "擬似ヘッダー {:?} を含むトレーラーが受理された",
            String::from_utf8_lossy(&pseudo),
        );
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// `:protocol` 値の構築時検査 (`Header::new`) と validation 側 (`is_valid_protocol`)
// の同値性 (RFC 8441 Section 4 / RFC 9110 Section 7.8)
//
// `qpack::header::check_upgrade_token` と `validation::is_valid_protocol` の
// 挙動が乖離していないことを保証する。
// =============================================================================

/// Property: `Header::new(b":protocol", v).is_ok()` ⇔ Extended CONNECT で
/// 同じ `v` を渡したリクエストが `validate_request_headers` を通る
#[test]
fn prop_protocol_check_consistency() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_VALIDATION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let len = noprop::sample_usize_in(ctx, 0..=32);
        let value = noprop::sample_bytes_vec(ctx, len);
        let build_ok = Header::new(b":protocol", &value).is_ok();

        // wire 模擬で Header を組み立て、validation 側の判定だけを取り出す
        let headers = vec![
            Header::new(b":method", b"CONNECT").expect("test must succeed"),
            wire_header(b":protocol", &value),
            Header::new(b":scheme", b"https").expect("test must succeed"),
            Header::new(b":path", b"/wt").expect("test must succeed"),
            Header::new(b":authority", b"example.com").expect("test must succeed"),
        ];
        let validate_ok = validate_request_headers(&headers).is_ok();

        assert_eq!(
            build_ok, validate_ok,
            "Header::new と validation::is_valid_protocol の判定が乖離: value={:?}",
            value,
        );
        Ok(())
    })?;
    Ok(())
}

/// authority 候補を生成する (host[:port] 構文の有効・無効を混在させる)。
///
/// 文字集合はすべて有効な field-value 文字に限定するため、Host 経路の
/// field-value 検査では落ちず、authority 構文検査 (is_valid_authority) の
/// 結果だけが Ok/Err を分ける。
fn authority_candidate(ctx: &mut noprop::TestCaseContext) -> Vec<u8> {
    let chars = b"ab01.:-[]";
    let len = noprop::sample_usize_in(ctx, 1..16);
    let mut value = Vec::new();
    for _ in 0..len {
        value.push(noprop::sample_choice(ctx, chars));
    }
    value
}

/// :authority 経路と Host 単独経路で authority 構文検証の結果が一致すること。
///
/// Host は :authority の代替として同じ uri-host[:port] 構文で検証される (RFC 9110
/// Section 7.2)。charset は field-value として有効かつ userinfo (@) や空値を含まないため、
/// 両経路で挙動が分かれる差分 (userinfo 拒否 / 空値 / CONNECT) を踏まず、
/// is_valid_authority の結果だけが Ok/Err を分ける。それらの差分は単体テストで固定する。
#[test]
fn prop_host_only_matches_authority_validation() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_VALIDATION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let value = authority_candidate(ctx);
        let with_authority = vec![
            Header::new(b":method", b"GET").expect("test must succeed"),
            Header::new(b":scheme", b"https").expect("test must succeed"),
            Header::new(b":path", b"/").expect("test must succeed"),
            Header::new(b":authority", &value).expect("test must succeed"),
        ];
        let with_host = vec![
            Header::new(b":method", b"GET").expect("test must succeed"),
            Header::new(b":scheme", b"https").expect("test must succeed"),
            Header::new(b":path", b"/").expect("test must succeed"),
            Header::new(b"host", &value).expect("test must succeed"),
        ];
        assert_eq!(
            validate_request_headers(&with_authority).is_ok(),
            validate_request_headers(&with_host).is_ok(),
            "value={:?}",
            value,
        );
        Ok(())
    })?;
    Ok(())
}
