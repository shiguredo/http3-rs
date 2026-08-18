//! Property-Based Testing for WebTransport Datagram
//! (RFC 9297, draft-ietf-webtrans-http3-15 Section 4.5)

use pbt::strategies::{sample_len, sample_varint_raw_in};
use shiguredo_http3::VarInt;
use shiguredo_http3::webtransport::Datagram;

// =============================================================================
// 生成ヘルパー
// =============================================================================

/// 有効なセッション ID (client-initiated bidirectional stream ID: 4 の倍数)
fn valid_session_id(ctx: &mut noprop::TestCaseContext) -> u64 {
    sample_varint_raw_in(ctx, 0..=249_999) * 4
}

/// 有効なペイロードを生成
fn valid_payload(ctx: &mut noprop::TestCaseContext) -> Vec<u8> {
    let len = sample_len(ctx, 0..=511);
    noprop::sample_bytes_vec(ctx, len)
}

// =============================================================================
// (a) Datagram encode/decode ラウンドトリップ
// =============================================================================

/// Property: 有効な session_id と任意のペイロードで encode → decode が元と一致する
#[test]
fn prop_datagram_roundtrip() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_DATAGRAM_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let session_id = valid_session_id(ctx);
        let payload = valid_payload(ctx);
        let datagram = Datagram::new(session_id, payload.clone()).expect("test must succeed");

        let mut buf = Vec::new();
        datagram.encode(&mut buf);

        let (decoded, consumed) =
            Datagram::decode(&buf).expect("decode should succeed for valid encoded datagram");

        assert_eq!(decoded.session_id, session_id, "session_id が一致しない");
        assert_eq!(decoded.payload, payload, "payload が一致しない");
        assert_eq!(consumed, buf.len(), "消費バイト数がバッファ長と不一致");
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// (b) quarter_stream_id == session_id / 4
// =============================================================================

/// Property: quarter_stream_id() は常に session_id / 4 を返す
#[test]
fn prop_quarter_stream_id_equals_session_id_div_4() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_DATAGRAM_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let session_id = valid_session_id(ctx);
        let datagram = Datagram::new(session_id, vec![]).expect("test must succeed");
        let qsi = datagram.quarter_stream_id();

        assert_eq!(qsi, session_id / 4, "quarter_stream_id != session_id / 4");
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// (c) decode が返す session_id は常に 4 の倍数
// =============================================================================

/// Property: decode で復元された session_id は常に 4 の倍数である
/// (quarter_stream_id * 4 で復元されるため)
#[test]
fn prop_decoded_session_id_always_multiple_of_4() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_DATAGRAM_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let session_id = valid_session_id(ctx);
        let payload = valid_payload(ctx);
        let datagram = Datagram::new(session_id, payload).expect("test must succeed");

        let mut buf = Vec::new();
        datagram.encode(&mut buf);

        let (decoded, _) = Datagram::decode(&buf).expect("decode should succeed");

        assert!(
            decoded.session_id % 4 == 0,
            "decode された session_id ({}) が 4 の倍数でない",
            decoded.session_id,
        );
        Ok(())
    })?;
    Ok(())
}

/// Property: 任意の varint 値を Quarter Stream ID として直接エンコードした場合でも
///           decode の結果の session_id は 4 の倍数
#[test]
fn prop_arbitrary_qsi_decodes_to_multiple_of_4() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_DATAGRAM_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let qsi = sample_varint_raw_in(ctx, 0..=VarInt::MAX.get() / 4);
        let payload = valid_payload(ctx);
        // Quarter Stream ID を直接エンコードする
        let mut buf = Vec::new();
        shiguredo_http3::varint::encode_into_vec(
            &mut buf,
            shiguredo_http3::VarInt::new(qsi).expect("test must succeed"),
        );
        buf.extend_from_slice(&payload);

        let (decoded, consumed) =
            Datagram::decode(&buf).expect("decode should succeed for valid varint + payload");

        assert_eq!(
            decoded.session_id,
            qsi * 4,
            "session_id ({}) != qsi * 4 ({})",
            decoded.session_id,
            qsi * 4,
        );
        assert_eq!(consumed, buf.len());
        Ok(())
    })?;
    Ok(())
}
