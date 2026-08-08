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

#[test]
fn send_request_fin_is_delivered_via_take_stream_data() {
    // fin=true のリクエスト送信で FIN がデータ消費後の追加呼び出しで交付されることを検証する
    // (RFC 9114 Section 4.1: 送信方向クローズ)
    let (mut client, _server) = setup_pair();
    let headers = request_headers();

    let stream_id = client
        .send_request(&headers, true)
        .expect("test must succeed");

    // 1 回目: HEADERS データ (FIN はデータと同時には交付されない)
    let (data, fin) = client
        .take_stream_data(stream_id)
        .expect("test must succeed");
    assert!(!data.is_empty(), "HEADERS データが取得できること");
    assert!(!fin, "データと同時に FIN は交付されないこと");

    // データ消費後・FIN 交付前は writable_streams に残り続ける
    // (busy loop の原因は「FIN-only ストリームが報告され続ける」ことなので、両側を固定する)
    let writable: Vec<u64> = client.writable_streams().collect();
    assert!(
        writable.contains(&stream_id),
        "FIN 交付前は writable_streams にストリームが残っていること: {writable:?}"
    );

    // 2 回目: データ消費後の追加呼び出しで (空, fin=true) が交付される
    let (data, fin) = client
        .take_stream_data(stream_id)
        .expect("test must succeed");
    assert!(data.is_empty(), "FIN 交付時はデータが空であること");
    assert!(fin, "データ消費後に FIN が交付されること");

    // 3 回目以降: FIN は 1 回だけ交付され、交付後は取得できない
    assert!(
        client.take_stream_data(stream_id).is_none(),
        "FIN 交付後は take_stream_data が None を返すこと"
    );

    // FIN 送達後は writable_streams にストリームが残らない
    let writable: Vec<u64> = client.writable_streams().collect();
    assert!(
        !writable.contains(&stream_id),
        "FIN 送達後は writable_streams にストリームが残らないこと: {writable:?}"
    );
}

#[test]
fn send_body_fin_is_delivered_after_body() {
    // fin=true のボディ送信で、ボディ消費後に FIN が交付されることを検証する
    let (mut client, _server) = setup_pair();
    let headers = request_headers();

    let stream_id = client
        .send_request(&headers, false)
        .expect("test must succeed");
    client
        .send_body(stream_id, b"hello", true)
        .expect("test must succeed");

    // 1 回目: HEADERS + DATA (fin=false)
    let (data, fin) = client
        .take_stream_data(stream_id)
        .expect("test must succeed");
    assert!(!data.is_empty(), "HEADERS + DATA が取得できること");
    assert!(!fin, "データと同時に FIN は交付されないこと");

    // 2 回目: (空, fin=true)
    let (data, fin) = client
        .take_stream_data(stream_id)
        .expect("test must succeed");
    assert!(data.is_empty(), "FIN 交付時はデータが空であること");
    assert!(fin, "データ消費後に FIN が交付されること");

    // 3 回目: None
    assert!(
        client.take_stream_data(stream_id).is_none(),
        "FIN 交付後は take_stream_data が None を返すこと"
    );
}

#[test]
fn fin_is_delivered_only_after_all_chunks_consumed() {
    // ボディが複数チャンクに分かれても、FIN は全データ消費後の追加呼び出しで交付されることを検証する
    let (mut client, _server) = setup_pair();
    let headers = request_headers();

    let stream_id = client
        .send_request(&headers, false)
        .expect("test must succeed");
    client
        .send_body(stream_id, b"hello", false)
        .expect("test must succeed");
    client
        .send_body(stream_id, b"world", false)
        .expect("test must succeed");
    client
        .send_body(stream_id, b"!", true)
        .expect("test must succeed");

    // 1 回目: HEADERS + 全 DATA (fin=false)
    let (data, fin) = client
        .take_stream_data(stream_id)
        .expect("test must succeed");
    assert!(!data.is_empty(), "HEADERS + 全 DATA が取得できること");
    assert!(!fin, "データと同時に FIN は交付されないこと");

    // 2 回目: 全データ消費後の追加呼び出しで (空, fin=true)
    let (data, fin) = client
        .take_stream_data(stream_id)
        .expect("test must succeed");
    assert!(data.is_empty(), "FIN 交付時はデータが空であること");
    assert!(fin, "全データ消費後に FIN が交付されること");

    // 3 回目: None
    assert!(
        client.take_stream_data(stream_id).is_none(),
        "FIN 交付後は take_stream_data が None を返すこと"
    );
}

