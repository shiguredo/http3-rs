//! quiche クライアント ↔ quinn サーバー 相互運用性テスト

use std::time::Duration;

use tokio::sync::mpsc;

use interop_h3::{generate_shared_certificate, run_quiche_client, start_quinn_server};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_http3_request_response() {
    let (cert_pem, key_pem) = generate_shared_certificate().expect("test must succeed");

    let (port_tx, port_rx) = std::sync::mpsc::channel();
    let (shutdown_tx, shutdown_rx) = mpsc::channel(1);

    let server_task = tokio::spawn(async move {
        if let Err(e) = start_quinn_server(cert_pem, key_pem, port_tx, shutdown_rx).await {
            eprintln!("[test] サーバーエラー: {:?}", e);
        }
    });

    let port = port_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("test must succeed");
    eprintln!("[test] サーバーポート: {}", port);

    tokio::time::sleep(Duration::from_millis(100)).await;

    // quiche クライアントでリクエスト
    let result = tokio::time::timeout(Duration::from_secs(10), run_quiche_client(port)).await;

    let _ = shutdown_tx.send(()).await;
    server_task.abort();

    match result {
        Ok(Ok(response)) => {
            let response_str = String::from_utf8_lossy(&response);
            eprintln!("[test] レスポンス: {}", response_str);
            assert_eq!(
                response_str.trim(),
                "Hello from quinn HTTP/3 server!",
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
