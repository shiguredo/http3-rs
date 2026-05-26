//! StreamHeader / Stream ID のプロパティ
//! (draft-ietf-webtrans-http3-15 Section 4)

use proptest::prelude::*;
use shiguredo_http3::webtransport::{StreamHeader, stream_type};

prop_compose! {
    /// 有効なセッション ID (client-initiated bidirectional stream ID: id % 4 == 0)
    fn valid_session_id()(id in 0u64..250_000) -> u64 {
        id * 4
    }
}

prop_compose! {
    /// 不正な単方向ストリームタイプ (0x54 以外)
    fn invalid_unidirectional_type()(
        value in (0u64..0x54).prop_union(0x55u64..0x1000)
    ) -> u64 {
        value
    }
}

prop_compose! {
    /// 不正な双方向シグナル値 (0x41 以外)
    fn invalid_bidirectional_signal()(
        value in (0u64..0x41).prop_union(0x42u64..0x1000)
    ) -> u64 {
        value
    }
}

/// 可変長整数をエンコード
fn encode_varint_for_test(buf: &mut Vec<u8>, value: u64) {
    shiguredo_http3::varint::encode_into_vec(
        buf,
        shiguredo_http3::VarInt::new(value).expect("value fits in VarInt"),
    );
}

// =============================================================================
// StreamHeader ラウンドトリップ (draft-ietf-webtrans-http3-15 Section 4)
// =============================================================================

proptest! {
    /// Property: 単方向ストリームヘッダーのラウンドトリップ
    #[test]
    fn prop_unidirectional_header_roundtrip(session_id in valid_session_id()) {
        let header = StreamHeader::new(session_id).expect("valid session_id");

        let mut buf = Vec::new();
        header.encode_unidirectional(&mut buf);

        let (decoded, consumed) = StreamHeader::decode_unidirectional(&buf).expect("valid encoded header");
        prop_assert_eq!(consumed, buf.len());
        prop_assert_eq!(decoded.session_id, session_id);
    }

    /// Property: 双方向ストリームヘッダーのラウンドトリップ
    #[test]
    fn prop_bidirectional_header_roundtrip(session_id in valid_session_id()) {
        let header = StreamHeader::new(session_id).expect("valid session_id");

        let mut buf = Vec::new();
        header.encode_bidirectional(&mut buf);

        let (decoded, consumed) = StreamHeader::decode_bidirectional(&buf).expect("valid encoded header");
        prop_assert_eq!(consumed, buf.len());
        prop_assert_eq!(decoded.session_id, session_id);
    }

    /// Property: 単方向ストリームヘッダーのフォーマット検証
    #[test]
    fn prop_unidirectional_header_format(session_id in valid_session_id()) {
        let header = StreamHeader::new(session_id).expect("valid session_id");

        let mut buf = Vec::new();
        header.encode_unidirectional(&mut buf);

        // 先頭は Stream Type (0x54) の varint エンコード
        // 0x54 = 84 は 2 バイト varint: 0x40 | (84 >> 8), 84 & 0xff = 0x40, 0x54
        prop_assert_eq!(buf[0], 0x40, "First byte should be 0x40 for 2-byte varint");
        prop_assert_eq!(buf[1], 0x54, "Second byte should be 0x54 (stream type)");
    }

    /// Property: 双方向ストリームヘッダーのフォーマット検証
    #[test]
    fn prop_bidirectional_header_format(session_id in valid_session_id()) {
        let header = StreamHeader::new(session_id).expect("valid session_id");

        let mut buf = Vec::new();
        header.encode_bidirectional(&mut buf);

        // 先頭は Signal Value (0x41) の varint エンコード
        // 0x41 = 65 は 2 バイト varint: 0x40 | (65 >> 8), 65 & 0xff = 0x40, 0x41
        prop_assert_eq!(buf[0], 0x40, "First byte should be 0x40 for 2-byte varint");
        prop_assert_eq!(buf[1], 0x41, "Second byte should be 0x41 (signal value)");
    }

    /// Property: encoded_size が実際のエンコード長と一致
    #[test]
    fn prop_encoded_size_matches_actual(session_id in valid_session_id()) {
        let header = StreamHeader::new(session_id).expect("valid session_id");

        let predicted_size = header.encoded_size();

        let mut buf_uni = Vec::new();
        header.encode_unidirectional(&mut buf_uni);

        let mut buf_bidi = Vec::new();
        header.encode_bidirectional(&mut buf_bidi);

        // 単方向と双方向は Signal/Type 部分が同じサイズなので同じ長さになる
        prop_assert_eq!(buf_uni.len(), buf_bidi.len());
        prop_assert_eq!(predicted_size, buf_uni.len());
    }
}

