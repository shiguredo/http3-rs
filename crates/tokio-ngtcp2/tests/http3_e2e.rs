//! HTTP/3 クライアント/サーバー I/O テスト
//!
//! 実際のネットワーク I/O を使用した HTTP/3 リクエスト/レスポンステスト

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use rcgen::{CertificateParams, KeyPair};
use serial_test::serial;
use tokio::time::timeout;

use shiguredo_ngtcp2::{Header, Http3Event};
use tokio_ngtcp2::{Client, Server};

/// テスト用の証明書と秘密鍵を動的に生成
fn generate_test_certs() -> (PathBuf, PathBuf) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let unique_id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let temp_dir =
        std::env::temp_dir().join(format!("http3_test_{}_{}", std::process::id(), unique_id));
    std::fs::create_dir_all(&temp_dir).expect("一時ディレクトリ作成失敗");

    let cert_path = temp_dir.join("cert.pem");
    let key_path = temp_dir.join("key.pem");

    // 証明書パラメータを設定
    let mut params =
        CertificateParams::new(vec!["localhost".to_string()]).expect("CertificateParams 作成失敗");
    params.distinguished_name.push(
        rcgen::DnType::CommonName,
        rcgen::DnValue::Utf8String("localhost".to_string()),
    );

    // 鍵ペアを生成
    let key_pair = KeyPair::generate().expect("鍵ペア生成失敗");

    // 自己署名証明書を生成
    let cert = params.self_signed(&key_pair).expect("証明書生成失敗");

    // PEM 形式で保存
    std::fs::write(&cert_path, cert.pem()).expect("証明書ファイル書き込み失敗");
    std::fs::write(&key_path, key_pair.serialize_pem()).expect("秘密鍵ファイル書き込み失敗");

    (cert_path, key_path)
}

/// HTTP/3 GET リクエスト/レスポンステスト
#[serial]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_http3_get_request() {
    let (cert_path, key_path) = generate_test_certs();

    let request_received = Arc::new(AtomicBool::new(false));
    let request_received_clone = request_received.clone();

    // サーバーを起動
    let mut server = Server::bind(
        "127.0.0.1:0".parse().expect("test must succeed"),
        &cert_path,
        &key_path,
        None,
        None,
    )
    .await
    .expect("サーバー作成失敗");

    let server_addr = server.local_addr();
    eprintln!("[test] サーバー起動: {}", server_addr);

    // サーバータスク
    let server_task = tokio::spawn(async move {
        let result = timeout(
            Duration::from_secs(10),
            server.run(move |addr, event| {
                eprintln!("[server] イベント受信: addr = {}", addr);
                match event {
                    Http3Event::HeadersBegin { stream_id } => {
                        eprintln!("[server] HeadersBegin: stream_id = {}", stream_id);
                        None
                    }
                    Http3Event::Header { stream_id, header } => {
                        eprintln!(
                            "[server] Header: stream_id = {}, name = {:?}, value = {:?}",
                            stream_id,
                            header.name_str(),
                            header.value_str()
                        );
                        None
                    }
                    Http3Event::HeadersEnd { stream_id, fin } => {
                        eprintln!(
                            "[server] HeadersEnd: stream_id = {}, fin = {}",
                            stream_id, fin
                        );
                        request_received_clone.store(true, Ordering::SeqCst);
                        // 200 OK レスポンスを返す
                        Some((vec![Header::status(200)], Vec::new()))
                    }
                    Http3Event::Data { stream_id, data } => {
                        eprintln!(
                            "[server] Data: stream_id = {}, len = {}",
                            stream_id,
                            data.len()
                        );
                        None
                    }
                    _ => {
                        eprintln!("[server] Other event: {:?}", event);
                        None
                    }
                }
            }),
        )
        .await;

        match result {
            Ok(r) => r,
            Err(_) => {
                eprintln!("[server] タイムアウト");
                Ok(())
            }
        }
    });

    // クライアントを作成
    let client_result = timeout(Duration::from_secs(10), async {
        let mut client = Client::connect_insecure_default(server_addr, "localhost")
            .await
            .expect("クライアント作成失敗");

        // ハンドシェイク
        client.handshake().await.expect("ハンドシェイク失敗");
        eprintln!("[client] ハンドシェイク完了");

        // GET リクエストを送信
        let headers = vec![
            Header::method("GET"),
            Header::scheme("https"),
            Header::authority(&format!("localhost:{}", server_addr.port())),
            Header::path("/"),
        ];

        let stream_id = client.send_request(&headers).expect("リクエスト送信失敗");
        eprintln!("[client] リクエスト送信: stream_id = {}", stream_id);

        // HTTP/3 データを送信
        client.flush().await.expect("フラッシュ失敗");

        // レスポンスを待つ (簡易実装: 少し待機)
        tokio::time::sleep(Duration::from_millis(500)).await;

        stream_id
    })
    .await;

    server_task.abort();

    match client_result {
        Ok(stream_id) => {
            eprintln!(
                "[test] HTTP/3 GET リクエストテスト完了: stream_id = {}",
                stream_id
            );
            assert!(
                request_received.load(Ordering::SeqCst),
                "サーバーがリクエストを受信するべき"
            );
        }
        Err(_) => {
            panic!("テストタイムアウト");
        }
    }
}

