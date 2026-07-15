//! ngtcp2 クライアント ↔ s2n-quic サーバー 相互運用性テスト
//!
//! ngtcp2 クライアント (nghttp3) から s2n-quic サーバー (shiguredo_http3) への
//! HTTP/3 通信を RFC 9114 に基づいてテストする。

use std::net::SocketAddr;
use std::time::Duration;

use shiguredo_ngtcp2::{Header as Ngtcp2Header, Http3Event};
use tokio_ngtcp2::Client;
use tokio_s2n_quic::{H3Response, H3Server, ServerConfig};

use interop_h3::generate_shared_certificate;

/// ngtcp2 クライアントのレスポンス
struct Ngtcp2Response {
    /// ステータスコード
    status: u16,
    /// レスポンスヘッダー
    headers: Vec<(Vec<u8>, Vec<u8>)>,
    /// レスポンスボディ
    body: Vec<u8>,
}

/// ngtcp2 クライアントでリクエストを送信してレスポンス全体を返す
async fn send_ngtcp2_request(
    server_addr: SocketAddr,
    method: &str,
    path: &str,
    body: Vec<u8>,
) -> Result<Ngtcp2Response, Box<dyn std::error::Error + Send + Sync>> {
    eprintln!("[ngtcp2 client] 接続開始: {}", server_addr);

    let mut client = Client::connect_insecure(server_addr, "localhost", None, None).await?;
    client.handshake().await?;

    let headers = vec![
        Ngtcp2Header::method(method),
        Ngtcp2Header::path(path),
        Ngtcp2Header::scheme("https"),
        Ngtcp2Header::authority("localhost"),
    ];

    let _stream_id = if body.is_empty() {
        client.send_request(&headers)?
    } else {
        client.send_request_with_body(&headers, body)?
    };

    let mut response_headers: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    let mut response_body = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);

    loop {
        client.flush().await?;
        client.recv(Duration::from_millis(100)).await?;

        while let Some(event) = client.poll() {
            match event {
                Http3Event::Header { header, .. } => {
                    response_headers.push((header.name.clone(), header.value.clone()));
                }
                Http3Event::Data { data, .. } => {
                    response_body.extend_from_slice(&data);
                }
                Http3Event::StreamEnd { .. } => {
                    let status = response_headers
                        .iter()
                        .find(|(n, _)| n == b":status")
                        .and_then(|(_, v)| std::str::from_utf8(v).ok())
                        .and_then(|s| s.parse::<u16>().ok())
                        .unwrap_or(0);
                    return Ok(Ngtcp2Response {
                        status,
                        headers: response_headers,
                        body: response_body,
                    });
                }
                _ => {}
            }
        }

        if tokio::time::Instant::now() >= deadline {
            eprintln!("[ngtcp2 client] タイムアウト");
            break;
        }
    }

    let status = response_headers
        .iter()
        .find(|(n, _)| n == b":status")
        .and_then(|(_, v)| std::str::from_utf8(v).ok())
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(0);

    Ok(Ngtcp2Response {
        status,
        headers: response_headers,
        body: response_body,
    })
}

