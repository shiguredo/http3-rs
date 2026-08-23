//! quinn クライアント ↔ ngtcp2 サーバー 相互運用性テスト

use std::time::Duration;

use shiguredo_ngtcp2::{Header as Ngtcp2Header, Http3Event};
use tokio_ngtcp2::Server;

use interop_h3::{generate_shared_certificate, run_quinn_client, save_certificate_files};

/// ngtcp2 サーバーを起動
async fn start_ngtcp2_server(
    cert_path: &std::path::Path,
    key_path: &std::path::Path,
) -> Result<(Server, std::net::SocketAddr), Box<dyn std::error::Error + Send + Sync>> {
    let server = Server::bind(
        "127.0.0.1:0".parse().expect("test must succeed"),
        cert_path,
        key_path,
        None,
        None,
    )
    .await?;

    let local_addr = server.local_addr();
    eprintln!("[ngtcp2 server] サーバー起動: {}", local_addr);

    Ok((server, local_addr))
}

#[tokio::test]
async fn test_http3_request_response() {
    let (cert_pem, key_pem) = generate_shared_certificate().expect("test must succeed");
    let (_cert_dir, cert_path, key_path) =
        save_certificate_files(&cert_pem, &key_pem).expect("test must succeed");

    // ngtcp2 サーバーを起動
    let (server, server_addr) = start_ngtcp2_server(&cert_path, &key_path)
        .await
        .expect("test must succeed");
    let port = server_addr.port();
    eprintln!("[test] サーバーポート: {}", port);

    let server_task = tokio::spawn(async move {
        let mut server = server;
        let _ = tokio::time::timeout(
            Duration::from_secs(10),
            server.run(|addr, event| {
                eprintln!("[ngtcp2 server] イベント: {:?} from {:?}", event, addr);

                match event {
                    Http3Event::HeadersEnd { stream_id, .. } => {
                        eprintln!("[ngtcp2 server] ヘッダー終了: stream_id = {}", stream_id);

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

    // quinn クライアントでリクエスト
    let result = tokio::time::timeout(Duration::from_secs(10), run_quinn_client(port)).await;

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
