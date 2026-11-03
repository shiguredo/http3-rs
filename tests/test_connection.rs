use shiguredo_http3::{
    ClientConnection, Error, ErrorCode, Event, Header, ServerConnection, Settings, VarInt,
    WebTransportEvent, webtransport,
};

/// 制御ストリーム ID を設定し、サーバー制御ストリームをクライアントに受信させたペアを構築する
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

#[test]
fn send_request_with_many_large_headers_succeeds() {
    // QPACK のバッファ見積りが不足すると encode が失敗し、以前は
    // ConnectionError(InternalError) が接続にラッチされて以降の送信がすべて失敗していた。
    // 104 ヘッダー (cookie × 100、各値 16,384 バイト) を送信しても成功し、
    // 直後の小さいリクエストも成功することを検証する。
    //
    // 0x80 (obs-text) はハフマン符号化が生データより長くなるため非ハフマンで
    // エンコードされ、値長プレフィックスが 3 バイトになる境界を確実に通る。
    let mut client = ClientConnection::new(Settings::default());
    client.set_control_stream_id(2).expect("test must succeed");

    let mut headers = request_headers();
    for _ in 0..100 {
        let value = vec![0x80u8; 16_384];
        headers.push(Header::new(b"cookie", value).expect("test must succeed"));
    }

    let stream_id = client
        .send_request(&headers, true)
        .expect("大きいヘッダーでも送信できること");
    assert_eq!(stream_id, 0);

    // 接続が恒久エラー状態になっていないこと
    let small_stream_id = client
        .send_request(&request_headers(), true)
        .expect("直後の小さいリクエストも送信できること");
    assert_eq!(small_stream_id, 4);

    // 大きいヘッダーのストリームから HEADERS データを取得できること
    assert!(
        client.take_stream_data(stream_id).is_some(),
        "大きいヘッダーの HEADERS データが取得できること"
    );
}

#[test]
fn local_send_error_does_not_latch_connection() {
    // ローカルな誤用 (制御ストリーム未設定) による送信エラーは、受信した
    // プロトコル違反ではないため接続を恒久エラー状態にしないことを検証する。
    // 以前は send_request が ConnectionError を remember_connection_error で
    // ラッチしていたため、以降の送信がすべて同じエラーで失敗していた。
    let mut client = ClientConnection::new(Settings::default());

    // control stream 未設定: ローカルな誤用で InternalError
    let err = client
        .send_request(&request_headers(), true)
        .expect_err("制御ストリーム未設定では送信できないこと");
    assert!(
        matches!(err, Error::ConnectionError(ErrorCode::InternalError)),
        "制御ストリーム未設定時は InternalError であること: {err:?}"
    );

    // 制御ストリームを設定すれば送信できる (エラーがラッチされていないこと)
    client.set_control_stream_id(2).expect("test must succeed");
    let stream_id = client
        .send_request(&request_headers(), true)
        .expect("エラー後に制御ストリームを設定すれば送信できること");
    assert_eq!(stream_id, 0, "失敗した送信はストリーム ID を消費しないこと");
}

#[test]
fn received_body_over_limit_is_excessive_load() {
    // 受信ボディの累積上限を超えた DATA は H3_EXCESSIVE_LOAD で接続を閉じる
    let (mut client, mut server) = setup_pair();
    server.set_max_received_body_size(4);

    let headers = vec![
        Header::new(b":method", b"POST").expect("test must succeed"),
        Header::new(b":path", b"/upload").expect("test must succeed"),
        Header::new(b":scheme", b"https").expect("test must succeed"),
        Header::new(b":authority", b"example.com").expect("test must succeed"),
    ];
    let stream_id = client
        .send_request(&headers, false)
        .expect("test must succeed");

    // ヘッダーをサーバーに届けてイベントを消費する
    let (header_data, _) = client
        .take_stream_data(stream_id)
        .expect("test must succeed");
    server
        .feed_stream(stream_id, &header_data, false)
        .expect("test must succeed");
    while server.poll_event().expect("test must succeed").is_some() {}

    // 上限 (4 バイト) を超える 5 バイトのボディを送る
    client
        .send_body(stream_id, b"hello", true)
        .expect("test must succeed");
    let (body_data, _) = client
        .take_stream_data(stream_id)
        .expect("test must succeed");

    let err = server
        .feed_stream(stream_id, &body_data, true)
        .expect_err("上限を超えるボディは拒否されること");
    assert!(
        matches!(err, Error::ConnectionError(ErrorCode::ExcessiveLoad)),
        "H3_EXCESSIVE_LOAD になること: {err:?}"
    );
}

#[test]
fn connection_debug_redacts_sensitive_headers() {
    // Connection の Debug 出力にイベントキュー経由で Cookie の値が漏れないこと
    let (mut client, mut server) = setup_pair();
    let headers = vec![
        Header::new(b":method", b"GET").expect("test must succeed"),
        Header::new(b":path", b"/").expect("test must succeed"),
        Header::new(b":scheme", b"https").expect("test must succeed"),
        Header::new(b":authority", b"example.com").expect("test must succeed"),
        Header::new(b"cookie", b"session=secret").expect("test must succeed"),
    ];
    let stream_id = client
        .send_request(&headers, true)
        .expect("test must succeed");
    let (header_data, _) = client
        .take_stream_data(stream_id)
        .expect("test must succeed");
    server
        .feed_stream(stream_id, &header_data, true)
        .expect("test must succeed");

    let debug = format!("{server:?}");
    assert!(
        !debug.contains("session=secret"),
        "Cookie の値が Connection の Debug 出力に含まれないこと"
    );
    assert!(
        debug.contains("<redacted>"),
        "Cookie の値が Connection の Debug 出力で伏せられていること"
    );
}

#[test]
fn request_stream_wt_stream_after_headers_is_frame_unexpected() {
    // リクエスト開始者が送る最初のバイト列より後で WT_STREAM (0x41) を受信すると
    // H3_FRAME_UNEXPECTED の接続エラーになること
    // (draft-ietf-webtrans-http3-17 Section 4.3)
    let (mut client, mut server) = setup_pair();
    let stream_id = client
        .send_request(&request_headers(), false)
        .expect("テスト用の処理に成功すること");

    // HEADERS をサーバーに届けてイベントを消費する
    let (header_data, _) = client
        .take_stream_data(stream_id)
        .expect("テスト用の処理に成功すること");
    server
        .feed_stream(stream_id, &header_data, false)
        .expect("テスト用の処理に成功すること");
    while server
        .poll_event()
        .expect("テスト用の処理に成功すること")
        .is_some()
    {}

    // リクエストの途中で WT_STREAM のフレームタイプだけを投入する
    // (WT_STREAM は length を持たないため、length は送らない)
    let err = server
        .feed_stream(stream_id, &[0x40, 0x41], false)
        .expect_err("不正な位置の WT_STREAM は拒否されること");
    assert!(
        matches!(err, Error::ConnectionError(ErrorCode::FrameUnexpected)),
        "H3_FRAME_UNEXPECTED になること: {err:?}"
    );
}

#[test]
fn request_stream_wt_stream_split_varint_is_frame_unexpected() {
    // WT_STREAM 型の可変長整数が複数の feed_stream 呼び出しに分割されても、
    // 型が確定した時点で H3_FRAME_UNEXPECTED になること
    // (draft-ietf-webtrans-http3-17 Section 4.3)
    let (mut client, mut server) = setup_pair();
    let stream_id = client
        .send_request(&request_headers(), false)
        .expect("テスト用の処理に成功すること");
    let (header_data, _) = client
        .take_stream_data(stream_id)
        .expect("テスト用の処理に成功すること");
    server
        .feed_stream(stream_id, &header_data, false)
        .expect("テスト用の処理に成功すること");
    while server
        .poll_event()
        .expect("テスト用の処理に成功すること")
        .is_some()
    {}

    // type=0x41 の 1 バイト目だけでは型を確定できないため、エラーにならない
    server
        .feed_stream(stream_id, &[0x40], false)
        .expect("型が未確定の間はエラーにならないこと");

    // 2 バイト目で型が確定するため、length の到着を待たずに接続エラーになる
    let err = server
        .feed_stream(stream_id, &[0x41], false)
        .expect_err("WT_STREAM 型の確定で拒否されること");
    assert!(
        matches!(err, Error::ConnectionError(ErrorCode::FrameUnexpected)),
        "H3_FRAME_UNEXPECTED になること: {err:?}"
    );
}

#[test]
fn response_stream_wt_stream_after_headers_is_frame_unexpected() {
    // クライアントがレスポンス受信方向のストリームで WT_STREAM (0x41) を受信すると
    // H3_FRAME_UNEXPECTED の接続エラーになること
    // (draft-ietf-webtrans-http3-17 Section 4.3: 開始者が送る最初のバイト列の例外は
    //  ストリームを開始する側にのみ該当する)
    let (mut client, mut server) = setup_pair();
    let stream_id = client
        .send_request(&request_headers(), false)
        .expect("テスト用の処理に成功すること");
    let (request_data, _) = client
        .take_stream_data(stream_id)
        .expect("テスト用の処理に成功すること");
    server
        .feed_stream(stream_id, &request_data, false)
        .expect("テスト用の処理に成功すること");
    while server
        .poll_event()
        .expect("テスト用の処理に成功すること")
        .is_some()
    {}

    // サーバーがレスポンス HEADERS を送信する
    let response = vec![Header::new(b":status", b"200").expect("テスト用の処理に成功すること")];
    server
        .send_response(stream_id, &response, false)
        .expect("テスト用の処理に成功すること");
    let (response_data, _) = server
        .take_stream_data(stream_id)
        .expect("テスト用の処理に成功すること");
    client
        .feed_stream(stream_id, &response_data, false)
        .expect("テスト用の処理に成功すること");
    while client
        .poll_event()
        .expect("テスト用の処理に成功すること")
        .is_some()
    {}

    // レスポンスの途中で WT_STREAM のフレームタイプだけを投入する
    let err = client
        .feed_stream(stream_id, &[0x40, 0x41], false)
        .expect_err("レスポンス方向の WT_STREAM は拒否されること");
    assert!(
        matches!(err, Error::ConnectionError(ErrorCode::FrameUnexpected)),
        "H3_FRAME_UNEXPECTED になること: {err:?}"
    );
}