/// ngtcp2 クライアントでリクエストを送信
async fn run_ngtcp2_client(
    server_addr: std::net::SocketAddr,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    eprintln!("[ngtcp2 client] 接続開始: {}", server_addr);

    let mut client = Client::connect_insecure(server_addr, "localhost", None, None).await?;

    eprintln!("[ngtcp2 client] ハンドシェイク開始");
    client.handshake().await?;
    eprintln!("[ngtcp2 client] ハンドシェイク完了");

    let headers = vec![
        Ngtcp2Header::method("GET"),
        Ngtcp2Header::path("/"),
        Ngtcp2Header::scheme("https"),
        Ngtcp2Header::authority("localhost"),
    ];

    let stream_id = client.send_request(&headers)?;
    eprintln!("[ngtcp2 client] リクエスト送信: stream_id = {}", stream_id);

    let mut response_body = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);

    loop {
        client.flush().await?;
        client.recv(Duration::from_millis(100)).await?;

        while let Some(event) = client.poll() {
            match event {
                Http3Event::HeadersBegin { stream_id } => {
                    eprintln!("[ngtcp2 client] ヘッダー開始: stream_id = {}", stream_id);
                }
                Http3Event::Header {
                    stream_id: _,
                    header,
                } => {
                    eprintln!(
                        "[ngtcp2 client]   {}: {}",
                        header.name_str().unwrap_or("?"),
                        header.value_str().unwrap_or("?")
                    );
                }
                Http3Event::HeadersEnd { stream_id, .. } => {
                    eprintln!("[ngtcp2 client] ヘッダー終了: stream_id = {}", stream_id);
                }
                Http3Event::Data { stream_id, data } => {
                    eprintln!(
                        "[ngtcp2 client] データ受信: stream_id = {}, len = {}",
                        stream_id,
                        data.len()
                    );
                    response_body.extend_from_slice(&data);
                }
                Http3Event::StreamEnd { stream_id } => {
                    eprintln!("[ngtcp2 client] ストリーム終了: stream_id = {}", stream_id);
                    return Ok(response_body);
                }
                _ => {}
            }
        }

        if tokio::time::Instant::now() >= deadline {
            eprintln!("[ngtcp2 client] タイムアウト");
            break;
        }
    }

    Ok(response_body)
}