/// HTTP/3 POST リクエストテスト
#[serial]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_http3_post_request() {
    let (cert_path, key_path) = generate_test_certs();

    let request_received = Arc::new(AtomicBool::new(false));
    let request_received_clone = request_received.clone();

    // サーバーを起動
    let mut server = Server::bind(
        "127.0.0.1:0".parse().expect("test must succeed"),
        &cert_path,
        &key_path,
        None,
        None,
    )
    .await
    .expect("サーバー作成失敗");

    let server_addr = server.local_addr();

    // サーバータスク
    let server_task = tokio::spawn(async move {
        let _ = timeout(
            Duration::from_secs(10),
            server.run(move |_addr, event| match event {
                Http3Event::HeadersEnd { stream_id, .. } => {
                    eprintln!("[server] POST リクエスト受信: stream_id = {}", stream_id);
                    request_received_clone.store(true, Ordering::SeqCst);
                    Some((vec![Header::status(201)], Vec::new()))
                }
                _ => None,
            }),
        )
        .await;
    });

    // クライアントを作成
    let client_result = timeout(Duration::from_secs(10), async {
        let mut client = Client::connect_insecure_default(server_addr, "localhost")
            .await
            .expect("クライアント作成失敗");

        // ハンドシェイク
        client.handshake().await.expect("ハンドシェイク失敗");

        // POST リクエストを送信
        let headers = vec![
            Header::method("POST"),
            Header::scheme("https"),
            Header::authority(&format!("localhost:{}", server_addr.port())),
            Header::path("/api/data"),
            Header::new(b"content-type", b"application/json"),
        ];

        let stream_id = client.send_request(&headers).expect("リクエスト送信失敗");
        eprintln!("[client] POST リクエスト送信: stream_id = {}", stream_id);

        // HTTP/3 データを送信
        client.flush().await.expect("フラッシュ失敗");

        tokio::time::sleep(Duration::from_millis(500)).await;

        stream_id
    })
    .await;

    server_task.abort();

    assert!(client_result.is_ok(), "テストがタイムアウトしないこと");
    assert!(
        request_received.load(Ordering::SeqCst),
        "サーバーがリクエストを受信するべき"
    );

    eprintln!("[test] HTTP/3 POST リクエストテスト完了");
}