#[test]
fn response_stream_wt_stream_at_first_byte_is_frame_unexpected() {
    // クライアントがレスポンス受信方向のストリームの先頭で WT_STREAM (0x41) を
    // 受信すると H3_FRAME_UNEXPECTED の接続エラーになること
    // (draft-ietf-webtrans-http3-17 Section 4.3: 開始者が送る最初のバイト列の例外は
    //  ストリームを開始する側にのみ該当する)
    let (mut client, mut server) = setup_pair();
    let stream_id = client
        .send_request(&request_headers(), false)
        .expect("テスト用の処理に成功すること");
    let (request_data, _) = client
        .take_stream_data(stream_id)
        .expect("テスト用の処理に成功すること");
    server
        .feed_stream(stream_id, &request_data, false)
        .expect("テスト用の処理に成功すること");
    while server
        .poll_event()
        .expect("テスト用の処理に成功すること")
        .is_some()
    {}

    // レスポンス HEADERS より前に WT_STREAM のフレームタイプだけを投入する
    let err = client
        .feed_stream(stream_id, &[0x40, 0x41], false)
        .expect_err("レスポンス先頭の WT_STREAM は拒否されること");
    assert!(
        matches!(err, Error::ConnectionError(ErrorCode::FrameUnexpected)),
        "H3_FRAME_UNEXPECTED になること: {err:?}"
    );
}

#[test]
fn control_stream_truncated_frame_type_is_frame_error() {
    // 制御ストリームでフレームタイプの可変長整数が途中のまま FIN した場合は、
    // 型を確定できないため一般のフレーム途中切れとして H3_FRAME_ERROR になること
    // (RFC 9114 Section 7.1)
    let (mut client, _server) = setup_pair();
    let err = client
        .feed_stream(3, &[0x40], true)
        .expect_err("型が未確定のまま FIN した場合はフレームエラーになること");
    assert!(
        matches!(err, Error::ConnectionError(ErrorCode::FrameError)),
        "H3_FRAME_ERROR になること: {err:?}"
    );
}

#[test]
fn control_stream_wt_stream_after_settings_is_frame_unexpected() {
    // SETTINGS 受信後の制御ストリームで WT_STREAM (0x41) を受信すると
    // H3_FRAME_UNEXPECTED の接続エラーになること
    // (draft-ietf-webtrans-http3-17 Section 4.3)
    let (mut client, _server) = setup_pair();

    // setup_pair で stream 3 の制御ストリーム (ストリームタイプ + SETTINGS) は
    // 受信済みのため、Ready 状態で WT_STREAM のフレームタイプだけを投入する
    let err = client
        .feed_stream(3, &[0x40, 0x41], false)
        .expect_err("制御ストリームの WT_STREAM は拒否されること");
    assert!(
        matches!(err, Error::ConnectionError(ErrorCode::FrameUnexpected)),
        "H3_FRAME_UNEXPECTED になること: {err:?}"
    );
}

#[test]
fn control_stream_wt_stream_before_settings_is_missing_settings() {
    // SETTINGS を先頭に受信していない制御ストリームでは、WT_STREAM (0x41) より
    // RFC 9114 Section 6.2.1 の H3_MISSING_SETTINGS が優先されること
    let mut server = ServerConnection::new(Settings::default());
    server
        .set_control_stream_id(3)
        .expect("テスト用の処理に成功すること");

    // クライアントの制御ストリーム (stream 2) のストリームタイプ 0x00 の直後に、
    // SETTINGS ではなく WT_STREAM (0x41) を置く。WT_STREAM は length を持たない
    // ため、フレームタイプだけで判定される (後続バイトの有無によらない)
    // (RFC 9114 Section 6.2.1, draft-ietf-webtrans-http3-17 Section 4.3)。
    for data in [&[0x00u8, 0x40, 0x41][..], &[0x00, 0x40, 0x41, 0x00][..]] {
        let mut server = ServerConnection::new(Settings::default());
        server
            .set_control_stream_id(3)
            .expect("テスト用の制御ストリーム設定に成功すること");
        let err = server
            .feed_stream(2, data, false)
            .expect_err("SETTINGS 前の WT_STREAM は拒否されること");
        assert!(
            matches!(err, Error::ConnectionError(ErrorCode::MissingSettings)),
            "H3_MISSING_SETTINGS になること: {err:?}"
        );
    }
}

// =============================================================================
// WebTransport draft-17 の先行ストリーム / データグラムのバッファリングと拒否
// (draft-ietf-webtrans-http3-17 Section 4.6)
//
// バッファ上限を超えたストリーム / データグラムの拒否がセッション終了へ
// 波及せず、同じセッションを継続して利用できることを公開接続 API 経由で検証する。
//
// Section 4.6 が MUST として定めるのは「保留するストリーム数 / データグラム数」の
// 上限である。本モジュールが併せて検証する 1 ストリームあたり 64 KiB の受信バイト
// 上限は、実装が独自に設けた DoS 対策である。
// =============================================================================

mod webtransport_buffering {
    use super::*;
    use shiguredo_http3::webtransport::stream::StreamHeader;
    use shiguredo_http3::webtransport::{Datagram, DraftVersion, ErrorCode as WtErrorCode};

    /// 本モジュールのテストで使うセッション ID
    ///
    /// クライアントは最初のリクエストとして CONNECT をクライアント開始 bidi の
    /// stream 0 に送るため、セッション ID (CONNECT ストリーム ID) は 0 になる。
    /// SETTINGS 受信前に到着する先行ストリームも、この ID 宛として送る。
    const SESSION_ID: u64 = 0;

    /// セッション確立前に保留できるストリーム数の上限
    ///
    /// 実装の `MAX_BUFFERED_STREAMS` と一致する境界値。公開 API からは参照できない
    /// ため、境界値テストの期待値としてここに固定する。Section 4.6 は保留する
    /// ストリーム数を制限することを MUST としており、この値はその実装値である。
    const MAX_BUFFERED_STREAMS: usize = 100;

    /// セッション確立前に 1 ストリームあたり保留できる受信バイト数の上限
    ///
    /// 実装の `WT_MAX_BUFFERED_STREAM_BYTES` (64 KiB) と一致する境界値である。
    /// この上限は draft-17 Section 4.6 の MUST ではなく、実装独自の DoS 対策である
    /// (Section 4.6 が上限を要求するのは保留するストリーム数とデータグラム数)。
    ///
    /// 判定対象は経路によって異なり、同じ上限でも 3 バイトの差が出る。
    /// - SETTINGS 受信前: signal value と session ID を含む受信バイト列全体
    /// - SETTINGS 受信後 (Pending セッション): ストリームヘッダーを除いたペイロードのみ
    const MAX_BUFFERED_STREAM_BYTES: usize = 64 * 1024;

    /// セッション確立前に保留できるデータグラム数の上限
    ///
    /// 実装の `MAX_BUFFERED_DATAGRAMS` と一致する境界値。Section 4.6 は保留する
    /// データグラム数を制限することを MUST としており、この値はその実装値である。
    const MAX_BUFFERED_DATAGRAMS: usize = 100;

    /// draft-17 のコードポイントと広いフロー制御上限を広告する WebTransport 設定
    ///
    /// フロー制御上限は、バッファ上限 (100 ストリーム) まで配送しても
    /// WT_FLOW_CONTROL_ERROR にならないよう十分大きくする。
    fn wt_draft17_settings() -> webtransport::Settings {
        webtransport::Settings::new()
            .wt_enabled_draft17(VarInt::new(1).expect("テスト用の設定値が VarInt に収まること"))
            .wt_initial_max_streams_uni(
                VarInt::new(1000).expect("テスト用の設定値が VarInt に収まること"),
            )
            .wt_initial_max_streams_bidi(
                VarInt::new(1000).expect("テスト用の設定値が VarInt に収まること"),
            )
            .wt_initial_max_data(
                VarInt::new(8 * 1024 * 1024).expect("テスト用の設定値が VarInt に収まること"),
            )
    }

    /// draft-17 の CONNECT ヘッダーを構築する
    ///
    /// 本文版のトークンである `webtransport-h3` を使う
    /// (draft-ietf-webtrans-http3-17 Section 9.1)。
    fn connect_headers() -> Vec<Header> {
        vec![
            Header::new(b":method", b"CONNECT").expect("テスト用のヘッダー作成に成功すること"),
            Header::new(b":protocol", b"webtransport-h3")
                .expect("テスト用のヘッダー作成に成功すること"),
            Header::new(b":scheme", b"https").expect("テスト用のヘッダー作成に成功すること"),
            Header::new(b":authority", b"example.com")
                .expect("テスト用のヘッダー作成に成功すること"),
            Header::new(b":path", b"/wt").expect("テスト用のヘッダー作成に成功すること"),
        ]
    }

    /// 双方向 WT ストリームの実バイト列 (signal value + session ID + ペイロード)
    ///
    /// (draft-ietf-webtrans-http3-17 Section 4.3)
    fn bidi_stream_bytes(session_id: u64, payload: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        StreamHeader::new(session_id)
            .expect("テスト用の session ID が client-initiated bidi ストリーム ID であること")
            .encode_bidirectional(&mut buf);
        buf.extend_from_slice(payload);
        buf
    }

    /// 単方向 WT ストリームの実バイト列 (stream type + session ID + ペイロード)
    ///
    /// (draft-ietf-webtrans-http3-17 Section 4.2)
    ///
    /// 単方向ストリームであっても、session ID 自体は CONNECT に使う
    /// client-initiated bidi ストリーム ID でなければならない。
    fn uni_stream_bytes(session_id: u64, payload: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        StreamHeader::new(session_id)
            .expect("テスト用の session ID が client-initiated bidi ストリーム ID であること")
            .encode_unidirectional(&mut buf);
        buf.extend_from_slice(payload);
        buf
    }

    /// HTTP Datagram の実バイト列 (Quarter Stream ID + ペイロード)
    ///
    /// (RFC 9297 Section 2.1, draft-ietf-webtrans-http3-17 Section 4.5)
    /// 位置に依存したバイト列を生成する
    ///
    /// 同一バイトの繰り返しでは、チャンクの分割位置がずれても内容が一致してしまい、
    /// データの欠落や重複を検出できない。位置ごとに異なる値にする。
    fn payload_bytes(len: usize) -> Vec<u8> {
        (0..len)
            .map(|i| {
                let mixed = (i as u32).wrapping_mul(0x9e37_79b9).wrapping_add(7);
                (mixed ^ (mixed >> 13)) as u8
            })
            .collect()
    }

    fn datagram_bytes(session_id: u64, payload: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        Datagram::new(session_id, payload.to_vec())
            .expect("テスト用の datagram 作成に成功すること")
            .encode(&mut buf);
        buf
    }

    /// draft-17 のサーバーとクライアントの接続
    ///
    /// クライアントにはサーバー SETTINGS と transport parameter 検証を済ませた状態で
    /// 構築し、クライアント SETTINGS のサーバーへの投入は呼び出し側に委ねる。
    /// SETTINGS の投入順序を制御するために必要な構成である。
    struct WtPair {
        server: ServerConnection,
        client: ClientConnection,
        /// サーバーへまだ投入していないクライアント SETTINGS
        client_control: Vec<u8>,
    }

