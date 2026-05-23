//! Property-Based Testing for HTTP/3 Connection (RFC 9114)
//!
//! get_stream_data のループ終了性とデータ完全性を検証する。

use proptest::prelude::*;
use shiguredo_http3::{ClientConnection, Header};

/// リクエスト送信後の take_stream_data ループ用ヘルパー
///
/// take_stream_data ループを実行し、収集したデータと反復回数を返す。
fn drain_stream_data(
    conn: &mut ClientConnection,
    stream_id: u64,
    max_iterations: usize,
) -> (Vec<u8>, usize) {
    let mut collected = Vec::new();
    let mut iterations = 0;

    while let Some((chunk, _fin)) = conn.take_stream_data(stream_id) {
        collected.extend_from_slice(&chunk);
        iterations += 1;
        if iterations >= max_iterations {
            break;
        }
    }

    (collected, iterations)
}

// =============================================================================
// get_stream_data Termination Properties
// =============================================================================

proptest! {
    /// Property: 任意サイズのボディで get_stream_data ループが有限回で終了する
    #[test]
    fn prop_get_stream_data_terminates(body_len in 0usize..8192) {
        let body = vec![0xABu8; body_len];

        let mut client = ClientConnection::with_default_settings();
        client.set_control_stream_id(2).unwrap();

        let request_headers = vec![
            Header::new(b":method", b"POST").unwrap(),
            Header::new(b":path", b"/").unwrap(),
            Header::new(b":scheme", b"https").unwrap(),
            Header::new(b":authority", b"example.com").unwrap(),
        ];

        // ボディ付きリクエストを送信 (fin=false でヘッダーのみ先に送信)
        let stream_id = client.send_request(&request_headers, false).unwrap();
        client.send_body(stream_id, &body, true).unwrap();

        let (_collected, iterations) = drain_stream_data(&mut client, stream_id, 10);

        prop_assert!(
            iterations < 10,
            "get_stream_data ループが 10 回以内に終了しなかった (iterations={})",
            iterations
        );
    }

    /// Property: ループで収集したデータ長が元のフレームデータ長と一致する (データ欠損なし)
    #[test]
    fn prop_get_stream_data_all_data_collected(body_len in 0usize..8192) {
        let body = vec![0xCDu8; body_len];

        let mut client = ClientConnection::with_default_settings();
        client.set_control_stream_id(2).unwrap();

        let request_headers = vec![
            Header::new(b":method", b"POST").unwrap(),
            Header::new(b":path", b"/").unwrap(),
            Header::new(b":scheme", b"https").unwrap(),
            Header::new(b":authority", b"example.com").unwrap(),
        ];

        let stream_id = client.send_request(&request_headers, false).unwrap();
        client.send_body(stream_id, &body, true).unwrap();

        let (collected, _iterations) = drain_stream_data(&mut client, stream_id, 10);

        // 収集データにはフレームヘッダーも含まれるため、ボディ長以上であることを検証
        prop_assert!(
            collected.len() >= body_len,
            "収集データ長 {} がボディ長 {} より小さい",
            collected.len(),
            body_len
        );
    }

    /// Property: 全データ消費後に get_stream_data が None を返す
    #[test]
    fn prop_get_stream_data_returns_none_after_consume(body_len in 0usize..8192) {
        let body = vec![0xEFu8; body_len];

        let mut client = ClientConnection::with_default_settings();
        client.set_control_stream_id(2).unwrap();

        let request_headers = vec![
            Header::new(b":method", b"POST").unwrap(),
            Header::new(b":path", b"/").unwrap(),
            Header::new(b":scheme", b"https").unwrap(),
            Header::new(b":authority", b"example.com").unwrap(),
        ];

        let stream_id = client.send_request(&request_headers, false).unwrap();
        client.send_body(stream_id, &body, true).unwrap();

        // 全データを消費
        let (_collected, _iterations) = drain_stream_data(&mut client, stream_id, 10);

        // 消費後は None が返ることを検証
        let result = client.get_stream_data(stream_id);
        prop_assert!(
            result.is_none(),
            "全データ消費後に get_stream_data が None ではなく {:?} を返した",
            result.map(|(d, f)| (d.len(), f))
        );
    }
}