/// 複数リクエストの並行処理テスト
#[serial]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_http3_concurrent_requests() {
    let (cert_path, key_path) = generate_test_certs();

    use std::sync::atomic::AtomicUsize;
    let request_count = Arc::new(AtomicUsize::new(0));
    let request_count_clone = request_count.clone();

    // サーバーを起動
    let mut server = Server::bind(
        "127.0.0.1:0".parse().expect("test must succeed"),
        &cert_path,
        &key_path,
        None,
        None,
    )
    .await
    .expect("サーバー作成失敗");

    let server_addr = server.local_addr();

    // サーバータスク
    let server_task = tokio::spawn(async move {
        let _ = timeout(
            Duration::from_secs(10),
            server.run(move |_addr, event| {
                if let Http3Event::HeadersEnd { stream_id, .. } = event {
                    let count = request_count_clone.fetch_add(1, Ordering::SeqCst) + 1;
                    eprintln!(
                        "[server] リクエスト {} 受信: stream_id = {}",
                        count, stream_id
                    );
                    Some((vec![Header::status(200)], Vec::new()))
                } else {
                    None
                }
            }),
        )
        .await;
    });

    // クライアントを作成して複数リクエストを送信
    let client_result = timeout(Duration::from_secs(10), async {
        let mut client = Client::connect_insecure_default(server_addr, "localhost")
            .await
            .expect("クライアント作成失敗");

        // ハンドシェイク
        client.handshake().await.expect("ハンドシェイク失敗");

        // 複数のリクエストを送信
        let request_paths = ["/", "/api/users", "/api/data"];
        let mut stream_ids = Vec::new();

        for path in &request_paths {
            let headers = vec![
                Header::method("GET"),
                Header::scheme("https"),
                Header::authority(&format!("localhost:{}", server_addr.port())),
                Header::path(path),
            ];

            let stream_id = client.send_request(&headers).expect("リクエスト送信失敗");
            stream_ids.push(stream_id);
            eprintln!(
                "[client] リクエスト送信: path = {}, stream_id = {}",
                path, stream_id
            );
        }

        // HTTP/3 データを送信
        client.flush().await.expect("フラッシュ失敗");

        tokio::time::sleep(Duration::from_millis(500)).await;

        stream_ids
    })
    .await;

    server_task.abort();

    match client_result {
        Ok(stream_ids) => {
            eprintln!("[test] 送信したリクエスト数: {}", stream_ids.len());
            let received = request_count.load(Ordering::SeqCst);
            eprintln!("[test] サーバーが受信したリクエスト数: {}", received);
            assert!(received >= 1, "少なくとも 1 リクエストが受信されるべき");
        }
        Err(_) => {
            panic!("テストタイムアウト");
        }
    }

    eprintln!("[test] HTTP/3 並行リクエストテスト完了");
}

/// HTTP/3 POST リクエスト + JSON ボディ送信テスト
#[serial]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_http3_request_with_body() {
    let (cert_path, key_path) = generate_test_certs();

    let body_received = Arc::new(AtomicBool::new(false));
    let body_received_clone = body_received.clone();
    let received_body = Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let received_body_clone = received_body.clone();

    // サーバーを起動
    let mut server = Server::bind(
        "127.0.0.1:0".parse().expect("test must succeed"),
        &cert_path,
        &key_path,
        None,
        None,
    )
    .await
    .expect("サーバー作成失敗");

    let server_addr = server.local_addr();
    eprintln!("[test] サーバー起動: {}", server_addr);

    // サーバータスク
    let server_task = tokio::spawn(async move {
        let _ = timeout(
            Duration::from_secs(10),
            server.run(move |_addr, event| {
                match event {
                    Http3Event::Data { stream_id, data } => {
                        eprintln!(
                            "[server] Data 受信: stream_id = {}, data = {:?}",
                            stream_id,
                            String::from_utf8_lossy(&data)
                        );
                        body_received_clone.store(true, Ordering::SeqCst);
                        received_body_clone
                            .lock()
                            .expect("test must succeed")
                            .extend_from_slice(&data);
                        None
                    }
                    Http3Event::HeadersEnd { stream_id, .. } => {
                        eprintln!("[server] HeadersEnd: stream_id = {}", stream_id);
                        // Data を受信するまでレスポンスを遅延
                        None
                    }
                    Http3Event::StreamEnd { stream_id } => {
                        eprintln!("[server] StreamEnd: stream_id = {}", stream_id);
                        // ストリーム終了時にレスポンスを返す
                        Some((vec![Header::status(200)], Vec::new()))
                    }
                    _ => None,
                }
            }),
        )
        .await;
    });

    // クライアントを作成
    let client_result = timeout(Duration::from_secs(10), async {
        let mut client = Client::connect_insecure_default(server_addr, "localhost")
            .await
            .expect("クライアント作成失敗");

        // ハンドシェイク
        client.handshake().await.expect("ハンドシェイク失敗");
        eprintln!("[client] ハンドシェイク完了");

        // POST リクエストをボディ付きで送信
        let body = br#"{"name": "test", "value": 123}"#.to_vec();
        let headers = vec![
            Header::method("POST"),
            Header::scheme("https"),
            Header::authority(&format!("localhost:{}", server_addr.port())),
            Header::path("/api/data"),
            Header::new(b"content-type", b"application/json"),
            Header::new(b"content-length", body.len().to_string().as_bytes()),
        ];

        let stream_id = client
            .send_request_with_body(&headers, body)
            .expect("リクエスト送信失敗");
        eprintln!("[client] POST リクエスト送信: stream_id = {}", stream_id);

        // HTTP/3 データを送信
        client.flush().await.expect("フラッシュ失敗");

        // サーバーがデータを処理する時間を確保
        tokio::time::sleep(Duration::from_millis(500)).await;

        stream_id
    })
    .await;

    server_task.abort();

    assert!(client_result.is_ok(), "テストがタイムアウトしないこと");
    assert!(
        body_received.load(Ordering::SeqCst),
        "サーバーがリクエストボディを受信するべき"
    );

    let body_data = received_body.lock().expect("test must succeed");
    assert!(!body_data.is_empty(), "受信したボディが空でないこと");
    eprintln!(
        "[test] 受信したボディ: {:?}",
        String::from_utf8_lossy(&body_data)
    );

    eprintln!("[test] HTTP/3 POST リクエスト + ボディ送信テスト完了");
}

