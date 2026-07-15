#![cfg(feature = "tquic-impl")]
//! tquic クライアント ↔ quinn サーバー 相互運用性テスト

use std::time::Duration;

use tokio::sync::mpsc;

use interop_h3::{generate_shared_certificate, run_tquic_client, start_quinn_server};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_http3_request_response() {
    let (cert_pem, key_pem) = generate_shared_certificate().expect("test must succeed");

    let (port_tx, port_rx) = std::sync::mpsc::channel();
    let (shutdown_tx, shutdown_rx) = mpsc::channel(1);

    // quinn サーバーを起動
    let server_task = tokio::spawn(async move {
        if let Err(e) = start_quinn_server(cert_pem, key_pem, port_tx, shutdown_rx).await {
            eprintln!("[test] server error: {:?}", e);
        }
    });

    let port = port_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("test must succeed");
    eprintln!("[test] server port: {}", port);

    tokio::time::sleep(Duration::from_millis(100)).await;

    // tquic クライアントでリクエスト
    let result =
        tokio::time::timeout(Duration::from_secs(10), async { run_tquic_client(port) }).await;

    let _ = shutdown_tx.send(()).await;
    server_task.abort();

    match result {
        Ok(Ok(response)) => {
            let response_str = String::from_utf8_lossy(&response);
            eprintln!("[test] response: {}", response_str);
            assert!(
                response_str.contains("Hello") || !response.is_empty(),
                "unexpected response: {:?}",
                response_str
            );
        }
        Ok(Err(e)) => {
            panic!("client error: {:?}", e);
        }
        Err(_) => {
            panic!("timeout");
        }
    }
}
