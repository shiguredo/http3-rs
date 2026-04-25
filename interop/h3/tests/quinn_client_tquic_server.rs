#![cfg(feature = "tquic-impl")]
//! quinn クライアント ↔ tquic サーバー 相互運用性テスト

use std::time::Duration;

use interop_h3::{
    generate_shared_certificate, run_quinn_client, save_certificate_files, start_tquic_server,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_http3_request_response() {
    let (cert_pem, key_pem) = generate_shared_certificate().unwrap();
    let (_cert_dir, cert_path, key_path) = save_certificate_files(&cert_pem, &key_pem).unwrap();

    let (port_tx, port_rx) = std::sync::mpsc::channel();
    let (shutdown_tx, shutdown_rx) = std::sync::mpsc::channel();

    // tquic サーバーを起動 (ブロッキングスレッド)
    let server_thread = std::thread::spawn(move || {
        if let Err(e) = start_tquic_server(cert_path, key_path, port_tx, shutdown_rx) {
            eprintln!("[test] server error: {:?}", e);
        }
    });

    let port = port_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    eprintln!("[test] server port: {}", port);

    tokio::time::sleep(Duration::from_millis(100)).await;

    // quinn クライアントでリクエスト
    let result = tokio::time::timeout(Duration::from_secs(10), run_quinn_client(port)).await;

    let _ = shutdown_tx.send(());
    let _ = server_thread.join();

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
