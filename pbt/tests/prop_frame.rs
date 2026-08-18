//! Property-Based Testing for HTTP/3 Frames (RFC 9114)

use shiguredo_http3::VarInt;
use shiguredo_http3::frame::{
    DataPayload, Frame, FrameType, GoawayPayload, HeadersPayload, SettingsPayload, UnknownFrame,
    decode_frame, encode_frame, encoded_frame_len,
};

/// 有効なフレームタイプを生成
fn valid_frame_type(ctx: &mut noprop::TestCaseContext) -> FrameType {
    noprop::sample_choice(
        ctx,
        &[
            FrameType::Data,
            FrameType::Headers,
            FrameType::CancelPush,
            FrameType::Settings,
            FrameType::PushPromise,
            FrameType::Goaway,
            FrameType::MaxPushId,
        ],
    )
}

/// 有効なペイロードデータを生成
fn valid_payload(ctx: &mut noprop::TestCaseContext) -> Vec<u8> {
    let len = noprop::sample_usize_in(ctx, 0..1024);
    noprop::sample_bytes_vec(ctx, len)
}

/// 有効なストリーム ID を生成 (GOAWAY 用)
fn valid_goaway_id(ctx: &mut noprop::TestCaseContext) -> u64 {
    // クライアント開始双方向ストリームは 4 の倍数
    noprop::sample_u64_in(ctx, 0..1000) * 4
}

/// 有効な QPACK エンコード済みヘッダーを生成
fn valid_encoded_headers(ctx: &mut noprop::TestCaseContext) -> Vec<u8> {
    let len = noprop::sample_usize_in(ctx, 2..256);
    let data = noprop::sample_bytes_vec(ctx, len);
    let mut result = vec![0x00, 0x00]; // RIC=0, Delta Base=0
    result.extend(data.into_iter().skip(2));
    result
}

/// 有効な SETTINGS エントリを生成
///
/// 重複 ID を除外し、bool 値 ID (0x08 / 0x33 / 0x2b603742) は値域を 0/1 に制限する。
/// H3 コア (0x01 / 0x06 / 0x07 / 0x08 / 0x33) と WebTransport 拡張 ID を網羅する。
/// [`shiguredo_http3::Setting::from_wire`] で構築可能な値のみを返す。
fn valid_settings_entries(ctx: &mut noprop::TestCaseContext) -> Vec<shiguredo_http3::Setting> {
    use shiguredo_http3::{Setting, VarInt};
    let ids = [
        0x01u64, 0x06, 0x07, 0x08, 0x33, 0x2b61, 0x2b64, 0x2b65, 0x14e9cd29, 0x2c7cf000,
        0x2b603742, 0xc671706a,
    ];
    let count = noprop::sample_usize_in(ctx, 0..8);
    let mut seen = std::collections::HashSet::new();
    let mut entries = Vec::new();
    for _ in 0..count {
        let id = noprop::sample_choice(ctx, &ids);
        // 重複 ID はスキップする
        if !seen.insert(id) {
            continue;
        }
        let value = noprop::sample_u64_in(ctx, 0..65536);
        let normalized = if matches!(id, 0x08 | 0x33 | 0x2b603742) {
            value & 1
        } else {
            value
        };
        entries.push(
            Setting::from_wire(
                VarInt::new(id).expect("test must succeed"),
                VarInt::new(normalized).expect("test must succeed"),
            )
            .expect("test must succeed"),
        );
    }
    entries
}

// =============================================================================
// Frame Type Properties
// =============================================================================

/// Property: from_type は既知のタイプに対して Some を返す
#[test]
fn prop_known_frame_type_recognized() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_FRAME_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let frame_type = valid_frame_type(ctx);
        let type_value = frame_type as u64;
        let parsed = FrameType::from_type(type_value);

        assert!(
            parsed.is_some(),
            "Frame type {:?} (0x{:02x}) should be recognized",
            frame_type,
            type_value
        );
        assert_eq!(parsed.expect("test must succeed"), frame_type);
        Ok(())
    })?;
    Ok(())
}

/// Property: HTTP/2 専用フレームタイプは is_http2_only で検出される
#[test]
fn prop_http2_frame_types_detected() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_FRAME_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let frame_type = noprop::sample_choice(ctx, &[0x02u64, 0x06, 0x08, 0x09]);
        assert!(
            FrameType::is_http2_only(frame_type),
            "Frame type 0x{:02x} should be HTTP/2 only",
            frame_type
        );
        Ok(())
    })?;
    Ok(())
}