/// HTTP/3 レスポンスボディ受信テスト
#[serial]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_http3_response_body() {
    let (cert_path, key_path) = generate_test_certs();

    let response_body = b"Hello, HTTP/3 World!".to_vec();
    let expected_body = response_body.clone();

    // サーバーを起動
    let mut server = Server::bind(
        "127.0.0.1:0".parse().expect("test must succeed"),
        &cert_path,
        &key_path,
        None,
        None,
    )
    .await
    .expect("サーバー作成失敗");

    let server_addr = server.local_addr();
    eprintln!("[test] サーバー起動: {}", server_addr);

    // サーバータスク (ボディ付きレスポンスを返す)
    let server_task = tokio::spawn(async move {
        let _ = timeout(
            Duration::from_secs(10),
            server.run(move |_addr, event| {
                if let Http3Event::HeadersEnd { stream_id, .. } = event {
                    eprintln!("[server] リクエスト受信: stream_id = {}", stream_id);
                    // ボディ付きレスポンスを返す
                    let headers = vec![
                        Header::status(200),
                        Header::new(b"content-type", b"text/plain"),
                    ];
                    return Some((headers, response_body.clone()));
                }
                None
            }),
        )
        .await;
    });

    // クライアントを作成してレスポンスボディを受信
    let client_result = timeout(Duration::from_secs(10), async {
        let mut client = Client::connect_insecure_default(server_addr, "localhost")
            .await
            .expect("クライアント作成失敗");

        // ハンドシェイク
        client.handshake().await.expect("ハンドシェイク失敗");
        eprintln!("[client] ハンドシェイク完了");

        // GET リクエストを送信
        let headers = vec![
            Header::method("GET"),
            Header::scheme("https"),
            Header::authority(&format!("localhost:{}", server_addr.port())),
            Header::path("/"),
        ];

        let stream_id = client.send_request(&headers).expect("リクエスト送信失敗");
        eprintln!("[client] リクエスト送信: stream_id = {}", stream_id);

        // HTTP/3 データを送信
        client.flush().await.expect("フラッシュ失敗");

        // レスポンスを受信
        let mut received_body = Vec::new();
        let mut response_received = false;

        for _ in 0..20 {
            client
                .recv(Duration::from_millis(100))
                .await
                .expect("受信失敗");

            while let Some(event) = client.poll() {
                match event {
                    Http3Event::Data { data, .. } => {
                        eprintln!("[client] Data 受信: {:?}", String::from_utf8_lossy(&data));
                        received_body.extend_from_slice(&data);
                    }
                    Http3Event::HeadersEnd { .. } => {
                        eprintln!("[client] HeadersEnd 受信");
                        response_received = true;
                    }
                    Http3Event::StreamEnd { .. } => {
                        eprintln!("[client] StreamEnd 受信");
                    }
                    _ => {}
                }
            }

            if response_received && !received_body.is_empty() {
                break;
            }
        }

        received_body
    })
    .await;

    server_task.abort();

    match client_result {
        Ok(received_body) => {
            eprintln!(
                "[test] 受信したボディ: {:?}",
                String::from_utf8_lossy(&received_body)
            );
            assert_eq!(
                received_body, expected_body,
                "受信したボディが期待値と一致するべき"
            );
        }
        Err(_) => {
            panic!("テストタイムアウト");
        }
    }

    eprintln!("[test] HTTP/3 レスポンスボディ受信テスト完了");
}