    impl WtPair {
        fn new() -> Self {
            let mut server = ServerConnection::new(
                Settings::new().enable_webtransport_server(wt_draft17_settings()),
            );
            server
                .set_control_stream_id(3)
                .expect("テスト用の制御ストリーム設定に成功すること");
            server
                .set_webtransport_transport_verified(true, true)
                .expect("テスト用の transport parameter 検証に成功すること");
            let (server_control, _) = server
                .take_stream_data(3)
                .expect("テスト用のサーバー SETTINGS 取得に成功すること");

            let mut client = ClientConnection::new(
                Settings::new().enable_webtransport_client(wt_draft17_settings()),
            );
            client
                .set_control_stream_id(2)
                .expect("テスト用の制御ストリーム設定に成功すること");
            client
                .set_webtransport_transport_verified(true, true)
                .expect("テスト用の transport parameter 検証に成功すること");
            let (client_control, _) = client
                .take_stream_data(2)
                .expect("テスト用のクライアント SETTINGS 取得に成功すること");

            client
                .feed_stream(3, &server_control, false)
                .expect("クライアントのサーバー SETTINGS 受信に成功すること");
            client
                .drain_events()
                .expect("テスト用のクライアントイベント取得に成功すること");
            assert_eq!(
                client.negotiated_wt_draft_version(),
                Some(DraftVersion::Draft17),
                "サーバー SETTINGS 受信後に draft-17 が選択されること"
            );

            Self {
                server,
                client,
                client_control,
            }
        }

        /// 遅延させていたクライアント SETTINGS をサーバーに投入する
        ///
        /// 保留していたストリームの再ディスパッチでイベントが発火し得るため、
        /// そのイベント列を呼び出し側が検証できるよう返す。
        fn deliver_client_settings(&mut self) -> Vec<Event> {
            self.server
                .feed_stream(2, &self.client_control, false)
                .expect("サーバーのクライアント SETTINGS 受信に成功すること");
            let events = self
                .server
                .drain_events()
                .expect("テスト用のサーバーイベント取得に成功すること");
            assert!(
                events
                    .iter()
                    .any(|event| matches!(event, Event::SettingsReceived { .. })),
                "クライアント SETTINGS 受信イベントが発火すること: {events:?}"
            );
            assert!(
                !events.iter().any(|event| matches!(
                    event,
                    Event::WebTransport(WebTransportEvent::BufferedStreamRejected { .. })
                )),
                "SETTINGS 受信時の再ディスパッチでは拒否イベントが発火しないこと: {events:?}"
            );
            // 再ディスパッチでセッション確立前の配送イベントが混ざらないこと
            assert_no_stream_delivery(&events);
            assert_no_session_closed(&events);
            assert_eq!(
                self.server.negotiated_wt_draft_version(),
                Some(DraftVersion::Draft17),
                "クライアント SETTINGS 受信後に draft-17 が選択されること"
            );
            events
        }

        /// クライアントから WT CONNECT を送り、サーバーに受理させる
        ///
        /// 2xx レスポンスは送信しないため、サーバー側のセッションは Pending のままになる。
        /// CONNECT 投入直後にサーバーのイベントを取得して返す。確立前のイベント列を
        /// 呼び出し側が検証できるよう、ここで破棄しない。
        fn send_connect(&mut self) -> (u64, Vec<Event>) {
            let stream_id = self
                .client
                .send_request(&connect_headers(), false)
                .expect("draft-17 の CONNECT 送信に成功すること");
            let (request, _) = self
                .client
                .take_stream_data(stream_id)
                .expect("テスト用の CONNECT ヘッダー取得に成功すること");
            self.server
                .feed_stream(stream_id, &request, false)
                .expect("draft-17 の CONNECT 受理に成功すること");
            let events = self
                .server
                .drain_events()
                .expect("テスト用のサーバーイベント取得に成功すること");
            (stream_id, events)
        }

        /// サーバーが 2xx レスポンスを生成するが、クライアントには投入しない
        ///
        /// `send_response` の時点でサーバー側のセッションは Established になり、
        /// バッファリングしていたストリーム / データグラムが配送される。
        fn send_response(&mut self, session_id: u64) -> Vec<u8> {
            let response = vec![
                Header::new(b":status", b"200").expect("テスト用のヘッダー作成に成功すること"),
            ];
            self.server
                .send_response(session_id, &response, false)
                .expect("テスト用の 200 応答送信に成功すること");
            self.server
                .take_stream_data(session_id)
                .expect("テスト用の 200 応答取得に成功すること")
                .0
        }

        /// 生成済みの 2xx レスポンスをクライアントに投入し、イベントを返す
        fn feed_response(&mut self, session_id: u64, response: &[u8]) -> Vec<Event> {
            self.client
                .feed_stream(session_id, response, false)
                .expect("クライアントの 200 応答受信に成功すること");
            self.client
                .drain_events()
                .expect("テスト用のクライアントイベント取得に成功すること")
        }

        /// 2xx レスポンスの生成とクライアントへの投入まで完了する
        fn establish(&mut self, session_id: u64) -> Vec<Event> {
            let response = self.send_response(session_id);
            self.feed_response(session_id, &response)
        }
    }

    /// イベント列から指定ストリームの WebTransport イベントを取り出す
    fn wt_events_for_stream(events: &[Event], stream_id: u64) -> Vec<&WebTransportEvent> {
        events
            .iter()
            .filter_map(|event| match event {
                Event::WebTransport(wt_event)
                    if wt_event_stream_id(wt_event) == Some(stream_id) =>
                {
                    Some(wt_event)
                }
                _ => None,
            })
            .collect()
    }

    /// WebTransport イベントが対象ストリームのものかを判定するための stream ID を返す
    fn wt_event_stream_id(event: &WebTransportEvent) -> Option<u64> {
        match event {
            WebTransportEvent::BidiStreamOpen { stream_id, .. }
            | WebTransportEvent::BidiStreamData { stream_id, .. }
            | WebTransportEvent::BidiStreamEnd { stream_id }
            | WebTransportEvent::UniStreamOpen { stream_id, .. }
            | WebTransportEvent::UniStreamData { stream_id, .. }
            | WebTransportEvent::UniStreamEnd { stream_id }
            | WebTransportEvent::BufferedStreamRejected { stream_id, .. } => Some(*stream_id),
            _ => None,
        }
    }

    /// 指定ストリームの双方向データを到着順に連結して返す
    fn bidi_data(events: &[Event], stream_id: u64) -> Vec<u8> {
        events
            .iter()
            .filter_map(|event| match event {
                Event::WebTransport(WebTransportEvent::BidiStreamData {
                    stream_id: event_stream_id,
                    data,
                }) if *event_stream_id == stream_id => Some(data.as_slice()),
                _ => None,
            })
            .flatten()
            .copied()
            .collect()
    }

    /// 指定ストリームの単方向データを到着順に連結して返す
    fn uni_data(events: &[Event], stream_id: u64) -> Vec<u8> {
        events
            .iter()
            .filter_map(|event| match event {
                Event::WebTransport(WebTransportEvent::UniStreamData {
                    stream_id: event_stream_id,
                    data,
                }) if *event_stream_id == stream_id => Some(data.as_slice()),
                _ => None,
            })
            .flatten()
            .copied()
            .collect()
    }

    /// 拒否イベントの (stream ID, error code) を到着順に返す
    fn rejected_streams(events: &[Event]) -> Vec<(u64, u64)> {
        events
            .iter()
            .filter_map(|event| match event {
                Event::WebTransport(WebTransportEvent::BufferedStreamRejected {
                    stream_id,
                    error_code,
                }) => Some((*stream_id, *error_code)),
                _ => None,
            })
            .collect()
    }

    /// Open / Data / End のいずれの配送イベントも含まれないことを検証する
    ///
    /// セッション確立前に保留されているストリームが、確立前に配送されていないことを
    /// イベント列全体に対して検証する。ストリーム ID を指定する
    /// `assert_stream_not_delivered` と異なり、対象を限定しない。
    fn assert_no_stream_delivery(events: &[Event]) {
        let delivered = events.iter().any(|event| {
            matches!(
                event,
                Event::WebTransport(
                    WebTransportEvent::BidiStreamOpen { .. }
                        | WebTransportEvent::BidiStreamData { .. }
                        | WebTransportEvent::BidiStreamEnd { .. }
                        | WebTransportEvent::UniStreamOpen { .. }
                        | WebTransportEvent::UniStreamData { .. }
                        | WebTransportEvent::UniStreamEnd { .. }
                )
            )
        });
        assert!(
            !delivered,
            "セッション確立前に Open / Data / End を発火しないこと: {events:?}"
        );
    }

    /// 指定ストリームの Open / Data / End イベントが発火していないことを検証する
    fn assert_stream_not_delivered(events: &[Event], stream_id: u64) {
        let delivered = events.iter().any(|event| match event {
            Event::WebTransport(
                WebTransportEvent::BidiStreamOpen {
                    stream_id: event_stream_id,
                    ..
                }
                | WebTransportEvent::BidiStreamData {
                    stream_id: event_stream_id,
                    ..
                }
                | WebTransportEvent::BidiStreamEnd {
                    stream_id: event_stream_id,
                }
                | WebTransportEvent::UniStreamOpen {
                    stream_id: event_stream_id,
                    ..
                }
                | WebTransportEvent::UniStreamData {
                    stream_id: event_stream_id,
                    ..
                }
                | WebTransportEvent::UniStreamEnd {
                    stream_id: event_stream_id,
                },
            ) => *event_stream_id == stream_id,
            _ => false,
        });
        assert!(
            !delivered,
            "拒否した stream {stream_id} のデータが配送されないこと: {events:?}"
        );
    }

    /// セッション終了イベントが発火していないことを検証する
    fn assert_no_session_closed(events: &[Event]) {
        assert!(
            !events.iter().any(|event| matches!(
                event,
                Event::WebTransport(WebTransportEvent::SessionClosed { .. })
            )),
            "セッションが終了していないこと: {events:?}"
        );
    }

    /// 指定セッションの SessionEstablished がイベント列に含まれることを検証する
    fn assert_session_established(events: &[Event], session_id: u64) {
        assert!(
            events.iter().any(|event| matches!(
                event,
                Event::WebTransport(WebTransportEvent::SessionEstablished {
                    session_id: event_session_id,
                    ..
                }) if *event_session_id == session_id
            )),
            "session {session_id} の SessionEstablished が発火すること: {events:?}"
        );
    }