/// Property: 有効な HTTP/3 フレームタイプは HTTP/2 専用ではない
#[test]
fn prop_http3_frame_types_not_http2_only() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_FRAME_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let frame_type = valid_frame_type(ctx);
        let type_value = frame_type as u64;
        assert!(
            !FrameType::is_http2_only(type_value),
            "HTTP/3 frame type 0x{:02x} should not be HTTP/2 only",
            type_value
        );
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// DATA Frame Properties
// =============================================================================

/// Property: DATA フレームのエンコード/デコードラウンドトリップ
#[test]
fn prop_data_frame_roundtrip() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_FRAME_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let payload = valid_payload(ctx);
        let frame = Frame::Data(DataPayload::new(payload.clone()));

        let mut buf = vec![0u8; encoded_frame_len(&frame).expect("test must succeed")];
        let encoded_len = encode_frame(&mut buf, &frame).expect("test must succeed");

        let (decoded, decoded_len) = decode_frame(&buf[..encoded_len]).expect("test must succeed");

        assert_eq!(encoded_len, decoded_len, "Encoded/decoded length mismatch");

        if let Frame::Data(data) = decoded {
            assert_eq!(data.data(), payload.as_slice(), "DATA payload mismatch");
        } else {
            panic!("Expected DATA frame");
        }
        Ok(())
    })?;
    Ok(())
}

/// Property: DATA フレームの長さはペイロード長と一致
#[test]
fn prop_data_frame_length_matches_payload() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_FRAME_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let payload = valid_payload(ctx);
        let frame = Frame::Data(DataPayload::new(payload.clone()));

        // FrameType::Data は 0x00 (1 バイト VarInt) で構築不能ではないため、
        // ランタイム値の `VarInt::new` を使って整合性を取る (const 文脈ではないので
        // `from_static` は使わない)。
        let expected_len = VarInt::new(FrameType::Data as u64)
            .expect("test must succeed")
            .encoded_len()
            + VarInt::new(payload.len() as u64)
                .expect("test must succeed")
                .encoded_len()
            + payload.len();

        let actual_len = encoded_frame_len(&frame).expect("test must succeed");

        assert_eq!(
            actual_len, expected_len,
            "Frame length calculation mismatch"
        );
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// HEADERS Frame Properties
// =============================================================================

/// Property: HEADERS フレームのエンコード/デコードラウンドトリップ
#[test]
fn prop_headers_frame_roundtrip() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_FRAME_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let encoded_block = valid_encoded_headers(ctx);
        let frame = Frame::Headers(HeadersPayload::new(encoded_block.clone()));

        let mut buf = vec![0u8; encoded_frame_len(&frame).expect("test must succeed")];
        let encoded_len = encode_frame(&mut buf, &frame).expect("test must succeed");

        let (decoded, decoded_len) = decode_frame(&buf[..encoded_len]).expect("test must succeed");

        assert_eq!(encoded_len, decoded_len);

        if let Frame::Headers(headers) = decoded {
            assert_eq!(
                headers.encoded_field_section(),
                encoded_block.as_slice(),
                "HEADERS encoded block mismatch"
            );
        } else {
            panic!("Expected HEADERS frame");
        }
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// SETTINGS Frame Properties
// =============================================================================

/// Property: SETTINGS フレームのエンコード/デコードラウンドトリップ
#[test]
fn prop_settings_frame_roundtrip() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_FRAME_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let entries = valid_settings_entries(ctx);
        // valid_settings_entries() は重複 ID を除外済みのため `add` は必ず Ok
        let mut payload = SettingsPayload::new();
        for setting in &entries {
            payload.add(*setting).expect("test must succeed");
        }
        let frame = Frame::Settings(payload);

        let mut buf = vec![0u8; encoded_frame_len(&frame).expect("test must succeed")];
        let encoded_len = encode_frame(&mut buf, &frame).expect("test must succeed");

        let (decoded, decoded_len) = decode_frame(&buf[..encoded_len]).expect("test must succeed");

        assert_eq!(encoded_len, decoded_len);

        if let Frame::Settings(settings) = decoded {
            assert_eq!(settings.settings().len(), entries.len());
            for (orig, decoded) in entries.iter().zip(settings.settings().iter()) {
                assert_eq!(orig, decoded);
            }
        } else {
            panic!("Expected SETTINGS frame");
        }
        Ok(())
    })?;
    Ok(())
}

