#![cfg(feature = "tquic-impl")]
//! ngtcp2 クライアント ↔ tquic サーバー 相互運用性テスト

use std::time::Duration;

use shiguredo_ngtcp2::{Header as Ngtcp2Header, Http3Event};
use tokio_ngtcp2::Client;

use interop_h3::{generate_shared_certificate, save_certificate_files, start_tquic_server};

/// ngtcp2 クライアントでリクエストを送信
async fn run_ngtcp2_client(
    server_addr: std::net::SocketAddr,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    eprintln!("[ngtcp2 client] connecting: {}", server_addr);

    let mut client = Client::connect_insecure(server_addr, "localhost", None, None).await?;

    eprintln!("[ngtcp2 client] handshake started");
    client.handshake().await?;
    eprintln!("[ngtcp2 client] handshake completed");

    let headers = vec![
        Ngtcp2Header::method("GET"),
        Ngtcp2Header::path("/"),
        Ngtcp2Header::scheme("https"),
        Ngtcp2Header::authority("localhost"),
    ];

    let stream_id = client.send_request(&headers)?;
    eprintln!("[ngtcp2 client] request sent: stream_id = {}", stream_id);

    let mut response_body = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);

    loop {
        client.flush().await?;
        client.recv(Duration::from_millis(100)).await?;

        while let Some(event) = client.poll() {
            match event {
                Http3Event::HeadersBegin { stream_id } => {
                    eprintln!("[ngtcp2 client] headers begin: stream_id = {}", stream_id);
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
                    eprintln!("[ngtcp2 client] headers end: stream_id = {}", stream_id);
                }
                Http3Event::Data { stream_id, data } => {
                    eprintln!(
                        "[ngtcp2 client] data received: stream_id = {}, len = {}",
                        stream_id,
                        data.len()
                    );
                    response_body.extend_from_slice(&data);
                }
                Http3Event::StreamEnd { stream_id } => {
                    eprintln!("[ngtcp2 client] stream end: stream_id = {}", stream_id);
                    return Ok(response_body);
                }
                _ => {}
            }
        }

        if tokio::time::Instant::now() >= deadline {
            eprintln!("[ngtcp2 client] timeout");
            break;
        }
    }

    Ok(response_body)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_http3_request_response() {
    let (cert_pem, key_pem) = generate_shared_certificate().expect("test must succeed");
    let (_cert_dir, cert_path, key_path) =
        save_certificate_files(&cert_pem, &key_pem).expect("test must succeed");

    let (port_tx, port_rx) = std::sync::mpsc::channel();
    let (shutdown_tx, shutdown_rx) = std::sync::mpsc::channel();

    // tquic サーバーを起動 (ブロッキングスレッド)
    let server_thread = std::thread::spawn(move || {
        if let Err(e) = start_tquic_server(cert_path, key_path, port_tx, shutdown_rx) {
            eprintln!("[test] server error: {:?}", e);
        }
    });

    let port = port_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("test must succeed");
    eprintln!("[test] server port: {}", port);

    tokio::time::sleep(Duration::from_millis(100)).await;

    // ngtcp2 クライアントでリクエスト
    let server_addr: std::net::SocketAddr = format!("127.0.0.1:{}", port)
        .parse()
        .expect("test must succeed");
    let result =
        tokio::time::timeout(Duration::from_secs(10), run_ngtcp2_client(server_addr)).await;

    let _ = shutdown_tx.send(());
    let _ = server_thread.join();

    match result {
        Ok(Ok(response)) => {
            let response_str = String::from_utf8_lossy(&response);
            eprintln!("[test] response: {}", response_str);
            assert!(
                response_str.trim() == "Hello from tquic HTTP/3 server!",
                "レスポンスボディが期待値と一致しない: {:?}",
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