    /// 指定ストリームの Open / Data / End がこの順で 1 回ずつ発火し、
    /// Open の session ID が一致することを検証する
    fn assert_bidi_stream_delivery(
        events: &[Event],
        stream_id: u64,
        session_id: u64,
        payload: &[u8],
    ) {
        let stream_events = wt_events_for_stream(events, stream_id);
        assert_eq!(
            stream_events.len(),
            3,
            "stream {stream_id} のイベントは Open / Data / End の 3 個であること: {stream_events:?}"
        );
        assert!(
            matches!(
                stream_events[0],
                WebTransportEvent::BidiStreamOpen {
                    stream_id: event_stream_id,
                    session_id: event_session_id,
                } if *event_stream_id == stream_id && *event_session_id == session_id
            ),
            "最初のイベントが session {session_id} の BidiStreamOpen であること: {stream_events:?}"
        );
        assert!(
            matches!(
                stream_events[1],
                WebTransportEvent::BidiStreamData {
                    stream_id: event_stream_id,
                    ..
                } if *event_stream_id == stream_id
            ),
            "2 番目のイベントが BidiStreamData であること: {stream_events:?}"
        );
        assert!(
            matches!(
                stream_events[2],
                WebTransportEvent::BidiStreamEnd {
                    stream_id: event_stream_id,
                } if *event_stream_id == stream_id
            ),
            "最後のイベントが BidiStreamEnd であること: {stream_events:?}"
        );
        assert_eq!(
            bidi_data(events, stream_id),
            payload,
            "stream {stream_id} のデータが欠損なく配送されること"
        );
    }

    /// 指定ストリームの Open / Data / End がこの順で 1 回ずつ発火し、
    /// Open の session ID が一致することを検証する
    fn assert_uni_stream_delivery(
        events: &[Event],
        stream_id: u64,
        session_id: u64,
        payload: &[u8],
    ) {
        let stream_events = wt_events_for_stream(events, stream_id);
        assert_eq!(
            stream_events.len(),
            3,
            "stream {stream_id} のイベントは Open / Data / End の 3 個であること: {stream_events:?}"
        );
        assert!(
            matches!(
                stream_events[0],
                WebTransportEvent::UniStreamOpen {
                    stream_id: event_stream_id,
                    session_id: event_session_id,
                } if *event_stream_id == stream_id && *event_session_id == session_id
            ),
            "最初のイベントが session {session_id} の UniStreamOpen であること: {stream_events:?}"
        );
        assert!(
            matches!(
                stream_events[1],
                WebTransportEvent::UniStreamData {
                    stream_id: event_stream_id,
                    ..
                } if *event_stream_id == stream_id
            ),
            "2 番目のイベントが UniStreamData であること: {stream_events:?}"
        );
        assert!(
            matches!(
                stream_events[2],
                WebTransportEvent::UniStreamEnd {
                    stream_id: event_stream_id,
                } if *event_stream_id == stream_id
            ),
            "最後のイベントが UniStreamEnd であること: {stream_events:?}"
        );
        assert_eq!(
            uni_data(events, stream_id),
            payload,
            "stream {stream_id} のデータが欠損なく配送されること"
        );
    }

