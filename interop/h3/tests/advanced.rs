//! 高度な HTTP/3 テスト
//!
//! 複数リクエスト、大きなボディのテスト

use std::time::Duration;

use tokio_s2n_quic::{H3Client, H3ClientRequest, H3Response, H3Server, ServerConfig};

use interop_h3::generate_shared_certificate;

/// 複数の連続リクエストをテスト
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_multiple_sequential_requests() {
    let (cert_pem, key_pem) = generate_shared_certificate().unwrap();

    let config = ServerConfig::new("127.0.0.1:0".parse().unwrap(), &cert_pem, &key_pem);
    let mut server = H3Server::bind(config).unwrap();
    let server_addr = server.local_addr();
    eprintln!("[server] サーバー起動: {}", server_addr);

    // サーバー: 3 リクエストを処理
    let server_handle = tokio::spawn(async move {
        let mut conn = server.accept().await.unwrap();
        for i in 0..3 {
            let request = conn.accept_request().await.unwrap();
            eprintln!(
                "[server] リクエスト {}: {}",
                i + 1,
                String::from_utf8_lossy(request.path())
            );
            request
                .send_response(
                    H3Response::new(200)
                        .header("content-type", "text/plain; charset=utf-8")
                        .body("Hello from HTTP/3 server!"),
                )
                .await
                .unwrap();
        }
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    // クライアント: 3 リクエストを順番に送信
    let client_config =
        tokio_s2n_quic::ClientConfig::new(server_addr, "localhost").ca_cert(&cert_pem);
    let mut client = H3Client::connect(client_config).await.unwrap();

    let mut responses = Vec::new();
    let paths = ["/path1", "/path2", "/path3"];
    for path in paths {
        let response = client
            .send_request(H3ClientRequest::get(path).authority("localhost"))
            .await
            .unwrap();
        eprintln!(
            "[client] レスポンス: status={}, body={}",
            response.status(),
            String::from_utf8_lossy(response.body())
        );
        responses.push(response);
    }

    server_handle.abort();

    assert_eq!(responses.len(), 3, "3 つのレスポンスを受信すべき");
    for (i, response) in responses.iter().enumerate() {
        assert_eq!(response.status(), 200);
        let body_str = String::from_utf8_lossy(response.body());
        assert!(
            body_str.contains("Hello"),
            "レスポンス {} が期待通りでない: {:?}",
            i + 1,
            body_str
        );
    }
}

/// 大きなボディのレスポンステスト (64KB)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_large_body_response() {
    let (cert_pem, key_pem) = generate_shared_certificate().unwrap();
    let body_size = 64 * 1024; // 64KB

    let config = ServerConfig::new("127.0.0.1:0".parse().unwrap(), &cert_pem, &key_pem);
    let mut server = H3Server::bind(config).unwrap();
    let server_addr = server.local_addr();
    eprintln!("[server] サーバー起動: {}", server_addr);

    // サーバー: 大きなボディでレスポンス
    let server_handle = tokio::spawn(async move {
        let mut conn = server.accept().await.unwrap();
        let request = conn.accept_request().await.unwrap();

        let body: Vec<u8> = (0..body_size).map(|i| (i % 256) as u8).collect();
        request
            .send_response(
                H3Response::new(200)
                    .header("content-type", "application/octet-stream")
                    .body(body),
            )
            .await
            .unwrap();
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    // クライアント
    let client_config =
        tokio_s2n_quic::ClientConfig::new(server_addr, "localhost").ca_cert(&cert_pem);
    let mut client = H3Client::connect(client_config).await.unwrap();

    let response = client
        .send_request(H3ClientRequest::get("/large").authority("localhost"))
        .await
        .unwrap();

    server_handle.abort();

    let body = response.body();
    eprintln!("[test] レスポンスサイズ: {} bytes", body.len());
    assert_eq!(body.len(), body_size, "レスポンスサイズが期待通りでない");
    for (i, &byte) in body.iter().enumerate() {
        assert_eq!(byte, (i % 256) as u8, "バイト {} が不一致", i);
    }
}
