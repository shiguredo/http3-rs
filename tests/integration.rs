//! 統合テスト: クライアント・サーバー間の通信シミュレーション

use shiguredo_http3::{ClientConnection, Event, Header, ServerConnection, Settings, VarInt};

fn vi(value: u64) -> VarInt {
    VarInt::new(value).expect("test must succeed")
}

/// クライアントからサーバーへのリクエスト・レスポンス交換をシミュレート
#[test]
fn test_client_server_exchange() -> Result<(), Box<dyn std::error::Error>> {
    // クライアントとサーバーを作成
    let mut client = ClientConnection::with_default_settings();
    let mut server = ServerConnection::with_default_settings();

    // 制御ストリーム ID を設定
    // クライアント: 単方向ストリーム ID 2 (0b10)
    // サーバー: 単方向ストリーム ID 3 (0b11)
    client.set_control_stream_id(2)?;
    server.set_control_stream_id(3)?;

    // === Phase 1: 制御ストリームの SETTINGS 交換 ===

    // クライアントの制御ストリームデータを取得
    let (client_ctrl_data, _) = client.take_stream_data(2).expect("test must succeed");

    // サーバーの制御ストリームデータを取得
    let (server_ctrl_data, _) = server.take_stream_data(3).expect("test must succeed");

    // 制御ストリームデータを相互に送信
    server.feed_stream(2, &client_ctrl_data, false)?;
    client.feed_stream(3, &server_ctrl_data, false)?;

    // SETTINGS イベントを確認
    let client_event = client.poll_event()?.expect("test must succeed");
    assert!(matches!(client_event, Event::SettingsReceived { .. }));

    let server_event = server.poll_event()?.expect("test must succeed");
    assert!(matches!(server_event, Event::SettingsReceived { .. }));

    // === Phase 2: リクエスト送信 ===

    // クライアントがリクエストを送信
    let request_headers = vec![
        Header::new(b":method", b"GET").expect("test must succeed"),
        Header::new(b":path", b"/index.html").expect("test must succeed"),
        Header::new(b":scheme", b"https").expect("test must succeed"),
        Header::new(b":authority", b"example.com").expect("test must succeed"),
        Header::new(b"user-agent", b"shiguredo-http3/1.0").expect("test must succeed"),
    ];

    let stream_id = client.send_request(&request_headers, true)?;
    assert_eq!(stream_id, 0); // 最初のクライアント双方向ストリーム

    // リクエストデータを取得
    let (request_data, _request_fin) = client
        .take_stream_data(stream_id)
        .expect("test must succeed");

    // データ消費後、FIN を確認
    let fin_check = client.get_stream_data(stream_id);
    assert!(fin_check.is_none() || fin_check.expect("test must succeed").1); // データなし or FIN=true

    // サーバーがリクエストを受信
    server.feed_stream(stream_id, &request_data, true)?;

    // サーバー側でヘッダーイベントを確認
    let mut received_headers = Vec::new();
    loop {
        match server.poll_event()? {
            Some(Event::HeadersBegin { stream_id: sid }) => {
                assert_eq!(sid, stream_id);
            }
            Some(Event::Header {
                stream_id: sid,
                name,
                value,
            }) => {
                assert_eq!(sid, stream_id);
                received_headers.push((name, value));
            }
            Some(Event::HeadersEnd { stream_id: sid }) => {
                assert_eq!(sid, stream_id);
                break;
            }
            Some(Event::StreamEnd { .. }) => break,
            None => break,
            _ => {}
        }
    }

    // リクエストヘッダーを検証
    assert!(
        received_headers
            .iter()
            .any(|(n, v)| n == b":method" && v == b"GET")
    );
    assert!(
        received_headers
            .iter()
            .any(|(n, v)| n == b":path" && v == b"/index.html")
    );

    // === Phase 3: レスポンス送信 ===

    // サーバーがレスポンスを送信
    let response_headers = vec![
        Header::new(b":status", b"200").expect("test must succeed"),
        Header::new(b"content-type", b"text/html").expect("test must succeed"),
        Header::new(b"content-length", b"13").expect("test must succeed"),
    ];

    server.send_response(stream_id, &response_headers, false)?;
    server.send_body(stream_id, b"Hello, World!", true)?;

    // レスポンスデータを取得
    let (response_data, _response_fin) = server
        .take_stream_data(stream_id)
        .expect("test must succeed");

    // クライアントがレスポンスを受信
    client.feed_stream(stream_id, &response_data, true)?;

    // クライアント側でレスポンスイベントを確認
    let mut response_headers_received = Vec::new();
    let mut body_data = Vec::new();

    loop {
        match client.poll_event()? {
            Some(Event::HeadersBegin { .. }) => {}
            Some(Event::Header { name, value, .. }) => {
                response_headers_received.push((name, value));
            }
            Some(Event::HeadersEnd { .. }) => {}
            Some(Event::Data { data, .. }) => {
                body_data.extend(data);
            }
            Some(Event::StreamEnd { .. }) => break,
            None => break,
            _ => {}
        }
    }

    // レスポンスを検証
    assert!(
        response_headers_received
            .iter()
            .any(|(n, v)| n == b":status" && v == b"200")
    );
    assert_eq!(body_data, b"Hello, World!");

    Ok(())
}