/// Property: 空の SETTINGS フレームは有効
#[test]
fn prop_empty_settings_frame_valid() {
    let frame = Frame::Settings(SettingsPayload::new());

    let mut buf = vec![0u8; encoded_frame_len(&frame).expect("test must succeed")];
    let encoded_len = encode_frame(&mut buf, &frame).expect("test must succeed");

    let (decoded, decoded_len) = decode_frame(&buf[..encoded_len]).expect("test must succeed");

    assert_eq!(encoded_len, decoded_len);

    if let Frame::Settings(settings) = decoded {
        assert!(settings.is_empty());
    } else {
        panic!("Expected SETTINGS frame");
    }
}

// =============================================================================
// GOAWAY Frame Properties
// =============================================================================

/// Property: GOAWAY フレームのエンコード/デコードラウンドトリップ
#[test]
fn prop_goaway_frame_roundtrip() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_FRAME_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let raw_id = valid_goaway_id(ctx);
        let id = VarInt::new(raw_id).expect("test must succeed");
        let frame = Frame::Goaway(GoawayPayload::new(id));

        let mut buf = vec![0u8; encoded_frame_len(&frame).expect("test must succeed")];
        let encoded_len = encode_frame(&mut buf, &frame).expect("test must succeed");

        let (decoded, decoded_len) = decode_frame(&buf[..encoded_len]).expect("test must succeed");

        assert_eq!(encoded_len, decoded_len);

        if let Frame::Goaway(goaway) = decoded {
            assert_eq!(goaway.id(), id, "GOAWAY id mismatch");
        } else {
            panic!("Expected GOAWAY frame");
        }
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// Unknown Frame Properties
// =============================================================================

/// Property: 未知のフレームタイプはペイロードとともに保存される
///
/// VarInt 全領域 (0..=2^62-1, RFC 9000 Section 16) から既知タイプと
/// HTTP/2 専用 ID (RFC 9114 Section 7.2 / Section 11.2.1 Table 2) を除いて生成する。
#[test]
fn prop_unknown_frame_preserved() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_FRAME_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        // 既知 / HTTP/2 専用フレームタイプを除外
        let unknown_type = noprop::sample_with_rejection(ctx, 256, |ctx| {
            let t = noprop::sample_u64_in(ctx, 0..(1u64 << 62));
            (FrameType::from_type(t).is_none() && !FrameType::is_http2_only(t)).then_some(t)
        });
        let payload = valid_payload(ctx);
        let frame_type = VarInt::new(unknown_type).expect("test must succeed");
        let unknown = UnknownFrame::new(frame_type, payload.clone())
            .expect("生成範囲は既知タイプでも HTTP/2 専用でもない");
        let frame = Frame::Unknown(unknown);

        let mut buf = vec![0u8; encoded_frame_len(&frame).expect("test must succeed")];
        let encoded_len = encode_frame(&mut buf, &frame).expect("test must succeed");

        let (decoded, decoded_len) = decode_frame(&buf[..encoded_len]).expect("test must succeed");

        assert_eq!(encoded_len, decoded_len);

        if let Frame::Unknown(decoded_unknown) = decoded {
            assert_eq!(decoded_unknown.frame_type(), frame_type);
            assert_eq!(decoded_unknown.payload(), payload.as_slice());
        } else {
            panic!("Expected Unknown frame");
        }
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// Frame Length Properties
// =============================================================================

/// Property: encoded_frame_len は常に実際のエンコード長と一致
#[test]
fn prop_encoded_frame_len_accurate() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_FRAME_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let payload = valid_payload(ctx);
        let frame = Frame::Data(DataPayload::new(payload));

        let predicted_len = encoded_frame_len(&frame).expect("test must succeed");
        let mut buf = vec![0u8; predicted_len + 100]; // 余裕を持たせる
        let actual_len = encode_frame(&mut buf, &frame).expect("test must succeed");

        assert_eq!(
            predicted_len, actual_len,
            "encoded_frame_len prediction mismatch"
        );
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// Construct-Time Validation Consistency
// =============================================================================

/// Property: GoawayPayload::from_static と new(VarInt::new(id)) が同じ値を返す
/// (`const fn` 検査とランタイム検査のロジック一致)
#[test]
fn prop_goaway_from_static_matches_new() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_FRAME_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let value = noprop::sample_u64_in(ctx, 0..=VarInt::MAX.get());
        let via_new = GoawayPayload::new(VarInt::new(value).expect("test must succeed"));
        let via_static = GoawayPayload::from_static(value);
        assert_eq!(via_new, via_static);
        Ok(())
    })?;
    Ok(())
}