/// HTTP/3 ストリーム多重化テスト
#[serial]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_http3_stream_multiplexing() {
    let (cert_path, key_path) = generate_test_certs();

    use std::sync::atomic::AtomicUsize;
    let request_count = Arc::new(AtomicUsize::new(0));
    let request_count_clone = request_count.clone();

    // サーバーを起動
    let mut server = Server::bind(
        "127.0.0.1:0".parse().expect("test must succeed"),
        &cert_path,
        &key_path,
        None,
        None,
    )
    .await
    .expect("サーバー作成失敗");

    let server_addr = server.local_addr();
    eprintln!("[test] サーバー起動: {}", server_addr);

    // サーバータスク
    let server_task = tokio::spawn(async move {
        let _ = timeout(
            Duration::from_secs(10),
            server.run(move |_addr, event| {
                if let Http3Event::HeadersEnd { stream_id, .. } = event {
                    let count = request_count_clone.fetch_add(1, Ordering::SeqCst) + 1;
                    eprintln!(
                        "[server] リクエスト {} 受信: stream_id = {}",
                        count, stream_id
                    );
                    // 各リクエストに対してユニークなレスポンスを返す
                    let body = format!("Response for stream {}", stream_id).into_bytes();
                    let headers = vec![
                        Header::status(200),
                        Header::new(b"x-stream-id", stream_id.to_string().as_bytes()),
                    ];
                    return Some((headers, body));
                }
                None
            }),
        )
        .await;
    });

    // クライアントを作成して複数のストリームを同時に開く
    let client_result = timeout(Duration::from_secs(10), async {
        let mut client = Client::connect_insecure_default(server_addr, "localhost")
            .await
            .expect("クライアント作成失敗");

        // ハンドシェイク
        client.handshake().await.expect("ハンドシェイク失敗");
        eprintln!("[client] ハンドシェイク完了");

        // 3 つのリクエストを同時に送信
        let paths = ["/stream1", "/stream2", "/stream3"];
        let mut stream_ids = Vec::new();

        for path in &paths {
            let headers = vec![
                Header::method("GET"),
                Header::scheme("https"),
                Header::authority(&format!("localhost:{}", server_addr.port())),
                Header::path(path),
            ];

            let stream_id = client.send_request(&headers).expect("リクエスト送信失敗");
            stream_ids.push(stream_id);
            eprintln!(
                "[client] リクエスト送信: path = {}, stream_id = {}",
                path, stream_id
            );
        }

        // HTTP/3 データを送信
        client.flush().await.expect("フラッシュ失敗");

        // レスポンスを受信
        let mut responses_received = 0;
        let mut bodies_received = std::collections::HashMap::new();

        for _ in 0..30 {
            client
                .recv(Duration::from_millis(100))
                .await
                .expect("受信失敗");

            while let Some(event) = client.poll() {
                match event {
                    Http3Event::Data { stream_id, data } => {
                        eprintln!(
                            "[client] Data 受信: stream_id = {}, data = {:?}",
                            stream_id,
                            String::from_utf8_lossy(&data)
                        );
                        bodies_received
                            .entry(stream_id)
                            .or_insert_with(Vec::new)
                            .extend_from_slice(&data);
                    }
                    Http3Event::HeadersEnd { stream_id, .. } => {
                        eprintln!("[client] HeadersEnd 受信: stream_id = {}", stream_id);
                        responses_received += 1;
                    }
                    _ => {}
                }
            }

            if responses_received >= 3 && bodies_received.len() >= 3 {
                break;
            }
        }

        (stream_ids, responses_received, bodies_received)
    })
    .await;

    server_task.abort();

    match client_result {
        Ok((stream_ids, responses_received, bodies_received)) => {
            eprintln!(
                "[test] 送信したストリーム数: {}, 受信したレスポンス数: {}, ボディ受信数: {}",
                stream_ids.len(),
                responses_received,
                bodies_received.len()
            );
            assert_eq!(stream_ids.len(), 3, "3 つのストリームを開くべき");
            assert!(responses_received >= 3, "3 つのレスポンスを受信するべき");
            assert!(bodies_received.len() >= 3, "3 つのボディを受信するべき");

            // 各ストリームに対して正しいレスポンスを受信したことを確認
            for stream_id in &stream_ids {
                if let Some(body) = bodies_received.get(stream_id) {
                    let body_str = String::from_utf8_lossy(body);
                    assert!(
                        body_str.contains(&stream_id.to_string()),
                        "ストリーム {} のボディにストリーム ID が含まれるべき",
                        stream_id
                    );
                }
            }
        }
        Err(_) => {
            panic!("テストタイムアウト");
        }
    }

    eprintln!("[test] HTTP/3 ストリーム多重化テスト完了");
}

