//! ngtcp2 クライアント ↔ quinn サーバー 相互運用性テスト

use std::time::Duration;

use shiguredo_ngtcp2::{Header as Ngtcp2Header, Http3Event};
use tokio::sync::mpsc;
use tokio_ngtcp2::Client;

use interop_h3::{generate_shared_certificate, start_quinn_server};

/// ngtcp2 クライアントでリクエストを送信
async fn run_ngtcp2_client(
    server_addr: std::net::SocketAddr,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    eprintln!("[ngtcp2 client] 接続開始: {}", server_addr);

    let mut client = Client::connect_insecure(server_addr, "localhost", None, None).await?;

    eprintln!("[ngtcp2 client] ハンドシェイク開始");
    client.handshake().await?;
    eprintln!("[ngtcp2 client] ハンドシェイク完了");

    let headers = vec![
        Ngtcp2Header::method("GET"),
        Ngtcp2Header::path("/"),
        Ngtcp2Header::scheme("https"),
        Ngtcp2Header::authority("localhost"),
    ];

    let stream_id = client.send_request(&headers)?;
    eprintln!("[ngtcp2 client] リクエスト送信: stream_id = {}", stream_id);

    let mut response_body = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);

    loop {
        client.flush().await?;
        client.recv(Duration::from_millis(100)).await?;

        while let Some(event) = client.poll() {
            match event {
                Http3Event::HeadersBegin { stream_id } => {
                    eprintln!("[ngtcp2 client] ヘッダー開始: stream_id = {}", stream_id);
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
                    eprintln!("[ngtcp2 client] ヘッダー終了: stream_id = {}", stream_id);
                }
                Http3Event::Data { stream_id, data } => {
                    eprintln!(
                        "[ngtcp2 client] データ受信: stream_id = {}, len = {}",
                        stream_id,
                        data.len()
                    );
                    response_body.extend_from_slice(&data);
                }
                Http3Event::StreamEnd { stream_id } => {
                    eprintln!("[ngtcp2 client] ストリーム終了: stream_id = {}", stream_id);
                    return Ok(response_body);
                }
                _ => {}
            }
        }

        if tokio::time::Instant::now() >= deadline {
            eprintln!("[ngtcp2 client] タイムアウト");
            break;
        }
    }

    Ok(response_body)
}

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

    // ngtcp2 クライアントでリクエスト
    let server_addr: std::net::SocketAddr = format!("127.0.0.1:{}", port)
        .parse()
        .expect("test must succeed");
    let result =
        tokio::time::timeout(Duration::from_secs(10), run_ngtcp2_client(server_addr)).await;

    let _ = shutdown_tx.send(()).await;
    server_task.abort();

    match result {
        Ok(Ok(response)) => {
            let response_str = String::from_utf8_lossy(&response);
            eprintln!("[test] レスポンス: {}", response_str);
            eprintln!("[test] 通信成功 (レスポンス長: {} bytes)", response.len());
        }
        Ok(Err(e)) => {
            panic!("クライアントエラー: {:?}", e);
        }
        Err(_) => {
            panic!("タイムアウト");
        }
    }
}