// =============================================================================
// Stream ID 分類 (draft-ietf-webtrans-http3-15 Section 4)
// =============================================================================

proptest! {
    /// Property: クライアント開始/サーバー開始の判定は相互排他的
    #[test]
    fn prop_stream_id_client_server_initiated(stream_id in any::<u64>()) {
        let is_client = stream_type::is_client_initiated(stream_id);
        let is_server = stream_type::is_server_initiated(stream_id);

        prop_assert!(is_client != is_server, "Stream ID must be either client or server initiated");
    }

    /// Property: 双方向/単方向の判定は相互排他的
    #[test]
    fn prop_stream_id_bidirectional_unidirectional(stream_id in any::<u64>()) {
        let is_bidi = stream_type::is_bidirectional(stream_id);
        let is_uni = stream_type::is_unidirectional(stream_id);

        prop_assert!(is_bidi != is_uni, "Stream ID must be either bidirectional or unidirectional");
    }

    /// Property: 全ストリームタイプの組み合わせ (4 パターン)
    #[test]
    fn prop_stream_id_all_combinations(base_id in 0u64..1_000_000) {
        // クライアント開始双方向 (0b00)
        let id_00 = base_id * 4;
        prop_assert!(stream_type::is_client_initiated(id_00));
        prop_assert!(stream_type::is_bidirectional(id_00));

        // サーバー開始双方向 (0b01)
        let id_01 = base_id * 4 + 1;
        prop_assert!(stream_type::is_server_initiated(id_01));
        prop_assert!(stream_type::is_bidirectional(id_01));

        // クライアント開始単方向 (0b10)
        let id_10 = base_id * 4 + 2;
        prop_assert!(stream_type::is_client_initiated(id_10));
        prop_assert!(stream_type::is_unidirectional(id_10));

        // サーバー開始単方向 (0b11)
        let id_11 = base_id * 4 + 3;
        prop_assert!(stream_type::is_server_initiated(id_11));
        prop_assert!(stream_type::is_unidirectional(id_11));
    }
}

// =============================================================================
// 不正な StreamHeader のデコード
// =============================================================================

proptest! {
    /// Property: 不正な stream type では単方向デコード失敗
    #[test]
    fn prop_unidirectional_header_invalid_type_fails(
        invalid_type in invalid_unidirectional_type(),
        session_id in valid_session_id(),
    ) {
        let mut buf = Vec::new();
        encode_varint_for_test(&mut buf, invalid_type);
        encode_varint_for_test(&mut buf, session_id);

        let result = StreamHeader::decode_unidirectional(&buf);
        prop_assert!(result.is_none(), "Invalid stream type should fail decode");
    }

    /// Property: 不正な signal value では双方向デコード失敗
    #[test]
    fn prop_bidirectional_header_invalid_type_fails(
        invalid_signal in invalid_bidirectional_signal(),
        session_id in valid_session_id(),
    ) {
        let mut buf = Vec::new();
        encode_varint_for_test(&mut buf, invalid_signal);
        encode_varint_for_test(&mut buf, session_id);

        let result = StreamHeader::decode_bidirectional(&buf);
        prop_assert!(result.is_none(), "Invalid signal value should fail decode");
    }

    /// Property: 空バッファでは StreamHeader デコード失敗
    #[test]
    fn prop_stream_header_empty_buffer_fails(_dummy in Just(())) {
        prop_assert!(StreamHeader::decode_unidirectional(&[]).is_none());
        prop_assert!(StreamHeader::decode_bidirectional(&[]).is_none());
    }
}
