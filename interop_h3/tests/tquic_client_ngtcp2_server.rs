#![cfg(feature = "tquic-impl")]
//! tquic クライアント ↔ ngtcp2 サーバー 相互運用性テスト

use std::time::Duration;

use shiguredo_ngtcp2::{Header as Ngtcp2Header, Http3Event};
use tokio_ngtcp2::Server;

use interop_h3::{generate_shared_certificate, run_tquic_client, save_certificate_files};

/// ngtcp2 サーバーを起動
async fn start_ngtcp2_server(
    cert_path: &std::path::Path,
    key_path: &std::path::Path,
) -> Result<(Server, std::net::SocketAddr), Box<dyn std::error::Error + Send + Sync>> {
    let server = Server::bind(
        "127.0.0.1:0".parse().unwrap(),
        cert_path,
        key_path,
        None,
        None,
    )
    .await?;

    let local_addr = server.local_addr();
    eprintln!("[ngtcp2 server] server started: {}", local_addr);

    Ok((server, local_addr))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_http3_request_response() {
    let (cert_pem, key_pem) = generate_shared_certificate().unwrap();
    let (_cert_dir, cert_path, key_path) = save_certificate_files(&cert_pem, &key_pem).unwrap();

    // ngtcp2 サーバーを起動
    let (server, server_addr) = start_ngtcp2_server(&cert_path, &key_path).await.unwrap();
    let port = server_addr.port();
    eprintln!("[test] server port: {}", port);

    let server_task = tokio::spawn(async move {
        let mut server = server;
        let _ = tokio::time::timeout(
            Duration::from_secs(10),
            server.run(|addr, event| {
                eprintln!("[ngtcp2 server] event: {:?} from {:?}", event, addr);

                match event {
                    Http3Event::HeadersEnd { stream_id, .. } => {
                        eprintln!("[ngtcp2 server] headers end: stream_id = {}", stream_id);

                        let response_headers = vec![
                            Ngtcp2Header::status(200),
                            Ngtcp2Header::new(b"content-type", b"text/plain; charset=utf-8"),
                        ];
                        let body = b"Hello from HTTP/3 server!".to_vec();

                        Some((response_headers, body))
                    }
                    _ => None,
                }
            }),
        )
        .await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    // tquic クライアントでリクエスト
    let result =
        tokio::time::timeout(Duration::from_secs(10), async { run_tquic_client(port) }).await;

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