#[test]
fn empty_body_with_fin_delivered_after_headers_consumed() {
    // 空ボディ + fin=true では DATA フレームが積まれず、HEADERS 消費直後の
    // 追加呼び出しで (空, fin=true) が交付されることを検証する
    // (HEADERS は必ず先行するため、FIN が最初の呼び出しで交付されることはない)
    let (mut client, _server) = setup_pair();
    let headers = request_headers();

    let stream_id = client
        .send_request(&headers, false)
        .expect("test must succeed");
    client
        .send_body(stream_id, &[], true)
        .expect("test must succeed");

    // 1 回目: HEADERS (fin=false)
    let (data, fin) = client
        .take_stream_data(stream_id)
        .expect("test must succeed");
    assert!(!data.is_empty(), "HEADERS が取得できること");
    assert!(!fin, "データと同時に FIN は交付されないこと");

    // 2 回目: 空ボディなのでデータ消費直後に (空, fin=true)
    let (data, fin) = client
        .take_stream_data(stream_id)
        .expect("test must succeed");
    assert!(data.is_empty(), "FIN 交付時はデータが空であること");
    assert!(fin, "空ボディ + fin では直後に FIN が交付されること");

    // 3 回目: None
    assert!(
        client.take_stream_data(stream_id).is_none(),
        "FIN 交付後は take_stream_data が None を返すこと"
    );
}

#[test]
fn fin_delivery_via_get_stream_data_and_consume_stream_data() {
    // get_stream_data + consume_stream_data の 2 段階 API でも
    // FIN がデータ消費後に交付され、交付後はどの API でも None になることを検証する
    let (mut client, _server) = setup_pair();
    let headers = request_headers();

    let stream_id = client
        .send_request(&headers, true)
        .expect("test must succeed");

    // 1 回目: HEADERS データ (fin=false)
    let (data, fin) = client
        .get_stream_data(stream_id)
        .expect("test must succeed");
    assert!(!fin, "データ消費前は fin=false であること");
    let len = data.len();
    client.consume_stream_data(stream_id, len);

    // 2 回目: データ消費後は (空, fin=true)
    let (data, fin) = client
        .get_stream_data(stream_id)
        .expect("test must succeed");
    assert!(data.is_empty(), "FIN 交付時はデータが空であること");
    assert!(fin, "データ消費後に FIN が交付されること");
    let len = data.len();
    client.consume_stream_data(stream_id, len);

    // 3 回目: FIN 交付済みのため 2 段階 API では None
    assert!(
        client.get_stream_data(stream_id).is_none(),
        "FIN 交付後は get_stream_data が None を返すこと"
    );

    // 交付後は take_stream_data でも None になる (cross-API の整合)
    assert!(
        client.take_stream_data(stream_id).is_none(),
        "FIN 交付後は take_stream_data も None を返すこと"
    );
}

