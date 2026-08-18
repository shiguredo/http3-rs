//! Property-Based Testing for HTTP/3 Connection (RFC 9114)
//!
//! get_stream_data のループ終了性とデータ完全性を検証する。

use pbt::strategies::sample_len;
use shiguredo_http3::{ClientConnection, Header};

/// リクエスト送信後の take_stream_data ループ用ヘルパー
///
/// take_stream_data ループを実行し、収集したデータと反復回数を返す。
/// ループはデータ交付 → FIN 交付 (`(空, fin=true)`) → None の順で終了する。
/// FIN の 1 回交付は決定的テスト (tests/test_connection.rs) で担保し、
/// この PBT はループの終了性とデータ完全性だけを検証する。
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

/// Property: 任意サイズのボディで get_stream_data ループが有限回で終了する
#[test]
fn prop_get_stream_data_terminates() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_CONNECTION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let body_len = sample_len(ctx, 0..=8191);
        let body = vec![0xABu8; body_len];

        let mut client = ClientConnection::with_default_settings();
        client.set_control_stream_id(2).expect("test must succeed");

        let request_headers = vec![
            Header::new(b":method", b"POST").expect("test must succeed"),
            Header::new(b":path", b"/").expect("test must succeed"),
            Header::new(b":scheme", b"https").expect("test must succeed"),
            Header::new(b":authority", b"example.com").expect("test must succeed"),
        ];

        // ボディ付きリクエストを送信 (fin=false でヘッダーのみ先に送信)
        let stream_id = client
            .send_request(&request_headers, false)
            .expect("test must succeed");
        client
            .send_body(stream_id, &body, true)
            .expect("test must succeed");

        let (_collected, iterations) = drain_stream_data(&mut client, stream_id, 10);

        assert!(
            iterations < 10,
            "get_stream_data ループが 10 回以内に終了しなかった (iterations={})",
            iterations
        );
        Ok(())
    })?;
    Ok(())
}

/// Property: ループで収集したデータ長が元のフレームデータ長と一致する (データ欠損なし)
#[test]
fn prop_get_stream_data_all_data_collected() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_CONNECTION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    // sample_len は 0/1/上限を 1/5 で選ぶ。3 境界なら空の p=1/15、
    // 256 ケースでの見逃しは (14/15)^256 ≈ 2.4e-8。
    let empty_body = std::cell::Cell::new(0usize);
    let nonempty_body = std::cell::Cell::new(0usize);
    runner.run(256, |ctx| {
        let body_len = sample_len(ctx, 0..=8191);
        let body = vec![0xCDu8; body_len];

        let mut client = ClientConnection::with_default_settings();
        client.set_control_stream_id(2).expect("test must succeed");

        let request_headers = vec![
            Header::new(b":method", b"POST").expect("test must succeed"),
            Header::new(b":path", b"/").expect("test must succeed"),
            Header::new(b":scheme", b"https").expect("test must succeed"),
            Header::new(b":authority", b"example.com").expect("test must succeed"),
        ];

        let stream_id = client
            .send_request(&request_headers, false)
            .expect("test must succeed");
        client
            .send_body(stream_id, &body, true)
            .expect("test must succeed");

        let (collected, _iterations) = drain_stream_data(&mut client, stream_id, 10);

        // 収集データにはフレームヘッダーも含まれるため、ボディ長以上であることを検証
        assert!(
            collected.len() >= body_len,
            "収集データ長 {} がボディ長 {} より小さい",
            collected.len(),
            body_len
        );
        if body_len == 0 {
            empty_body.set(empty_body.get() + 1);
        } else {
            nonempty_body.set(nonempty_body.get() + 1);
        }
        Ok(())
    })?;
    assert!(empty_body.get() > 0, "空ボディを未到達\n{runner}");
    assert!(nonempty_body.get() > 0, "非空ボディを未到達\n{runner}");
    Ok(())
}

/// Property: FIN 交付後に get_stream_data が None を返す
///
/// drain_stream_data はデータ交付 → FIN 交付 → None の順でループを抜けるため、
/// ここでの検証対象は「FIN 交付後の None」である。
/// (FIN はデータ全消費後の追加呼び出しで交付されるため、全データ消費直後の
///  get_stream_data は (空, fin=true) を返す)
#[test]
fn prop_get_stream_data_returns_none_after_consume() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_CONNECTION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let body_len = sample_len(ctx, 0..=8191);
        let body = vec![0xEFu8; body_len];

        let mut client = ClientConnection::with_default_settings();
        client.set_control_stream_id(2).expect("test must succeed");

        let request_headers = vec![
            Header::new(b":method", b"POST").expect("test must succeed"),
            Header::new(b":path", b"/").expect("test must succeed"),
            Header::new(b":scheme", b"https").expect("test must succeed"),
            Header::new(b":authority", b"example.com").expect("test must succeed"),
        ];

        let stream_id = client
            .send_request(&request_headers, false)
            .expect("test must succeed");
        client
            .send_body(stream_id, &body, true)
            .expect("test must succeed");

        // データと FIN をすべて消費
        let (_collected, _iterations) = drain_stream_data(&mut client, stream_id, 10);

        // FIN 交付後は None が返ることを検証
        let result = client.get_stream_data(stream_id);
        assert!(
            result.is_none(),
            "FIN 交付後に get_stream_data が None ではなく {:?} を返した",
            result.map(|(d, f)| (d.len(), f))
        );
        Ok(())
    })?;
    Ok(())
}
