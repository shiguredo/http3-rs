use shiguredo_http3::{
    ClientConnection, Error, ErrorCode, Event, Header, ServerConnection, Settings, VarInt,
};

/// GOAWAY テスト用のクライアント・サーバーペアを構築する
fn setup_pair() -> (ClientConnection, ServerConnection) {
    let mut client = ClientConnection::new(Settings::default());
    client.set_control_stream_id(2).expect("test must succeed");
    let mut server = ServerConnection::new(Settings::default());
    server.set_control_stream_id(3).expect("test must succeed");

    // サーバー制御ストリームデータ（ストリームタイプ + SETTINGS）をクライアントに feed
    let (server_ctrl, _) = server.take_stream_data(3).expect("test must succeed");
    client
        .feed_stream(3, &server_ctrl, false)
        .expect("test must succeed");

    // SETTINGS イベントを消費
    while let Some(_ev) = client.poll_event().expect("test must succeed") {}

    (client, server)
}

fn request_headers() -> Vec<Header> {
    vec![
        Header::new(b":method", b"GET").expect("test must succeed"),
        Header::new(b":path", b"/").expect("test must succeed"),
        Header::new(b":scheme", b"https").expect("test must succeed"),
        Header::new(b":authority", b"example.com").expect("test must succeed"),
    ]
}

#[test]
fn send_request_after_goaway_returns_stream_error() {
    // GOAWAY 受信後の send_request が StreamError(RequestRejected) を返すことを検証する
    let (mut client, mut server) = setup_pair();
    let headers = request_headers();

    // 1 本目のリクエスト送信 (stream_id=0, next_stream_id は 4 になる)
    let stream_id = client
        .send_request(&headers, true)
        .expect("test must succeed");
    assert_eq!(stream_id, 0);

    // サーバーが GOAWAY(4) を送信: stream_id >= 4 は受け付けない
    server
        .send_goaway(VarInt::new(4).expect("test must succeed"))
        .expect("test must succeed");
    let (goaway_data, _) = server.take_stream_data(3).expect("test must succeed");
    client
        .feed_stream(3, &goaway_data, false)
        .expect("test must succeed");

    // GoawayReceived イベントを消費
    let event = client.poll_event().expect("test must succeed");
    assert!(
        matches!(event, Some(Event::GoawayReceived { .. })),
        "GoawayReceived イベントが発生すること"
    );

    // 2 本目のリクエスト送信: StreamError(RequestRejected) で拒否される
    let err = client.send_request(&headers, true).unwrap_err();
    assert!(
        matches!(err, Error::StreamError(ErrorCode::RequestRejected)),
        "GOAWAY 境界超過時は StreamError(RequestRejected) であること: {err:?}"
    );
}

#[test]
fn send_request_below_goaway_boundary_succeeds() {
    // GOAWAY 境界値未満のストリーム ID では正常に送信できることを検証する
    let (mut client, mut server) = setup_pair();
    let headers = request_headers();

    // サーバーが GOAWAY(8) を送信: stream_id >= 8 は受け付けない
    server
        .send_goaway(VarInt::new(8).expect("test must succeed"))
        .expect("test must succeed");
    let (goaway_data, _) = server.take_stream_data(3).expect("test must succeed");
    client
        .feed_stream(3, &goaway_data, false)
        .expect("test must succeed");

    // GoawayReceived イベントを消費
    while let Some(_ev) = client.poll_event().expect("test must succeed") {}

    // stream_id=0 (next=4): 4 < 8 なので送信できる
    let stream_id = client
        .send_request(&headers, true)
        .expect("test must succeed");
    assert_eq!(stream_id, 0, "境界値未満のリクエストは送信できること");

    // stream_id=4 (next=8): 8 >= 8 なので次は拒否される
    let stream_id2 = client
        .send_request(&headers, true)
        .expect("test must succeed");
    assert_eq!(stream_id2, 4, "境界値直前のリクエストも送信できること");

    // stream_id=8 (next=12): 8 >= 8 で拒否
    let err = client.send_request(&headers, true).unwrap_err();
    assert!(
        matches!(err, Error::StreamError(ErrorCode::RequestRejected)),
        "境界値以上のリクエストは拒否されること: {err:?}"
    );
}

#[test]
fn connection_maintained_after_stream_error() {
    // StreamError 返却後も接続が維持され、既存ストリームの操作が可能であることを検証する
    let (mut client, mut server) = setup_pair();
    let headers = request_headers();

    // 1 本目のリクエスト送信 (fin=false でストリームを開いたまま)
    let stream_id = client
        .send_request(&headers, false)
        .expect("test must succeed");
    assert_eq!(stream_id, 0);

    // サーバーが GOAWAY(4) を送信
    server
        .send_goaway(VarInt::new(4).expect("test must succeed"))
        .expect("test must succeed");
    let (goaway_data, _) = server.take_stream_data(3).expect("test must succeed");
    client
        .feed_stream(3, &goaway_data, false)
        .expect("test must succeed");
    while let Some(_ev) = client.poll_event().expect("test must succeed") {}

    // 新規リクエストは拒否される
    let err = client.send_request(&headers, true).unwrap_err();
    assert!(matches!(
        err,
        Error::StreamError(ErrorCode::RequestRejected)
    ));

    // 既存ストリーム (stream_id=0) のデータ取得は引き続き可能
    // (send_request 時に HEADERS フレームが生成されているので take_stream_data で取得できる)
    let data = client.take_stream_data(stream_id);
    assert!(
        data.is_some(),
        "StreamError 後も既存ストリームのデータ取得が可能であること"
    );
}

#[test]
fn empty_goaway_payload_is_frame_error() {
    // payload 長 0 の GOAWAY を制御ストリームに投入すると H3_FRAME_ERROR になることを検証する
    // (RFC 9114 Section 7.1)。デコード失敗が BufferTooShort のままだと接続エラーに集約されない。
    let (mut client, _server) = setup_pair();

    // setup_pair で stream 3 は制御ストリームとして確立済み (type 0x00 + SETTINGS 投入済み)。
    // そこへ GOAWAY (type=0x07) で length=0x00 の不正フレームを投入する。
    let err = client.feed_stream(3, &[0x07, 0x00], false).unwrap_err();
    assert!(
        matches!(err, Error::ConnectionError(ErrorCode::FrameError)),
        "空 payload の GOAWAY は H3_FRAME_ERROR であること: {err:?}"
    );
}
