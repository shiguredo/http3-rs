//! s2n-quic クライアント ↔ quinn サーバー 相互運用性テスト

use std::net::SocketAddr;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio_s2n_quic::{ClientConfig, H3Client, H3ClientRequest};

use interop_h3::{generate_shared_certificate, start_quinn_server};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_http3_request_response() {
    let (cert_pem, key_pem) = generate_shared_certificate().unwrap();

    let (port_tx, port_rx) = std::sync::mpsc::channel();
    let (shutdown_tx, shutdown_rx) = mpsc::channel(1);

    let cert_pem_for_server = cert_pem.clone();
    let server_task = tokio::spawn(async move {
        if let Err(e) = start_quinn_server(cert_pem_for_server, key_pem, port_tx, shutdown_rx).await
        {
            eprintln!("[test] サーバーエラー: {:?}", e);
        }
    });

    let port = port_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    eprintln!("[test] サーバーポート: {}", port);

    tokio::time::sleep(Duration::from_millis(100)).await;

    // s2n-quic クライアントでリクエスト
    let client_result = tokio::time::timeout(Duration::from_secs(10), async {
        let addr: SocketAddr = format!("127.0.0.1:{}", port).parse().unwrap();
        let config = ClientConfig::new(addr, "localhost").ca_cert(&cert_pem);
        let mut client = H3Client::connect(config).await?;

        let response = client
            .send_request(H3ClientRequest::get("/").authority("localhost"))
            .await?;

        eprintln!(
            "[s2n client] レスポンス: status={}, body={}",
            response.status(),
            String::from_utf8_lossy(response.body())
        );

        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(response.body().to_vec())
    })
    .await;

    let _ = shutdown_tx.send(()).await;
    server_task.abort();

    match client_result {
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