#[tokio::test]
async fn test_http3_request_response() {
    let (cert_pem, key_pem) = generate_shared_certificate().expect("test must succeed");

    // s2n-quic サーバーを起動
    let config = ServerConfig::new(
        "127.0.0.1:0".parse().expect("test must succeed"),
        &cert_pem,
        &key_pem,
    );
    let mut server = H3Server::bind(config).expect("test must succeed");
    let server_addr = server.local_addr();
    eprintln!("[test] サーバーアドレス: {}", server_addr);

    let server_task = tokio::spawn(async move {
        let mut conn = server.accept().await.expect("test must succeed");
        let request = conn.accept_request().await.expect("test must succeed");
        eprintln!(
            "[server] リクエスト受信: {}",
            String::from_utf8_lossy(request.path())
        );
        request
            .send_response(
                H3Response::new(200)
                    .header("content-type", "text/plain; charset=utf-8")
                    .body("Hello from HTTP/3 server!"),
            )
            .await
            .expect("test must succeed");
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    // ngtcp2 クライアントでリクエスト
    let result =
        tokio::time::timeout(Duration::from_secs(10), run_ngtcp2_client(server_addr)).await;

    server_task.abort();

    match result {
        Ok(Ok(response)) => {
            let response_str = String::from_utf8_lossy(&response);
            eprintln!("[test] レスポンス: {}", response_str);
            eprintln!("[test] 通信成功 (レスポンス長: {} bytes)", response.len());
        }
        Ok(Err(e)) => {
            panic!("クライアントエラー: {:?}", e);
        }
        Err(_) => {
            panic!("タイムアウト");
        }
    }
}

/// RFC 9114 Section 4.1 - POST リクエスト with body
///
/// ngtcp2 クライアントがボディ付き POST リクエストを送信できること、
/// s2n サーバー (shiguredo_http3) が適切に受信して 200 を返せることを確認する。
#[tokio::test]
async fn test_post_request_with_body() {
    let (cert_pem, key_pem) = generate_shared_certificate().expect("test must succeed");

    let config = ServerConfig::new(
        "127.0.0.1:0".parse().expect("test must succeed"),
        &cert_pem,
        &key_pem,
    );
    let mut server = H3Server::bind(config).expect("test must succeed");
    let server_addr = server.local_addr();

    let server_task = tokio::spawn(async move {
        let mut conn = server.accept().await.expect("test must succeed");
        let request = conn.accept_request().await.expect("test must succeed");
        eprintln!(
            "[server] POST 受信: path={}, body_len={}",
            String::from_utf8_lossy(request.path()),
            request.body().len()
        );
        request
            .send_response(
                H3Response::new(200)
                    .header("content-type", "text/plain; charset=utf-8")
                    .body("OK"),
            )
            .await
            .expect("test must succeed");
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let result = tokio::time::timeout(
        Duration::from_secs(10),
        send_ngtcp2_request(server_addr, "POST", "/", b"Hello, server!".to_vec()),
    )
    .await;

    server_task.abort();

    match result {
        Ok(Ok(response)) => {
            assert_eq!(response.status, 200, "POST リクエストに 200 が返るべき");
            assert!(!response.body.is_empty(), "レスポンスボディが空でない");
        }
        Ok(Err(e)) => panic!("クライアントエラー: {:?}", e),
        Err(_) => panic!("タイムアウト"),
    }
}

/// RFC 9114 Section 4.1 - カスタムレスポンスヘッダー
///
/// s2n サーバー (shiguredo_http3) がカスタムヘッダーを含むレスポンスを返し、
/// ngtcp2 クライアントが正しく受信できることを確認する。
#[tokio::test]
async fn test_response_custom_headers() {
    let (cert_pem, key_pem) = generate_shared_certificate().expect("test must succeed");

    let config = ServerConfig::new(
        "127.0.0.1:0".parse().expect("test must succeed"),
        &cert_pem,
        &key_pem,
    );
    let mut server = H3Server::bind(config).expect("test must succeed");
    let server_addr = server.local_addr();

    let server_task = tokio::spawn(async move {
        let mut conn = server.accept().await.expect("test must succeed");
        let request = conn.accept_request().await.expect("test must succeed");
        eprintln!(
            "[server] リクエスト受信: {}",
            String::from_utf8_lossy(request.path())
        );
        request
            .send_response(
                H3Response::new(200)
                    .header("content-type", "text/plain; charset=utf-8")
                    .header("x-server", "shiguredo_http3")
                    .header("x-custom-header", "custom-value")
                    .body("Hello"),
            )
            .await
            .expect("test must succeed");
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let result = tokio::time::timeout(
        Duration::from_secs(10),
        send_ngtcp2_request(server_addr, "GET", "/", vec![]),
    )
    .await;

    server_task.abort();

    match result {
        Ok(Ok(response)) => {
            assert_eq!(response.status, 200, ":status が 200 でない");
            assert!(
                response
                    .headers
                    .iter()
                    .any(|(n, v)| n == b"x-server" && v == b"shiguredo_http3"),
                "x-server ヘッダーがない"
            );
            assert!(
                response
                    .headers
                    .iter()
                    .any(|(n, v)| n == b"x-custom-header" && v == b"custom-value"),
                "x-custom-header ヘッダーがない"
            );
            assert_eq!(response.body, b"Hello", "レスポンスボディが不一致");
        }
        Ok(Err(e)) => panic!("クライアントエラー: {:?}", e),
        Err(_) => panic!("タイムアウト"),
    }
}

/// RFC 9114 Section 4.1 - 4xx ステータスコード
///
/// s2n サーバー (shiguredo_http3) がパスに応じて 404 を返し、
/// ngtcp2 クライアントが正しくステータスコードを受信できることを確認する。
#[tokio::test]
async fn test_status_404() {
    let (cert_pem, key_pem) = generate_shared_certificate().expect("test must succeed");

    let config = ServerConfig::new(
        "127.0.0.1:0".parse().expect("test must succeed"),
        &cert_pem,
        &key_pem,
    );
    let mut server = H3Server::bind(config).expect("test must succeed");
    let server_addr = server.local_addr();

    let server_task = tokio::spawn(async move {
        let mut conn = server.accept().await.expect("test must succeed");
        let request = conn.accept_request().await.expect("test must succeed");
        let path = request.path().to_vec();
        eprintln!(
            "[server] リクエスト受信: {}",
            String::from_utf8_lossy(&path)
        );
        let status = if path == b"/not-found" { 404 } else { 200 };
        request
            .send_response(H3Response::new(status).body(if status == 404 {
                "Not Found"
            } else {
                "OK"
            }))
            .await
            .expect("test must succeed");
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let result = tokio::time::timeout(
        Duration::from_secs(10),
        send_ngtcp2_request(server_addr, "GET", "/not-found", vec![]),
    )
    .await;

    server_task.abort();

    match result {
        Ok(Ok(response)) => {
            assert_eq!(response.status, 404, "/not-found に 404 が返るべき");
        }
        Ok(Err(e)) => panic!("クライアントエラー: {:?}", e),
        Err(_) => panic!("タイムアウト"),
    }
}