    #[test]
    fn server_pre_settings_bidi_stream_count_overflow_keeps_session_usable() {
        // SETTINGS 受信前の保留ストリーム数の上限超過を検証する。
        // クライアントはサーバー SETTINGS を受信して transport parameter 検証も
        // 済ませているが、クライアント SETTINGS のサーバーへの投入だけを遅らせる。
        // サーバーは上限までクライアント開始 bidi を保留し、上限超過分だけを
        // WT_BUFFERED_STREAM_REJECTED で拒否する。その後 SETTINGS と CONNECT を
        // 処理して 2xx を完了すると、同じセッション ID の新しいストリームで
        // データを受信できることまで確認する。保留するストリーム数を制限すること
        // 自体は draft-ietf-webtrans-http3-17 Section 4.6 の MUST である。
        let mut pair = WtPair::new();

        let mut kept_payloads = Vec::new();
        for i in 0..MAX_BUFFERED_STREAMS {
            let stream_id = (i as u64 + 1) * 4;
            let payload = format!("kept-{i}").into_bytes();
            pair.server
                .feed_stream(stream_id, &bidi_stream_bytes(SESSION_ID, &payload), false)
                .expect("上限内の bidi ストリームが保留されること");
            kept_payloads.push((stream_id, payload));
        }

        let events = pair
            .server
            .drain_events()
            .expect("テスト用のサーバーイベント取得に成功すること");
        assert!(
            events.is_empty(),
            "上限内の保留ではイベントが発火しないこと: {events:?}"
        );

        // 上限 + 1 本目は拒否される。拒否はストリーム単位で、接続エラーにはならない。
        let overflow_stream_id = (MAX_BUFFERED_STREAMS as u64 + 1) * 4;
        pair.server
            .feed_stream(
                overflow_stream_id,
                &bidi_stream_bytes(SESSION_ID, b"rejected"),
                false,
            )
            .expect("上限超過の bidi ストリームでも接続エラーにならないこと");
        let events = pair
            .server
            .drain_events()
            .expect("テスト用のサーバーイベント取得に成功すること");
        assert_eq!(
            rejected_streams(&events),
            vec![(
                overflow_stream_id,
                WtErrorCode::BufferedStreamRejected as u64
            )],
            "上限超過ストリームだけが WT_BUFFERED_STREAM_REJECTED で拒否されること: {events:?}"
        );
        // 拒否イベントと同じバッチに配送イベントが混ざっていないことを確認する。
        assert_stream_not_delivered(&events, overflow_stream_id);
        assert_no_session_closed(&events);

        // 遅延させていたクライアント SETTINGS を投入する。保留していたストリームは
        // 再ディスパッチされ、Pending セッションにバッファリングされる。
        let settings_events = pair.deliver_client_settings();
        assert_no_stream_delivery(&settings_events);

        let (session_id, connect_events) = pair.send_connect();
        assert_eq!(session_id, SESSION_ID, "CONNECT は stream 0 で送られること");
        assert_no_stream_delivery(&connect_events);

        let client_events = pair.establish(session_id);
        assert_session_established(&client_events, session_id);

        let events = pair
            .server
            .drain_events()
            .expect("テスト用のサーバーイベント取得に成功すること");
        assert!(
            matches!(
                events.first(),
                Some(Event::WebTransport(
                    WebTransportEvent::SessionEstablished {
                        session_id: event_session_id,
                        ..
                    }
                )) if *event_session_id == session_id
            ),
            "セッション確立イベントが最初に発火すること: {events:?}"
        );
        let open_count = events
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    Event::WebTransport(WebTransportEvent::BidiStreamOpen {
                        session_id: event_session_id,
                        ..
                    }) if *event_session_id == session_id
                )
            })
            .count();
        assert_eq!(
            open_count, MAX_BUFFERED_STREAMS,
            "上限まで保持した全ストリームがセッション確立時に配送されること"
        );
        for (stream_id, payload) in &kept_payloads {
            assert_eq!(
                bidi_data(&events, *stream_id),
                *payload,
                "保持済み stream {stream_id} のデータが配送されること"
            );
        }
        assert_stream_not_delivered(&events, overflow_stream_id);
        assert_no_session_closed(&events);

        // 確立後は同じセッション ID の新しいストリームでデータを受信できる。
        let new_stream_id = overflow_stream_id + 4;
        pair.server
            .feed_stream(
                new_stream_id,
                &bidi_stream_bytes(session_id, b"after"),
                true,
            )
            .expect("確立後の新規 bidi ストリームが受理されること");
        let events = pair
            .server
            .drain_events()
            .expect("テスト用のサーバーイベント取得に成功すること");
        assert_bidi_stream_delivery(&events, new_stream_id, session_id, b"after");
        assert_no_session_closed(&events);
    }

    #[test]
    fn server_pre_settings_bidi_stream_data_overflow_keeps_session_usable() {
        // SETTINGS 受信前の 1 ストリームあたりのデータ量上限を検証する。
        // この上限は実装独自の DoS 対策であり、draft-ietf-webtrans-http3-17
        // Section 4.6 が MUST としているのは保留するストリーム数 / データグラム数の
        // 上限である。SETTINGS 受信前は signal value と session ID を含む受信
        // バイト列全体で判定されるため、境界値にはヘッダーの 3 バイトを含める。
        //
        // ヘッダーを含めて上限ちょうどのストリームは保持され、1 バイト超過した
        // ストリームだけが WT_BUFFERED_STREAM_REJECTED で拒否される。
        // 拒否したストリームの後続チャンクは配送されず、保持したほかのストリームは
        // セッション確立時に配送されること、拒否後も同じセッションで新しい
        // ストリームを受信できることを確認する。
        let mut pair = WtPair::new();

        let header_len = bidi_stream_bytes(SESSION_ID, &[]).len();
        assert_eq!(
            header_len, 3,
            "draft-17 の bidi ヘッダーは 3 バイトであること"
        );

        // 上限ちょうど (ヘッダー 3 バイト + ペイロード 65,533 バイト)
        let accepted_stream_id = 4;
        let accepted_payload = vec![0x41u8; MAX_BUFFERED_STREAM_BYTES - header_len];
        pair.server
            .feed_stream(
                accepted_stream_id,
                &bidi_stream_bytes(SESSION_ID, &accepted_payload),
                false,
            )
            .expect("上限ちょうどの bidi ストリームが保持されること");

        // 1 バイト超過 (ヘッダー 3 バイト + ペイロード 65,534 バイト)
        let rejected_stream_id = 8;
        let rejected_payload = vec![0x42u8; MAX_BUFFERED_STREAM_BYTES - header_len + 1];
        pair.server
            .feed_stream(
                rejected_stream_id,
                &bidi_stream_bytes(SESSION_ID, &rejected_payload),
                false,
            )
            .expect("上限超過の bidi ストリームでも接続エラーにならないこと");

        // 保持済みの別ストリーム (データ量上限の影響を受けない)
        let kept_stream_id = 12;
        pair.server
            .feed_stream(
                kept_stream_id,
                &bidi_stream_bytes(SESSION_ID, b"kept"),
                false,
            )
            .expect("上限内の bidi ストリームが保持されること");

        let events = pair
            .server
            .drain_events()
            .expect("テスト用のサーバーイベント取得に成功すること");
        assert_eq!(
            rejected_streams(&events),
            vec![(
                rejected_stream_id,
                WtErrorCode::BufferedStreamRejected as u64
            )],
            "上限を 1 バイト超えたストリームだけが拒否されること: {events:?}"
        );
        // 拒否イベントと同じバッチに配送イベントが混ざっていないことを確認する。
        assert_stream_not_delivered(&events, rejected_stream_id);
        assert_no_session_closed(&events);

        // SETTINGS 受信前の拒否では、同じ stream ID の後続チャンクも破棄される。
        pair.server
            .feed_stream(rejected_stream_id, b"later-chunk", false)
            .expect("拒否済みストリームの後続チャンクで接続エラーにならないこと");
        pair.server
            .feed_stream(rejected_stream_id, &[], true)
            .expect("拒否済みストリームの FIN で接続エラーにならないこと");
        let events = pair
            .server
            .drain_events()
            .expect("テスト用のサーバーイベント取得に成功すること");
        assert!(
            events.is_empty(),
            "拒否済みストリームの後続チャンクはイベントを発火しないこと: {events:?}"
        );

        // 遅延させていたクライアント SETTINGS を投入してセッションを確立する。
        let settings_events = pair.deliver_client_settings();
        assert_no_stream_delivery(&settings_events);
        let (session_id, connect_events) = pair.send_connect();
        assert_no_stream_delivery(&connect_events);
        pair.establish(session_id);

        let events = pair
            .server
            .drain_events()
            .expect("テスト用のサーバーイベント取得に成功すること");
        assert_session_established(&events, session_id);
        assert_eq!(
            bidi_data(&events, accepted_stream_id),
            accepted_payload,
            "上限ちょうどのストリームのデータが欠損なく配送されること"
        );
        assert_eq!(
            bidi_data(&events, kept_stream_id),
            b"kept",
            "保持済みの別ストリームのデータが配送されること"
        );
        assert_stream_not_delivered(&events, rejected_stream_id);
        assert_no_session_closed(&events);

        // 確立後は同じセッション ID の新しいストリームでデータを受信できる。
        let new_stream_id = rejected_stream_id + 8;
        pair.server
            .feed_stream(
                new_stream_id,
                &bidi_stream_bytes(session_id, b"after"),
                true,
            )
            .expect("確立後の新規 bidi ストリームが受理されること");
        let events = pair
            .server
            .drain_events()
            .expect("テスト用のサーバーイベント取得に成功すること");
        assert_bidi_stream_delivery(&events, new_stream_id, session_id, b"after");
        assert_no_session_closed(&events);

        // 新しい uni ストリームでもデータを受信できる。
        let new_uni_stream_id = 6;
        pair.server
            .feed_stream(
                new_uni_stream_id,
                &uni_stream_bytes(session_id, b"after-uni"),
                false,
            )
            .expect("確立後の新規 uni ストリームが受理されること");
        pair.server
            .feed_stream(new_uni_stream_id, &[], true)
            .expect("確立後の新規 uni ストリームの FIN が受理されること");
        let events = pair
            .server
            .drain_events()
            .expect("テスト用のサーバーイベント取得に成功すること");
        assert_uni_stream_delivery(&events, new_uni_stream_id, session_id, b"after-uni");
        assert_no_session_closed(&events);
    }

    #[test]
    fn server_pending_session_buffers_uni_and_bidi_until_established() {
        // SETTINGS 交換と transport parameter 検証を完了した後、CONNECT の受理前に
        // 到着した uni / bidi ストリームが Pending セッションに保留され、
        // セッション確立時に Open / Data / End の順で配送されることを検証する
        // (draft-ietf-webtrans-http3-17 Section 4.6)。
        // ここではクライアントが 2xx を待たずに送ったストリームが、CONNECT の
        // 処理より先にサーバーへ到着する順序を再現する。
        let mut pair = WtPair::new();
        let settings_events = pair.deliver_client_settings();
        assert_no_stream_delivery(&settings_events);

        // CONNECT はまだ送らず、session ID 0 宛の先行ストリームだけを到着させる。
        // サーバーは最初のストリームで Pending セッション 0 を自動生成する。
        // 単方向ストリーム: データと FIN を別チャンクで到着させる。
        let uni_stream_id = 6;
        pair.server
            .feed_stream(uni_stream_id, &uni_stream_bytes(SESSION_ID, b"uni"), false)
            .expect("確立前の uni ストリームが保留されること");
        pair.server
            .feed_stream(uni_stream_id, &[], true)
            .expect("確立前の uni ストリームの FIN が保留されること");

        // 双方向ストリーム: データと FIN を同じチャンクで到着させる。
        let bidi_stream_id = 4;
        pair.server
            .feed_stream(
                bidi_stream_id,
                &bidi_stream_bytes(SESSION_ID, b"bidi"),
                true,
            )
            .expect("確立前の bidi ストリームが保留されること");

        // 先行ストリームの到着時点で配送イベントが発火していないことを、
        // CONNECT 投入前のイベント列そのもので確認する。
        let pre_connect_events = pair
            .server
            .drain_events()
            .expect("テスト用のサーバーイベント取得に成功すること");
        assert!(
            pre_connect_events.is_empty(),
            "セッション確立前はイベントを発火しないこと: {pre_connect_events:?}"
        );
        assert_no_stream_delivery(&pre_connect_events);

        // 先行ストリームの到着後に CONNECT を処理する。ストリームで自動生成した
        // Pending セッション 0 が CONNECT のセッションとして再利用される。
        let (session_id, connect_events) = pair.send_connect();
        assert_eq!(
            session_id, SESSION_ID,
            "先行ストリームの session ID と CONNECT の stream ID が一致すること"
        );
        assert_no_stream_delivery(&connect_events);

        let client_events = pair.establish(session_id);
        assert_session_established(&client_events, session_id);

        let events = pair
            .server
            .drain_events()
            .expect("テスト用のサーバーイベント取得に成功すること");
        assert_session_established(&events, session_id);
        assert_uni_stream_delivery(&events, uni_stream_id, session_id, b"uni");
        assert_no_session_closed(&events);
        assert_bidi_stream_delivery(&events, bidi_stream_id, session_id, b"bidi");
        assert_no_session_closed(&events);

        // 確立後の新しい uni / bidi ストリームも通常どおり配送される。
        let new_uni_stream_id = 10;
        pair.server
            .feed_stream(
                new_uni_stream_id,
                &uni_stream_bytes(session_id, b"uni-after"),
                false,
            )
            .expect("確立後の新規 uni ストリームが受理されること");
        pair.server
            .feed_stream(new_uni_stream_id, &[], true)
            .expect("確立後の新規 uni ストリームの FIN が受理されること");
        let new_bidi_stream_id = 8;
        pair.server
            .feed_stream(
                new_bidi_stream_id,
                &bidi_stream_bytes(session_id, b"bidi-after"),
                true,
            )
            .expect("確立後の新規 bidi ストリームが受理されること");
        let events = pair
            .server
            .drain_events()
            .expect("テスト用のサーバーイベント取得に成功すること");
        assert_uni_stream_delivery(&events, new_uni_stream_id, session_id, b"uni-after");
        assert_no_session_closed(&events);
        assert_bidi_stream_delivery(&events, new_bidi_stream_id, session_id, b"bidi-after");
        assert_no_session_closed(&events);
    }

    #[test]
    fn server_pending_session_bidi_stream_count_overflow_keeps_session_usable() {
        // SETTINGS 交換後の Pending セッションでも、ストリーム数の上限超過分は
        // WT_BUFFERED_STREAM_REJECTED で拒否され、セッション確立時に配送されない。
        // 保持済みのストリームは配送され、同じセッションで新しいストリームを
        // 受信できることを確認する。保留するストリーム数を制限すること自体は
        // draft-ietf-webtrans-http3-17 Section 4.6 の MUST である。
        let mut pair = WtPair::new();
        let settings_events = pair.deliver_client_settings();
        assert_no_stream_delivery(&settings_events);

        let mut kept_payloads = Vec::new();
        for i in 0..MAX_BUFFERED_STREAMS {
            let stream_id = (i as u64 + 1) * 4;
            let payload = format!("pending-{i}").into_bytes();
            pair.server
                .feed_stream(stream_id, &bidi_stream_bytes(SESSION_ID, &payload), false)
                .expect("上限内の bidi ストリームが保留されること");
            kept_payloads.push((stream_id, payload));
        }
        let events = pair
            .server
            .drain_events()
            .expect("テスト用のサーバーイベント取得に成功すること");
        assert!(
            events.is_empty(),
            "上限内の保留ではイベントが発火しないこと: {events:?}"
        );

        let overflow_stream_id = (MAX_BUFFERED_STREAMS as u64 + 1) * 4;
        pair.server
            .feed_stream(
                overflow_stream_id,
                &bidi_stream_bytes(SESSION_ID, b"rejected"),
                false,
            )
            .expect("上限超過の bidi ストリームでも接続エラーにならないこと");
        let events = pair
            .server
            .drain_events()
            .expect("テスト用のサーバーイベント取得に成功すること");
        assert_eq!(
            rejected_streams(&events),
            vec![(
                overflow_stream_id,
                WtErrorCode::BufferedStreamRejected as u64
            )],
            "上限超過ストリームだけが WT_BUFFERED_STREAM_REJECTED で拒否されること: {events:?}"
        );
        // 拒否イベントと同じバッチに配送イベントが混ざっていないことを確認する。
        assert_stream_not_delivered(&events, overflow_stream_id);
        assert_no_session_closed(&events);

        let (session_id, connect_events) = pair.send_connect();
        assert_no_stream_delivery(&connect_events);
        pair.establish(session_id);

        let events = pair
            .server
            .drain_events()
            .expect("テスト用のサーバーイベント取得に成功すること");
        assert_session_established(&events, session_id);
        let open_count = events
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    Event::WebTransport(WebTransportEvent::BidiStreamOpen {
                        session_id: event_session_id,
                        ..
                    }) if *event_session_id == session_id
                )
            })
            .count();
        assert_eq!(
            open_count, MAX_BUFFERED_STREAMS,
            "上限まで保持した全ストリームがセッション確立時に配送されること"
        );
        for (stream_id, payload) in &kept_payloads {
            assert_eq!(
                bidi_data(&events, *stream_id),
                *payload,
                "保持済み stream {stream_id} のデータが配送されること"
            );
        }
        assert_stream_not_delivered(&events, overflow_stream_id);
        assert_no_session_closed(&events);

        let new_stream_id = overflow_stream_id + 4;
        pair.server
            .feed_stream(
                new_stream_id,
                &bidi_stream_bytes(session_id, b"after"),
                true,
            )
            .expect("確立後の新規 bidi ストリームが受理されること");
        let events = pair
            .server
            .drain_events()
            .expect("テスト用のサーバーイベント取得に成功すること");
        assert_bidi_stream_delivery(&events, new_stream_id, session_id, b"after");
        assert_no_session_closed(&events);
    }

    #[test]
    fn server_pending_session_datagram_overflow_drops_only_overflow_and_keeps_session_usable() {
        // Pending セッション中のデータグラムは上限まで保留され、上限超過分は
        // 黙って破棄される。破棄はセッション終了へ波及せず、セッション確立後は
        // 保持したデータグラムだけが配送される。自動再送は行わないため、
        // 破棄したデータグラムは再送されず、その後に送った新しいデータグラムだけが
        // 配送されることを確認する。保留するデータグラム数を制限すること自体は
        // draft-ietf-webtrans-http3-17 Section 4.6 の MUST である
        // (データグラムの形式は RFC 9297 Section 2.1)。
        let mut pair = WtPair::new();
        let settings_events = pair.deliver_client_settings();
        assert_no_stream_delivery(&settings_events);
        let (session_id, connect_events) = pair.send_connect();
        assert_no_stream_delivery(&connect_events);

        let mut kept_payloads = Vec::new();
        for i in 0..MAX_BUFFERED_DATAGRAMS {
            let payload = format!("datagram-{i}").into_bytes();
            pair.server
                .feed_datagram(&datagram_bytes(session_id, &payload))
                .expect("上限内の datagram が保留されること");
            kept_payloads.push(payload);
        }
        let dropped_payload = b"dropped".to_vec();
        pair.server
            .feed_datagram(&datagram_bytes(session_id, &dropped_payload))
            .expect("上限超過の datagram でも接続エラーにならないこと");

        let events = pair
            .server
            .drain_events()
            .expect("テスト用のサーバーイベント取得に成功すること");
        assert!(
            events.is_empty(),
            "セッション確立前は datagram を配送しないこと: {events:?}"
        );

        pair.establish(session_id);
        let events = pair
            .server
            .drain_events()
            .expect("テスト用のサーバーイベント取得に成功すること");
        assert_session_established(&events, session_id);
        let delivered: Vec<Vec<u8>> = events
            .iter()
            .filter_map(|event| match event {
                Event::WebTransport(WebTransportEvent::Datagram { payload, .. }) => {
                    Some(payload.clone())
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            delivered, kept_payloads,
            "上限まで保持した datagram が到着順に欠損なく配送されること: {events:?}"
        );
        assert!(
            !delivered.contains(&dropped_payload),
            "上限超過で破棄した datagram が配送されないこと: {events:?}"
        );
        assert_no_session_closed(&events);

        // 確立後は同じセッションで新しいデータグラムとストリームを利用できる。
        pair.server
            .feed_datagram(&datagram_bytes(session_id, b"after"))
            .expect("確立後の datagram が受理されること");
        let events = pair
            .server
            .drain_events()
            .expect("テスト用のサーバーイベント取得に成功すること");
        assert!(
            events.iter().any(|event| matches!(
                event,
                Event::WebTransport(WebTransportEvent::Datagram { payload, .. })
                    if payload == b"after"
            )),
            "確立後の新しい datagram が配送されること: {events:?}"
        );
        assert_no_session_closed(&events);

        let new_stream_id = 4;
        pair.server
            .feed_stream(
                new_stream_id,
                &bidi_stream_bytes(session_id, b"stream"),
                true,
            )
            .expect("確立後の新規 bidi ストリームが受理されること");
        let events = pair
            .server
            .drain_events()
            .expect("テスト用のサーバーイベント取得に成功すること");
        assert_bidi_stream_delivery(&events, new_stream_id, session_id, b"stream");
        assert_no_session_closed(&events);
    }

    #[test]
    fn client_pending_session_buffers_server_started_streams_and_datagrams() {
        // サーバーからの先着ケースを検証する。サーバーは send_response で
        // セッションを確立した後、生成した 2xx バイトのクライアントへの投入だけを
        // 遅らせる。クライアントの既存 Pending セッションには、2xx 受信前に
        // サーバー開始 bidi / uni ストリームとデータグラムが到着する。
        // これらはセッション確立まで保留され、確立時に配送される
        // (draft-ietf-webtrans-http3-17 Section 4.6)。
        let mut pair = WtPair::new();
        let settings_events = pair.deliver_client_settings();
        assert_no_stream_delivery(&settings_events);
        let (session_id, connect_events) = pair.send_connect();
        assert_no_stream_delivery(&connect_events);
        let response = pair.send_response(session_id);

        // サーバー開始 bidi ストリーム: データと FIN を同じチャンクで到着させる。
        let bidi_stream_id = 1;
        pair.client
            .feed_stream(
                bidi_stream_id,
                &bidi_stream_bytes(session_id, b"server-bidi"),
                true,
            )
            .expect("2xx 前のサーバー開始 bidi ストリームが保留されること");

        // サーバー開始 uni ストリーム: データと FIN を別チャンクで到着させる。
        // サーバーの制御ストリーム (stream 3) と衝突しない ID を使う。
        let uni_stream_id = 7;
        pair.client
            .feed_stream(
                uni_stream_id,
                &uni_stream_bytes(session_id, b"server-uni"),
                false,
            )
            .expect("2xx 前のサーバー開始 uni ストリームが保留されること");
        pair.client
            .feed_stream(uni_stream_id, &[], true)
            .expect("2xx 前のサーバー開始 uni ストリームの FIN が保留されること");

        let datagram_payload = b"server-datagram".to_vec();
        pair.client
            .feed_datagram(&datagram_bytes(session_id, &datagram_payload))
            .expect("2xx 前の datagram が保留されること");

        let events = pair
            .client
            .drain_events()
            .expect("テスト用のクライアントイベント取得に成功すること");
        assert!(
            events.is_empty(),
            "2xx 受信前は Open / Data / End を発火しないこと: {events:?}"
        );
        assert_no_stream_delivery(&events);

        let events = pair.feed_response(session_id, &response);
        assert_session_established(&events, session_id);
        assert_bidi_stream_delivery(&events, bidi_stream_id, session_id, b"server-bidi");
        assert_no_session_closed(&events);
        assert_uni_stream_delivery(&events, uni_stream_id, session_id, b"server-uni");
        assert_no_session_closed(&events);
        assert!(
            events.iter().any(|event| matches!(
                event,
                Event::WebTransport(WebTransportEvent::Datagram { payload, .. })
                    if *payload == datagram_payload
            )),
            "確立時に保持した datagram が配送されること: {events:?}"
        );
        assert_no_session_closed(&events);

        // 確立後は新しいサーバー開始ストリームとデータグラムを利用できる。
        let new_bidi_stream_id = 5;
        pair.client
            .feed_stream(
                new_bidi_stream_id,
                &bidi_stream_bytes(session_id, b"after"),
                true,
            )
            .expect("確立後のサーバー開始 bidi ストリームが受理されること");
        pair.client
            .feed_datagram(&datagram_bytes(session_id, b"after-datagram"))
            .expect("確立後の datagram が受理されること");
        let events = pair
            .client
            .drain_events()
            .expect("テスト用のクライアントイベント取得に成功すること");
        assert_bidi_stream_delivery(&events, new_bidi_stream_id, session_id, b"after");
        assert_no_session_closed(&events);
        assert!(
            events.iter().any(|event| matches!(
                event,
                Event::WebTransport(WebTransportEvent::Datagram { payload, .. })
                    if payload == b"after-datagram"
            )),
            "確立後の新しい datagram が配送されること: {events:?}"
        );
    }

    #[test]
    fn client_pending_session_bidi_stream_count_overflow_keeps_session_usable() {
        // クライアント側の Pending セッションでも、2xx 前に到着したサーバー開始
        // bidi ストリームが上限を超えると WT_BUFFERED_STREAM_REJECTED で拒否され、
        // セッション確立時に配送されない。保持済みのストリームは配送され、
        // 同じセッションで新しいストリームを受信できることを確認する。保留する
        // ストリーム数を制限すること自体は draft-ietf-webtrans-http3-17
        // Section 4.6 の MUST である。
        let mut pair = WtPair::new();
        let settings_events = pair.deliver_client_settings();
        assert_no_stream_delivery(&settings_events);
        let (session_id, connect_events) = pair.send_connect();
        assert_no_stream_delivery(&connect_events);
        let response = pair.send_response(session_id);

        let mut kept_payloads = Vec::new();
        for i in 0..MAX_BUFFERED_STREAMS {
            let stream_id = 1 + (i as u64) * 4;
            let payload = format!("client-pending-{i}").into_bytes();
            pair.client
                .feed_stream(stream_id, &bidi_stream_bytes(session_id, &payload), false)
                .expect("上限内のサーバー開始 bidi ストリームが保留されること");
            kept_payloads.push((stream_id, payload));
        }
        let events = pair
            .client
            .drain_events()
            .expect("テスト用のクライアントイベント取得に成功すること");
        assert!(
            events.is_empty(),
            "2xx 受信前はイベントが発火しないこと: {events:?}"
        );

        let overflow_stream_id = 1 + (MAX_BUFFERED_STREAMS as u64) * 4;
        pair.client
            .feed_stream(
                overflow_stream_id,
                &bidi_stream_bytes(session_id, b"rejected"),
                false,
            )
            .expect("上限超過の bidi ストリームでも接続エラーにならないこと");
        let events = pair
            .client
            .drain_events()
            .expect("テスト用のクライアントイベント取得に成功すること");
        assert_eq!(
            rejected_streams(&events),
            vec![(
                overflow_stream_id,
                WtErrorCode::BufferedStreamRejected as u64
            )],
            "上限超過ストリームだけが WT_BUFFERED_STREAM_REJECTED で拒否されること: {events:?}"
        );
        // 拒否イベントと同じバッチに配送イベントが混ざっていないことを確認する。
        assert_stream_not_delivered(&events, overflow_stream_id);
        assert_no_session_closed(&events);

        let events = pair.feed_response(session_id, &response);
        assert_session_established(&events, session_id);
        let open_count = events
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    Event::WebTransport(WebTransportEvent::BidiStreamOpen {
                        session_id: event_session_id,
                        ..
                    }) if *event_session_id == session_id
                )
            })
            .count();
        assert_eq!(
            open_count, MAX_BUFFERED_STREAMS,
            "上限まで保持した全ストリームがセッション確立時に配送されること"
        );
        for (stream_id, payload) in &kept_payloads {
            assert_eq!(
                bidi_data(&events, *stream_id),
                *payload,
                "保持済み stream {stream_id} のデータが配送されること"
            );
        }
        assert_stream_not_delivered(&events, overflow_stream_id);
        assert_no_session_closed(&events);

        let new_stream_id = overflow_stream_id + 4;
        pair.client
            .feed_stream(
                new_stream_id,
                &bidi_stream_bytes(session_id, b"after"),
                true,
            )
            .expect("確立後のサーバー開始 bidi ストリームが受理されること");
        let events = pair
            .client
            .drain_events()
            .expect("テスト用のクライアントイベント取得に成功すること");
        assert_bidi_stream_delivery(&events, new_stream_id, session_id, b"after");
        assert_no_session_closed(&events);

        // 新しい uni ストリームでもデータを受信できる。
        // サーバー開始 uni は 3 mod 4 で、制御ストリーム (stream 3) と衝突しない ID を使う。
        let new_uni_stream_id = 407;
        pair.client
            .feed_stream(
                new_uni_stream_id,
                &uni_stream_bytes(session_id, b"after-uni"),
                false,
            )
            .expect("確立後のサーバー開始 uni ストリームが受理されること");
        pair.client
            .feed_stream(new_uni_stream_id, &[], true)
            .expect("確立後のサーバー開始 uni ストリームの FIN が受理されること");
        let events = pair
            .client
            .drain_events()
            .expect("テスト用のクライアントイベント取得に成功すること");
        assert_uni_stream_delivery(&events, new_uni_stream_id, session_id, b"after-uni");
        assert_no_session_closed(&events);
    }

    #[test]
    fn client_pending_session_datagram_overflow_drops_only_overflow_and_keeps_session_usable() {
        // クライアント側の Pending セッションでも、2xx 前に到着したデータグラムは
        // 上限まで保留され、上限超過分は黙って破棄される。破棄はセッション終了へ
        // 波及せず、確立後は新しいデータグラムを利用できる。保留するデータグラム数を
        // 制限すること自体は draft-ietf-webtrans-http3-17 Section 4.6 の MUST である
        // (データグラムの形式は RFC 9297 Section 2.1)。
        let mut pair = WtPair::new();
        let settings_events = pair.deliver_client_settings();
        assert_no_stream_delivery(&settings_events);
        let (session_id, connect_events) = pair.send_connect();
        assert_no_stream_delivery(&connect_events);
        let response = pair.send_response(session_id);

        let mut kept_payloads = Vec::new();
        for i in 0..MAX_BUFFERED_DATAGRAMS {
            let payload = format!("client-datagram-{i}").into_bytes();
            pair.client
                .feed_datagram(&datagram_bytes(session_id, &payload))
                .expect("上限内の datagram が保留されること");
            kept_payloads.push(payload);
        }
        let dropped_payload = b"client-dropped".to_vec();
        pair.client
            .feed_datagram(&datagram_bytes(session_id, &dropped_payload))
            .expect("上限超過の datagram でも接続エラーにならないこと");

        let events = pair
            .client
            .drain_events()
            .expect("テスト用のクライアントイベント取得に成功すること");
        assert!(
            events.is_empty(),
            "2xx 受信前は datagram を配送しないこと: {events:?}"
        );

        let events = pair.feed_response(session_id, &response);
        assert_session_established(&events, session_id);
        let delivered: Vec<Vec<u8>> = events
            .iter()
            .filter_map(|event| match event {
                Event::WebTransport(WebTransportEvent::Datagram { payload, .. }) => {
                    Some(payload.clone())
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            delivered, kept_payloads,
            "上限まで保持した datagram が到着順に欠損なく配送されること: {events:?}"
        );
        assert!(
            !delivered.contains(&dropped_payload),
            "上限超過で破棄した datagram が配送されないこと: {events:?}"
        );
        assert_no_session_closed(&events);

        pair.client
            .feed_datagram(&datagram_bytes(session_id, b"after"))
            .expect("確立後の datagram が受理されること");
        let events = pair
            .client
            .drain_events()
            .expect("テスト用のクライアントイベント取得に成功すること");
        assert!(
            events.iter().any(|event| matches!(
                event,
                Event::WebTransport(WebTransportEvent::Datagram { payload, .. })
                    if payload == b"after"
            )),
            "確立後の新しい datagram が配送されること: {events:?}"
        );
        assert_no_session_closed(&events);
    }

    #[test]
    fn server_pending_session_bidi_stream_data_overflow_keeps_session_usable() {
        // SETTINGS 交換後、CONNECT 受理前の Pending セッションで、1 ストリームあたりの
        // 受信データ量の上限を累積で超えるケースを検証する。上限判定は累積の
        // ペイロード長で行われるため、1 回の大きなチャンクではなく 2 チャンクに
        // 分けて上限を跨ぐ。この上限は実装独自の DoS 対策であり、draft-17
        // Section 4.6 が MUST としているのは保留するストリーム数 / データグラム数の
        // 上限である。
        //
        // 上限を超えたストリームだけが WT_BUFFERED_STREAM_REJECTED で拒否され、
        // セッションは継続し、確立後に拒否分が配送されないこと、別ストリームでは
        // 引き続き受信できることを確認する。
        let mut pair = WtPair::new();
        let settings_events = pair.deliver_client_settings();
        assert_no_stream_delivery(&settings_events);
        let (session_id, connect_events) = pair.send_connect();
        assert_no_stream_delivery(&connect_events);

        // 1 チャンク目: 上限 (64 KiB) 未満の 60,000 バイト。
        // 2 チャンク目と合わせて累積 66,000 バイト (> 64 KiB) にする。
        const FIRST_CHUNK_LEN: usize = 60_000;
        const SECOND_CHUNK_LEN: usize = 6_000;
        const {
            assert!(
                FIRST_CHUNK_LEN + SECOND_CHUNK_LEN > MAX_BUFFERED_STREAM_BYTES,
                "累積が 1 ストリームあたりの上限を超えること"
            );
        }
        let rejected_stream_id = 4;
        let first_chunk = vec![0x5a; FIRST_CHUNK_LEN];
        pair.server
            .feed_stream(
                rejected_stream_id,
                &bidi_stream_bytes(session_id, &first_chunk),
                false,
            )
            .expect("上限未満の bidi ストリームが保留されること");
        let events = pair
            .server
            .drain_events()
            .expect("テスト用のサーバーイベント取得に成功すること");
        assert!(
            events.is_empty(),
            "上限未満の累積ではイベントが発火しないこと: {events:?}"
        );

        // 2 チャンク目: 6,000 バイトを追記して累積 66,000 バイトにする。
        // 累積が上限を超えるため、この時点で拒否される。
        let second_chunk = vec![0x5b; SECOND_CHUNK_LEN];
        pair.server
            .feed_stream(rejected_stream_id, &second_chunk, false)
            .expect("上限超過の追記でも接続エラーにならないこと");
        let events = pair
            .server
            .drain_events()
            .expect("テスト用のサーバーイベント取得に成功すること");
        assert_eq!(
            rejected_streams(&events),
            vec![(
                rejected_stream_id,
                WtErrorCode::BufferedStreamRejected as u64
            )],
            "累積が上限を超えたストリームだけが拒否されること: {events:?}"
        );
        // 拒否イベントと同じバッチに配送イベントが混ざっていないことを確認する。
        assert_stream_not_delivered(&events, rejected_stream_id);
        assert_no_session_closed(&events);

        // 拒否されたストリームとは別のストリームは引き続き保留できる。
        let kept_stream_id = 8;
        pair.server
            .feed_stream(
                kept_stream_id,
                &bidi_stream_bytes(session_id, b"kept"),
                true,
            )
            .expect("上限内の bidi ストリームが保留されること");

        pair.establish(session_id);
        let events = pair
            .server
            .drain_events()
            .expect("テスト用のサーバーイベント取得に成功すること");
        assert_session_established(&events, session_id);
        assert_bidi_stream_delivery(&events, kept_stream_id, session_id, b"kept");
        assert_no_session_closed(&events);
        assert_stream_not_delivered(&events, rejected_stream_id);
        assert_no_session_closed(&events);

        // 確立後は同じセッションの別ストリームで再送受信できる。
        let new_stream_id = rejected_stream_id + 8;
        pair.server
            .feed_stream(
                new_stream_id,
                &bidi_stream_bytes(session_id, b"after"),
                true,
            )
            .expect("確立後の新規 bidi ストリームが受理されること");
        let events = pair
            .server
            .drain_events()
            .expect("テスト用のサーバーイベント取得に成功すること");
        assert_bidi_stream_delivery(&events, new_stream_id, session_id, b"after");
        assert_no_session_closed(&events);
    }

    #[test]
    fn server_pending_session_uni_stream_data_overflow_keeps_session_usable() {
        // Pending セッションの単方向ストリームでも、1 ストリームあたりの受信
        // データ量の上限を累積で超えた場合の拒否を検証する。単方向ストリームは
        // ヘッダー解決と後続チャンクの追記で bidi とは別の経路を通るため、
        // bidi のテストとは独立に確認する。上限は実装独自の DoS 対策であり、
        // draft-17 Section 4.6 が MUST としているのは保留するストリーム数 /
        // データグラム数の上限である。
        //
        // 上限を超えたストリームだけが WT_BUFFERED_STREAM_REJECTED で拒否され、
        // セッションは継続し、確立後に拒否分が配送されないこと、別ストリームでは
        // 引き続き受信できることを確認する。
        let mut pair = WtPair::new();
        let settings_events = pair.deliver_client_settings();
        assert_no_stream_delivery(&settings_events);
        let (session_id, connect_events) = pair.send_connect();
        assert_no_stream_delivery(&connect_events);

        // クライアント開始 uni は 2 mod 4。stream 2 は制御ストリームなので 6 以降を使う。
        const FIRST_CHUNK_LEN: usize = 60_000;
        const SECOND_CHUNK_LEN: usize = 6_000;
        const {
            assert!(
                FIRST_CHUNK_LEN + SECOND_CHUNK_LEN > MAX_BUFFERED_STREAM_BYTES,
                "累積が 1 ストリームあたりの上限を超えること"
            );
        }
        let rejected_stream_id = 6;
        let first_chunk = vec![0x6a; FIRST_CHUNK_LEN];
        pair.server
            .feed_stream(
                rejected_stream_id,
                &uni_stream_bytes(session_id, &first_chunk),
                false,
            )
            .expect("上限未満の uni ストリームが保留されること");
        let events = pair
            .server
            .drain_events()
            .expect("テスト用のサーバーイベント取得に成功すること");
        assert!(
            events.is_empty(),
            "上限未満の累積ではイベントが発火しないこと: {events:?}"
        );

        // 2 チャンク目: 6,000 バイトを追記して累積 66,000 バイトにする。
        let second_chunk = vec![0x6b; SECOND_CHUNK_LEN];
        pair.server
            .feed_stream(rejected_stream_id, &second_chunk, false)
            .expect("上限超過の追記でも接続エラーにならないこと");
        let events = pair
            .server
            .drain_events()
            .expect("テスト用のサーバーイベント取得に成功すること");
        assert_eq!(
            rejected_streams(&events),
            vec![(
                rejected_stream_id,
                WtErrorCode::BufferedStreamRejected as u64
            )],
            "累積が上限を超えた uni ストリームだけが拒否されること: {events:?}"
        );
        // 拒否イベントと同じバッチに配送イベントが混ざっていないことを確認する。
        assert_stream_not_delivered(&events, rejected_stream_id);
        assert_no_session_closed(&events);

        // 拒否されたストリームとは別のストリームは引き続き保留できる。
        let kept_stream_id = 10;
        pair.server
            .feed_stream(
                kept_stream_id,
                &uni_stream_bytes(session_id, b"kept-uni"),
                false,
            )
            .expect("上限内の uni ストリームが保留されること");
        pair.server
            .feed_stream(kept_stream_id, &[], true)
            .expect("上限内の uni ストリームの FIN が保留されること");

        pair.establish(session_id);
        let events = pair
            .server
            .drain_events()
            .expect("テスト用のサーバーイベント取得に成功すること");
        assert_session_established(&events, session_id);
        assert_uni_stream_delivery(&events, kept_stream_id, session_id, b"kept-uni");
        assert_no_session_closed(&events);
        assert_stream_not_delivered(&events, rejected_stream_id);
        assert_no_session_closed(&events);

        // 確立後は同じセッションの新しい uni / bidi ストリームで再送受信できる。
        let new_uni_stream_id = rejected_stream_id + 8;
        pair.server
            .feed_stream(
                new_uni_stream_id,
                &uni_stream_bytes(session_id, b"uni-after"),
                false,
            )
            .expect("確立後の新規 uni ストリームが受理されること");
        pair.server
            .feed_stream(new_uni_stream_id, &[], true)
            .expect("確立後の新規 uni ストリームの FIN が受理されること");
        let new_bidi_stream_id = 4;
        pair.server
            .feed_stream(
                new_bidi_stream_id,
                &bidi_stream_bytes(session_id, b"bidi-after"),
                true,
            )
            .expect("確立後の新規 bidi ストリームが受理されること");
        let events = pair
            .server
            .drain_events()
            .expect("テスト用のサーバーイベント取得に成功すること");
        assert_uni_stream_delivery(&events, new_uni_stream_id, session_id, b"uni-after");
        assert_no_session_closed(&events);
        assert_bidi_stream_delivery(&events, new_bidi_stream_id, session_id, b"bidi-after");
        assert_no_session_closed(&events);
    }

    #[test]
    fn server_pending_session_first_chunk_data_overflow_keeps_session_usable() {
        // Pending セッションで、最初のチャンクだけで 1 ストリームあたりの上限を
        // 超える bidi / uni ストリームが拒否されることを検証する。ヘッダー解決時の
        // 初回ペイロード判定は、後続チャンクの追記判定とは別の経路を通るため、
        // 独立に確認する。上限は実装独自の DoS 対策であり、draft-17 Section 4.6 が
        // MUST としているのは保留するストリーム数 / データグラム数の上限である。
        //
        // 上限ちょうどのストリームは受理され、1 バイト超過したストリームだけが
        // WT_BUFFERED_STREAM_REJECTED で拒否されること、セッションが継続し、
        // 確立後に超過分が配送されないことを確認する。
        let mut pair = WtPair::new();
        let settings_events = pair.deliver_client_settings();
        assert_no_stream_delivery(&settings_events);
        let (session_id, connect_events) = pair.send_connect();
        assert_no_stream_delivery(&connect_events);

        // 上限ちょうど: ペイロード長がちょうど上限 (ヘッダーは含めない)
        let accepted_payload = payload_bytes(MAX_BUFFERED_STREAM_BYTES);
        let accepted_bidi_stream_id = 4;
        pair.server
            .feed_stream(
                accepted_bidi_stream_id,
                &bidi_stream_bytes(session_id, &accepted_payload),
                false,
            )
            .expect("上限ちょうどの bidi ストリームが保留されること");
        // FIN の処理を独立して検証するため、FIN は別チャンクで送る
        pair.server
            .feed_stream(accepted_bidi_stream_id, &[], true)
            .expect("上限ちょうどの bidi ストリームの FIN が保留されること");

        // 1 バイト超過: 最初のチャンクだけで上限を超える
        let rejected_payload = payload_bytes(MAX_BUFFERED_STREAM_BYTES + 1);
        let rejected_bidi_stream_id = 8;
        pair.server
            .feed_stream(
                rejected_bidi_stream_id,
                &bidi_stream_bytes(session_id, &rejected_payload),
                false,
            )
            .expect("上限超過の初回チャンクでも接続エラーにならないこと");
        let rejected_uni_stream_id = 10;
        pair.server
            .feed_stream(
                rejected_uni_stream_id,
                &uni_stream_bytes(session_id, &rejected_payload),
                false,
            )
            .expect("上限超過の uni 初回チャンクでも接続エラーにならないこと");

        let events = pair
            .server
            .drain_events()
            .expect("テスト用のサーバーイベント取得に成功すること");
        let mut rejected = rejected_streams(&events);
        rejected.sort_unstable();
        assert_eq!(
            rejected,
            vec![
                (
                    rejected_bidi_stream_id,
                    WtErrorCode::BufferedStreamRejected as u64
                ),
                (
                    rejected_uni_stream_id,
                    WtErrorCode::BufferedStreamRejected as u64
                ),
            ],
            "最初のチャンクで上限を超えたストリームだけが拒否されること: {events:?}"
        );
        assert_stream_not_delivered(&events, rejected_bidi_stream_id);
        assert_stream_not_delivered(&events, rejected_uni_stream_id);
        assert_no_session_closed(&events);

        pair.establish(session_id);
        let events = pair
            .server
            .drain_events()
            .expect("テスト用のサーバーイベント取得に成功すること");
        assert_session_established(&events, session_id);
        assert_bidi_stream_delivery(
            &events,
            accepted_bidi_stream_id,
            session_id,
            &accepted_payload,
        );
        assert_no_session_closed(&events);
        assert_stream_not_delivered(&events, rejected_bidi_stream_id);
        assert_stream_not_delivered(&events, rejected_uni_stream_id);

        // 確立後は同じセッションの新しいストリームで再送受信できる。
        let new_stream_id = 12;
        pair.server
            .feed_stream(
                new_stream_id,
                &bidi_stream_bytes(session_id, b"after"),
                true,
            )
            .expect("確立後の新規 bidi ストリームが受理されること");
        let events = pair
            .server
            .drain_events()
            .expect("テスト用のサーバーイベント取得に成功すること");
        assert_bidi_stream_delivery(&events, new_stream_id, session_id, b"after");
        assert_no_session_closed(&events);
    }

    #[test]
    fn server_pending_session_uni_stream_count_overflow_keeps_session_usable() {
        // SETTINGS 交換後の Pending セッションで、クライアント開始 uni ストリームの
        // 数の上限超過を検証する。uni の拒否判定は bidi とは別の経路
        // (`resolve_wt_uni_stream_session_id`) を通るため、bidi のテストとは
        // 独立に確認する。クライアント開始 uni は 2 mod 4 で、stream 2 は制御
        // ストリームなので stream 6 以降を使う。
        //
        // 上限を超えたストリームだけが WT_BUFFERED_STREAM_REJECTED で拒否され、
        // セッションは継続し、確立後に拒否分が配送されないこと、新しい uni / bidi
        // ストリームで再送受信できることを確認する。保留するストリーム数を制限する
        // こと自体は draft-17 Section 4.6 の MUST である。
        let mut pair = WtPair::new();
        let settings_events = pair.deliver_client_settings();
        assert_no_stream_delivery(&settings_events);
        let (session_id, connect_events) = pair.send_connect();
        assert_no_stream_delivery(&connect_events);

        let mut kept_payloads = Vec::new();
        for i in 0..MAX_BUFFERED_STREAMS {
            let stream_id = 6 + (i as u64) * 4;
            let payload = format!("uni-pending-{i}").into_bytes();
            pair.server
                .feed_stream(stream_id, &uni_stream_bytes(session_id, &payload), false)
                .expect("上限内の uni ストリームが保留されること");
            kept_payloads.push((stream_id, payload));
        }
        let events = pair
            .server
            .drain_events()
            .expect("テスト用のサーバーイベント取得に成功すること");
        assert!(
            events.is_empty(),
            "上限内の保留ではイベントが発火しないこと: {events:?}"
        );

        // 上限 + 1 本目の uni ストリームは拒否される。
        let overflow_stream_id = 6 + (MAX_BUFFERED_STREAMS as u64) * 4;
        pair.server
            .feed_stream(
                overflow_stream_id,
                &uni_stream_bytes(session_id, b"rejected"),
                false,
            )
            .expect("上限超過の uni ストリームでも接続エラーにならないこと");
        let events = pair
            .server
            .drain_events()
            .expect("テスト用のサーバーイベント取得に成功すること");
        assert_eq!(
            rejected_streams(&events),
            vec![(
                overflow_stream_id,
                WtErrorCode::BufferedStreamRejected as u64
            )],
            "上限超過の uni ストリームだけが WT_BUFFERED_STREAM_REJECTED で拒否されること: {events:?}"
        );
        // 拒否イベントと同じバッチに配送イベントが混ざっていないことを確認する。
        assert_stream_not_delivered(&events, overflow_stream_id);
        assert_no_session_closed(&events);

        pair.establish(session_id);
        let events = pair
            .server
            .drain_events()
            .expect("テスト用のサーバーイベント取得に成功すること");
        assert_session_established(&events, session_id);
        let open_count = events
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    Event::WebTransport(WebTransportEvent::UniStreamOpen {
                        session_id: event_session_id,
                        ..
                    }) if *event_session_id == session_id
                )
            })
            .count();
        assert_eq!(
            open_count, MAX_BUFFERED_STREAMS,
            "上限まで保持した全 uni ストリームがセッション確立時に配送されること"
        );
        for (stream_id, payload) in &kept_payloads {
            assert_eq!(
                uni_data(&events, *stream_id),
                *payload,
                "保持済み uni stream {stream_id} のデータが配送されること"
            );
        }
        assert_stream_not_delivered(&events, overflow_stream_id);
        assert_no_session_closed(&events);

        // 確立後は拒否されたストリームではなく、新しい uni / bidi ストリームで
        // 再送受信できる。
        let new_uni_stream_id = overflow_stream_id + 4;
        pair.server
            .feed_stream(
                new_uni_stream_id,
                &uni_stream_bytes(session_id, b"uni-after"),
                false,
            )
            .expect("確立後の新規 uni ストリームが受理されること");
        pair.server
            .feed_stream(new_uni_stream_id, &[], true)
            .expect("確立後の新規 uni ストリームの FIN が受理されること");
        let new_bidi_stream_id = 4;
        pair.server
            .feed_stream(
                new_bidi_stream_id,
                &bidi_stream_bytes(session_id, b"bidi-after"),
                true,
            )
            .expect("確立後の新規 bidi ストリームが受理されること");
        let events = pair
            .server
            .drain_events()
            .expect("テスト用のサーバーイベント取得に成功すること");
        assert_uni_stream_delivery(&events, new_uni_stream_id, session_id, b"uni-after");
        assert_no_session_closed(&events);
        assert_bidi_stream_delivery(&events, new_bidi_stream_id, session_id, b"bidi-after");
        assert_no_session_closed(&events);
    }
}
