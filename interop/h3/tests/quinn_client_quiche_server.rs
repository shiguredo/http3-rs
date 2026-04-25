//! quinn クライアント ↔ quiche サーバー 相互運用性テスト

use std::time::Duration;

use tokio::sync::mpsc;

use interop_h3::{
    generate_shared_certificate, run_quinn_client, save_certificate_files, start_quiche_server,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_http3_request_response() {
    let (cert_pem, key_pem) = generate_shared_certificate().unwrap();
    let (_cert_dir, cert_path, key_path) = save_certificate_files(&cert_pem, &key_pem).unwrap();

    let (port_tx, port_rx) = std::sync::mpsc::channel();
    let (shutdown_tx, shutdown_rx) = mpsc::channel(1);

    // quiche サーバーを起動
    let server_task = tokio::spawn(async move {
        if let Err(e) = start_quiche_server(cert_path, key_path, port_tx, shutdown_rx).await {
            eprintln!("[test] サーバーエラー: {:?}", e);
        }
    });

    let port = port_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    eprintln!("[test] サーバーポート: {}", port);

    tokio::time::sleep(Duration::from_millis(100)).await;

    // quinn クライアントでリクエスト
    let result = tokio::time::timeout(Duration::from_secs(10), run_quinn_client(port)).await;

    let _ = shutdown_tx.send(()).await;
    server_task.abort();

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
