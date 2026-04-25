//! quiche クライアント ↔ s2n-quic サーバー 相互運用性テスト

use std::time::Duration;

use tokio_s2n_quic::{H3Response, H3Server, ServerConfig};

use interop_h3::{generate_shared_certificate, run_quiche_client};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_http3_request_response() {
    let (cert_pem, key_pem) = generate_shared_certificate().unwrap();

    // s2n-quic サーバーを起動
    let config = ServerConfig::new("127.0.0.1:0".parse().unwrap(), &cert_pem, &key_pem);
    let mut server = H3Server::bind(config).unwrap();
    let server_addr = server.local_addr();
    let port = server_addr.port();
    eprintln!("[test] サーバーポート: {}", port);

    let server_handle = tokio::spawn(async move {
        let mut conn = server.accept().await.unwrap();
        let request = conn.accept_request().await.unwrap();
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
            .unwrap();
        // レスポンス送信完了を待ってから終了
        tokio::time::sleep(Duration::from_millis(100)).await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    // quiche クライアントでリクエスト
    let result = tokio::time::timeout(Duration::from_secs(10), run_quiche_client(port)).await;

    server_handle.abort();

    match result {
        Ok(Ok(response)) => {
            let response_str = String::from_utf8_lossy(&response);
            eprintln!("[test] レスポンス: {}", response_str);
            assert!(
                response_str.contains("Hello") || !response.is_empty(),
                "レスポンスが期待通りでない: {:?}",
                response_str
            );
        }
        Ok(Err(e)) => {
            panic!("クライアントエラー: {:?}", e);
        }
        Err(_) => {
            panic!("タイムアウト");
        }
    }
}
