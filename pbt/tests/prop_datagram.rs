//! Property-Based Testing for WebTransport Datagram
//! (RFC 9297, draft-ietf-webtrans-http3-15 Section 4.5)

use proptest::prelude::*;
use shiguredo_http3::VarInt;
use shiguredo_http3::webtransport::Datagram;

// =============================================================================
// Strategy ヘルパー
// =============================================================================

prop_compose! {
    /// 有効なセッション ID (client-initiated bidirectional stream ID: 4 の倍数)
    fn valid_session_id()(id in 0u64..250_000) -> u64 {
        id * 4
    }
}

prop_compose! {
    /// 有効なペイロードを生成
    fn valid_payload()(
        len in 0usize..512,
    )(
        data in prop::collection::vec(any::<u8>(), len)
    ) -> Vec<u8> {
        data
    }
}

// =============================================================================
// (a) Datagram encode/decode ラウンドトリップ
// =============================================================================

proptest! {
    /// Property: 有効な session_id と任意のペイロードで encode → decode が元と一致する
    #[test]
    fn prop_datagram_roundtrip(
        session_id in valid_session_id(),
        payload in valid_payload(),
    ) {
        let datagram = Datagram::new(session_id, payload.clone()).expect("test must succeed");

        let mut buf = Vec::new();
        datagram.encode(&mut buf);

        let (decoded, consumed) = Datagram::decode(&buf)
            .expect("decode should succeed for valid encoded datagram");

        prop_assert_eq!(
            decoded.session_id, session_id,
            "session_id が一致しない"
        );
        prop_assert_eq!(
            decoded.payload, payload,
            "payload が一致しない"
        );
        prop_assert_eq!(
            consumed,
            buf.len(),
            "消費バイト数がバッファ長と不一致"
        );
    }
}

// =============================================================================
// (b) quarter_stream_id == session_id / 4
// =============================================================================

proptest! {
    /// Property: quarter_stream_id() は常に session_id / 4 を返す
    #[test]
    fn prop_quarter_stream_id_equals_session_id_div_4(
        session_id in valid_session_id(),
    ) {
        let datagram = Datagram::new(session_id, vec![]).expect("test must succeed");
        let qsi = datagram.quarter_stream_id();

        prop_assert_eq!(
            qsi,
            session_id / 4,
            "quarter_stream_id != session_id / 4"
        );
    }
}

// =============================================================================
// (c) decode が返す session_id は常に 4 の倍数
// =============================================================================

proptest! {
    /// Property: decode で復元された session_id は常に 4 の倍数である
    /// (quarter_stream_id * 4 で復元されるため)
    #[test]
    fn prop_decoded_session_id_always_multiple_of_4(
        session_id in valid_session_id(),
        payload in valid_payload(),
    ) {
        let datagram = Datagram::new(session_id, payload).expect("test must succeed");

        let mut buf = Vec::new();
        datagram.encode(&mut buf);

        let (decoded, _) = Datagram::decode(&buf)
            .expect("decode should succeed");

        prop_assert!(
            decoded.session_id % 4 == 0,
            "decode された session_id ({}) が 4 の倍数でない",
            decoded.session_id,
        );
    }

    /// Property: 任意の varint 値を Quarter Stream ID として直接エンコードした場合でも
    ///           decode の結果の session_id は 4 の倍数
    #[test]
    fn prop_arbitrary_qsi_decodes_to_multiple_of_4(
        qsi in 0u64..=(VarInt::MAX.get() / 4),
        payload in valid_payload(),
    ) {
        // Quarter Stream ID を直接エンコードする
        let mut buf = Vec::new();
        shiguredo_http3::varint::encode_into_vec(
            &mut buf,
            shiguredo_http3::VarInt::new(qsi).expect("test must succeed"),
        );
        buf.extend_from_slice(&payload);

        let (decoded, consumed) = Datagram::decode(&buf)
            .expect("decode should succeed for valid varint + payload");

        prop_assert_eq!(
            decoded.session_id,
            qsi * 4,
            "session_id ({}) != qsi * 4 ({})",
            decoded.session_id,
            qsi * 4,
        );
        prop_assert_eq!(consumed, buf.len());
    }
}
