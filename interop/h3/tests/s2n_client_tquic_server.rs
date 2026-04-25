#![cfg(feature = "tquic-impl")]
//! s2n-quic クライアント ↔ tquic サーバー 相互運用性テスト

use std::net::SocketAddr;
use std::time::Duration;

use tokio_s2n_quic::{ClientConfig, H3Client, H3ClientRequest};

use interop_h3::{generate_shared_certificate, save_certificate_files, start_tquic_server};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_http3_request_response() {
    let (cert_pem, key_pem) = generate_shared_certificate().unwrap();
    let (_cert_dir, cert_path, key_path) = save_certificate_files(&cert_pem, &key_pem).unwrap();

    let (port_tx, port_rx) = std::sync::mpsc::channel();
    let (shutdown_tx, shutdown_rx) = std::sync::mpsc::channel();

    // tquic サーバーを起動 (ブロッキングスレッド)
    let server_cert_path = cert_path.clone();
    let server_key_path = key_path.clone();
    let server_thread = std::thread::spawn(move || {
        if let Err(e) = start_tquic_server(server_cert_path, server_key_path, port_tx, shutdown_rx)
        {
            eprintln!("[test] server error: {:?}", e);
        }
    });

    let port = port_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    eprintln!("[test] server port: {}", port);

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
            "[s2n client] response: status={}, body={}",
            response.status(),
            String::from_utf8_lossy(response.body())
        );

        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(response.body().to_vec())
    })
    .await;

    let _ = shutdown_tx.send(());
    let _ = server_thread.join();

    match client_result {
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