#[test]
fn fin_is_delivered_after_partial_consumption() {
    // 2 段階 API でデータを部分消費した場合、FIN は全消費後の
    // 追加呼び出しで交付されることを検証する
    let (mut client, _server) = setup_pair();
    let headers = request_headers();

    let stream_id = client
        .send_request(&headers, false)
        .expect("test must succeed");
    client
        .send_body(stream_id, b"hello world", true)
        .expect("test must succeed");

    // 1 回目: HEADERS + DATA (fin=false)。半分だけ消費する
    let (data, fin) = client
        .get_stream_data(stream_id)
        .expect("test must succeed");
    assert!(!fin, "部分消費前は fin=false であること");
    let half = data.len() / 2;
    client.consume_stream_data(stream_id, half);

    // 2 回目: 残データが返る (fin=false)。全消費する
    let (data, fin) = client
        .get_stream_data(stream_id)
        .expect("test must succeed");
    assert!(!data.is_empty(), "残データが取得できること");
    assert!(!fin, "データが残っている間は fin=false であること");
    let len = data.len();
    client.consume_stream_data(stream_id, len);

    // 3 回目: 全消費後の追加呼び出しで (空, fin=true)
    let (data, fin) = client
        .get_stream_data(stream_id)
        .expect("test must succeed");
    assert!(data.is_empty(), "FIN 交付時はデータが空であること");
    assert!(fin, "全データ消費後に FIN が交付されること");
    let len = data.len();
    client.consume_stream_data(stream_id, len);

    // 4 回目: None
    assert!(
        client.get_stream_data(stream_id).is_none(),
        "FIN 交付後は get_stream_data が None を返すこと"
    );
}

#[test]
fn fin_delivered_by_take_is_unavailable_via_get_stream_data() {
    // take_stream_data で FIN を交付した後は get_stream_data でも取得できないことを検証する
    let (mut client, _server) = setup_pair();
    let headers = request_headers();

    let stream_id = client
        .send_request(&headers, true)
        .expect("test must succeed");

    // 1 回目: HEADERS (fin=false)
    let (data, fin) = client
        .take_stream_data(stream_id)
        .expect("test must succeed");
    assert!(!fin, "データと同時に FIN は交付されないこと");
    assert!(!data.is_empty(), "HEADERS が取得できること");

    // 2 回目: take_stream_data で (空, fin=true)
    let (data, fin) = client
        .take_stream_data(stream_id)
        .expect("test must succeed");
    assert!(data.is_empty(), "FIN 交付時はデータが空であること");
    assert!(fin, "take_stream_data で FIN が交付されること");

    // 交付後は get_stream_data でも None (cross-API の整合)
    assert!(
        client.get_stream_data(stream_id).is_none(),
        "FIN 交付後は get_stream_data も None を返すこと"
    );
}

#[test]
fn send_response_fin_is_delivered_via_take_stream_data() {
    // fin=true のレスポンス送信で FIN がデータ消費後の追加呼び出しで交付されることを検証する
    // (RFC 9114 Section 4.1: 最終レスポンス送信後にストリームを閉じる)
    let (mut client, mut server) = setup_pair();
    let headers = request_headers();

    // クライアントからリクエストを送信してサーバーに feed
    let stream_id = client
        .send_request(&headers, false)
        .expect("test must succeed");
    let (req_data, _) = client
        .take_stream_data(stream_id)
        .expect("test must succeed");
    server
        .feed_stream(stream_id, &req_data, true)
        .expect("test must succeed");
    while let Some(_ev) = server.poll_event().expect("test must succeed") {}

    // サーバーが fin=true でレスポンスを送信
    let response = vec![Header::new(b":status", b"200").expect("test must succeed")];
    server
        .send_response(stream_id, &response, true)
        .expect("test must succeed");

    // 1 回目: HEADERS (fin=false)
    let (data, fin) = server
        .take_stream_data(stream_id)
        .expect("test must succeed");
    assert!(!data.is_empty(), "HEADERS が取得できること");
    assert!(!fin, "データと同時に FIN は交付されないこと");

    // 2 回目: (空, fin=true)
    let (data, fin) = server
        .take_stream_data(stream_id)
        .expect("test must succeed");
    assert!(data.is_empty(), "FIN 交付時はデータが空であること");
    assert!(fin, "データ消費後に FIN が交付されること");

    // 3 回目: None
    assert!(
        server.take_stream_data(stream_id).is_none(),
        "FIN 交付後は take_stream_data が None を返すこと"
    );

    // FIN 送達後は writable_streams にストリームが残らない
    let writable: Vec<u64> = server.writable_streams().collect();
    assert!(
        !writable.contains(&stream_id),
        "FIN 送達後は writable_streams にストリームが残らないこと: {writable:?}"
    );
}