/// GOAWAY フレームの送受信をテスト
#[test]
fn test_goaway_exchange() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = ClientConnection::with_default_settings();
    let mut server = ServerConnection::with_default_settings();

    client.set_control_stream_id(2)?;
    server.set_control_stream_id(3)?;

    // 制御ストリームを交換
    let (client_ctrl, _) = client.take_stream_data(2).expect("test must succeed");
    let (server_ctrl, _) = server.take_stream_data(3).expect("test must succeed");

    server.feed_stream(2, &client_ctrl, false)?;
    client.feed_stream(3, &server_ctrl, false)?;

    // SETTINGS を消費
    let _ = client.poll_event()?;
    let _ = server.poll_event()?;

    // サーバーが GOAWAY を送信
    server.send_goaway(vi(0))?;

    // GOAWAY データを取得して送信
    let (goaway_data, _) = server.take_stream_data(3).expect("test must succeed");

    client.feed_stream(3, &goaway_data, false)?;

    // クライアントが GOAWAY を受信
    let event = client.poll_event()?.expect("test must succeed");
    let Event::GoawayReceived { id } = event else {
        panic!("expected Event::GoawayReceived, got {event:?}");
    };
    assert_eq!(id, vi(0));

    Ok(())
}

/// GOAWAY の単調減少制約 (RFC 9114 Section 5.2: "MUST NOT increase the value")
#[test]
fn test_goaway_monotonic_decrease() -> Result<(), Box<dyn std::error::Error>> {
    let mut server = ServerConnection::with_default_settings();
    server.set_control_stream_id(3)?;

    // 初回 GOAWAY (stream id 8 = client-initiated bidi 3 番目)
    server.send_goaway(vi(8))?;

    // 同一 ID の再送は許可される
    server.send_goaway(vi(8))?;

    // より小さな ID への減少も許可される
    server.send_goaway(vi(4))?;

    // より大きな ID への増加は IdError で拒否される
    let result = server.send_goaway(vi(8));
    assert!(result.is_err(), "GOAWAY id increase must be rejected");

    Ok(())
}

/// 複数リクエストの並行処理をテスト
#[test]
fn test_multiple_requests() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = ClientConnection::with_default_settings();
    let mut server = ServerConnection::with_default_settings();

    client.set_control_stream_id(2)?;
    server.set_control_stream_id(3)?;

    // 制御ストリームを交換
    let (client_ctrl, _) = client.take_stream_data(2).expect("test must succeed");
    let (server_ctrl, _) = server.take_stream_data(3).expect("test must succeed");

    server.feed_stream(2, &client_ctrl, false)?;
    client.feed_stream(3, &server_ctrl, false)?;

    // SETTINGS を消費
    let _ = client.poll_event()?;
    let _ = server.poll_event()?;

    // 複数のリクエストを送信
    let headers1 = vec![
        Header::new(b":method", b"GET").expect("test must succeed"),
        Header::new(b":path", b"/page1").expect("test must succeed"),
        Header::new(b":scheme", b"https").expect("test must succeed"),
        Header::new(b":authority", b"example.com").expect("test must succeed"),
    ];

    let headers2 = vec![
        Header::new(b":method", b"GET").expect("test must succeed"),
        Header::new(b":path", b"/page2").expect("test must succeed"),
        Header::new(b":scheme", b"https").expect("test must succeed"),
        Header::new(b":authority", b"example.com").expect("test must succeed"),
    ];

    let stream_id1 = client.send_request(&headers1, true)?;
    let stream_id2 = client.send_request(&headers2, true)?;

    // ストリーム ID は 4 ずつ増加
    assert_eq!(stream_id1, 0);
    assert_eq!(stream_id2, 4);

    // リクエストデータを取得
    let (data1, _) = client
        .take_stream_data(stream_id1)
        .expect("test must succeed");
    let (data2, _) = client
        .take_stream_data(stream_id2)
        .expect("test must succeed");

    // サーバーに送信
    server.feed_stream(stream_id1, &data1, true)?;
    server.feed_stream(stream_id2, &data2, true)?;

    // 両方のリクエストが受信されたことを確認
    let mut stream_ids = Vec::new();
    for event in server.drain_events()? {
        if let Event::HeadersBegin { stream_id } = event {
            stream_ids.push(stream_id);
        }
    }

    assert!(stream_ids.contains(&stream_id1));
    assert!(stream_ids.contains(&stream_id2));

    Ok(())
}

/// カスタム設定での接続をテスト
#[test]
fn test_custom_settings() {
    let settings = Settings::new()
        .max_field_section_size(vi(8192))
        .qpack_max_table_capacity(vi(4096))
        .qpack_blocked_streams(vi(100));

    let mut client = ClientConnection::new(settings);
    client.set_control_stream_id(2).expect("test must succeed");

    // ローカル設定を確認
    let local = client.local_settings();
    assert_eq!(local.max_field_section_size, Some(vi(8192)));
    assert_eq!(local.qpack_max_table_capacity, Some(vi(4096)));
    assert_eq!(local.qpack_blocked_streams, Some(vi(100)));
}
