//! quinn クライアント ↔ s2n-quic サーバー 相互運用性テスト

use std::time::Duration;

use tokio_s2n_quic::{H3Response, H3Server, ServerConfig};

use interop_h3::{generate_shared_certificate, run_quinn_client};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
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

    // quinn クライアントでリクエスト
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        run_quinn_client(server_addr.port()),
    )
    .await;

    server_task.abort();

    match result {
        Ok(Ok(response)) => {
            let response_str = String::from_utf8_lossy(&response);
            eprintln!("[test] レスポンス: {}", response_str);
            assert_eq!(
                response_str.trim(),
                "Hello from HTTP/3 server!",
                "レスポンスボディが期待値と一致しない: {:?}",
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