/// フロー制御ウィンドウを超えるボディ送信が完走するテスト
///
/// デフォルトのストリームウィンドウ (1MB) を超える 2MB のレスポンスボディを
/// サーバーが送信し、受信側が事前に広げたウィンドウで完走することを検証する
/// (RFC 9000 Section 18.2)。ウィンドウ拡張なしの場合は MAX_STREAM_DATA による
/// 拡張が必要となり、クライアント側の受信速度に大きく依存するため、
/// ここでは初期ウィンドウを広げて送信経路そのものを検証する。
#[serial]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_http3_large_body_upload() {
    let (cert_path, key_path) = generate_test_certs();

    let expected_body = (0u8..255)
        .cycle()
        .take(2 * 1024 * 1024)
        .collect::<Vec<u8>>();

    let server_expected = expected_body.clone();

    // サーバーを起動 (2MB レスポンスを返す)
    let mut server = Server::bind(
        "127.0.0.1:0".parse().expect("test must succeed"),
        &cert_path,
        &key_path,
        None,
        None,
    )
    .await
    .expect("サーバー作成失敗");

    let server_addr = server.local_addr();
    eprintln!("[test] サーバー起動: {}", server_addr);

    let server_task = tokio::spawn(async move {
        let _ = timeout(
            Duration::from_secs(30),
            server.run(move |_addr, event| {
                if let Http3Event::HeadersEnd { .. } = event {
                    eprintln!("[server] リクエスト受信");
                    let headers = vec![
                        Header::status(200),
                        Header::new(b"content-type", b"application/octet-stream"),
                    ];
                    return Some((headers, server_expected.clone()));
                }
                None
            }),
        )
        .await;
    });

    // クライアントを作成して 2MB ボディのレスポンスを受信
    let client_result = timeout(Duration::from_secs(30), async {
        // 受信側ウィンドウ (bidi_local = クライアントがサーバーの送信を
        // 受け入れる量) を 5MB に広げて 2MB のレスポンスが
        // MAX_STREAM_DATA 拡張なしで通るようにする (RFC 9000 Section 18.2)
        let params = shiguredo_ngtcp2::TransportParams::new()
            .with_initial_max_stream_data_bidi_local(5 * 1024 * 1024)
            .with_initial_max_stream_data_bidi_remote(5 * 1024 * 1024)
            .into_raw();
        let mut client = Client::connect_insecure(server_addr, "localhost", Some(params), None)
            .await
            .expect("クライアント作成失敗");

        client.handshake().await.expect("ハンドシェイク失敗");
        eprintln!("[client] ハンドシェイク完了");

        let headers = vec![
            Header::method("GET"),
            Header::scheme("https"),
            Header::authority(&format!("localhost:{}", server_addr.port())),
            Header::path("/"),
        ];

        let stream_id = client.send_request(&headers).expect("リクエスト送信失敗");
        eprintln!("[client] リクエスト送信: stream_id = {}", stream_id);

        client.flush().await.expect("フラッシュ失敗");

        // 全ボディを受信し、FIN (StreamEnd) まで待つ
        let mut received_body = Vec::new();

        loop {
            client
                .recv(Duration::from_millis(50))
                .await
                .expect("受信失敗");

            // 受信した ACK を QUIC に反映し、フロー制御ウィンドウを広げる
            client.flush().await.expect("フラッシュ失敗");

            let mut stream_end = false;

            while let Some(event) = client.poll() {
                match event {
                    Http3Event::Data { data, .. } => {
                        received_body.extend_from_slice(&data);
                    }
                    Http3Event::StreamEnd { .. } => {
                        eprintln!(
                            "[client] StreamEnd 受信: ボディ {} バイト",
                            received_body.len()
                        );
                        stream_end = true;
                    }
                    _ => {}
                }
            }

            if stream_end {
                break;
            }
        }

        received_body
    })
    .await;

    server_task.abort();

    match client_result {
        Ok(received_body) => {
            assert_eq!(
                received_body,
                expected_body,
                "受信したボディ ({} バイト) が期待値 ({} バイト) と一致するべき",
                received_body.len(),
                expected_body.len()
            );
            eprintln!("[test] 2MB ボディ送信テスト完了");
        }
        Err(_) => {
            panic!("テストタイムアウト: 2MB ボディが完走しなかった");
        }
    }
}
