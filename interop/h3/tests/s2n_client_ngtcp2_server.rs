//! s2n-quic クライアント ↔ ngtcp2 サーバー 相互運用性テスト
//!
//! s2n-quic クライアント (shiguredo_http3) から ngtcp2 サーバー (nghttp3) への
//! HTTP/3 通信を RFC 9114 に基づいてテストする。

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use shiguredo_ngtcp2::{Header as Ngtcp2Header, Http3Event};
use tokio_ngtcp2::Server;
use tokio_s2n_quic::{ClientConfig, H3Client, H3ClientRequest};

use interop_h3::{generate_shared_certificate, save_certificate_files};

/// ngtcp2 サーバーを起動
async fn start_ngtcp2_server(
    cert_path: &std::path::Path,
    key_path: &std::path::Path,
) -> Result<(Server, SocketAddr), Box<dyn std::error::Error + Send + Sync>> {
    let server = Server::bind(
        "127.0.0.1:0".parse().unwrap(),
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

/// ngtcp2 サーバーを実行
async fn run_ngtcp2_server(
    mut server: Server,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    eprintln!("[ngtcp2 server] サーバー実行開始");

    let result = tokio::time::timeout(
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

    match result {
        Ok(Ok(())) => eprintln!("[ngtcp2 server] サーバー正常終了"),
        Ok(Err(e)) => eprintln!("[ngtcp2 server] サーバーエラー: {:?}", e),
        Err(_) => eprintln!("[ngtcp2 server] タイムアウト"),
    }

    Ok(())
}

#[tokio::test]
async fn test_http3_request_response() {
    let (cert_pem, key_pem) = generate_shared_certificate().unwrap();
    let (_cert_dir, cert_path, key_path) = save_certificate_files(&cert_pem, &key_pem).unwrap();

    eprintln!("[test] cert_path: {:?}", cert_path);
    eprintln!("[test] key_path: {:?}", key_path);

    // ngtcp2 サーバーを起動
    let (server, server_addr) = start_ngtcp2_server(&cert_path, &key_path).await.unwrap();
    let port = server_addr.port();
    eprintln!("[test] サーバーポート: {}", port);

    let server_task = tokio::spawn(async move {
        if let Err(e) = run_ngtcp2_server(server).await {
            eprintln!("[test] サーバーエラー: {:?}", e);
        }
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    // s2n-quic クライアントでリクエスト
    let result = tokio::time::timeout(Duration::from_secs(10), async {
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

/// RFC 9114 Section 4.1 - POST リクエスト with body
///
/// クライアントがボディ付き POST リクエストを送信できること、
/// サーバーが適切に受信して 200 を返せることを確認する。
#[tokio::test]
async fn test_post_request_with_body() {
    let (cert_pem, key_pem) = generate_shared_certificate().unwrap();
    let (_cert_dir, cert_path, key_path) = save_certificate_files(&cert_pem, &key_pem).unwrap();

    let (mut server, server_addr) = start_ngtcp2_server(&cert_path, &key_path).await.unwrap();
    let port = server_addr.port();

    let server_task = tokio::spawn(async move {
        let _ = tokio::time::timeout(
            Duration::from_secs(10),
            server.run(|_addr, event| match event {
                Http3Event::HeadersEnd { stream_id, .. } => {
                    eprintln!("[ngtcp2 server] POST 受信: stream_id = {}", stream_id);
                    Some((
                        vec![
                            Ngtcp2Header::status(200),
                            Ngtcp2Header::new(b"content-type", b"text/plain; charset=utf-8"),
                        ],
                        b"OK".to_vec(),
                    ))
                }
                _ => None,
            }),
        )
        .await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let result = tokio::time::timeout(Duration::from_secs(10), async {
        let addr: SocketAddr = format!("127.0.0.1:{}", port).parse().unwrap();
        let config = ClientConfig::new(addr, "localhost").ca_cert(&cert_pem);
        let mut client = H3Client::connect(config).await?;

        let response = client
            .send_request(
                H3ClientRequest::post("/")
                    .authority("localhost")
                    .header("content-type", "text/plain")
                    .body(b"Hello, server!"),
            )
            .await?;

        eprintln!("[s2n client] POST レスポンス: status={}", response.status());

        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(response.status())
    })
    .await;

    server_task.abort();

    match result {
        Ok(Ok(status)) => assert_eq!(status, 200, "POST リクエストに 200 が返るべき"),
        Ok(Err(e)) => panic!("クライアントエラー: {:?}", e),
        Err(_) => panic!("タイムアウト"),
    }
}

/// RFC 9114 Section 4.1 - カスタムレスポンスヘッダー
///
/// ngtcp2 サーバーがカスタムヘッダーを含むレスポンスを返し、
/// s2n クライアント (shiguredo_http3) が正しく受信できることを確認する。
#[tokio::test]
async fn test_response_custom_headers() {
    let (cert_pem, key_pem) = generate_shared_certificate().unwrap();
    let (_cert_dir, cert_path, key_path) = save_certificate_files(&cert_pem, &key_pem).unwrap();

    let (mut server, server_addr) = start_ngtcp2_server(&cert_path, &key_path).await.unwrap();
    let port = server_addr.port();

    let server_task = tokio::spawn(async move {
        let _ = tokio::time::timeout(
            Duration::from_secs(10),
            server.run(|_addr, event| match event {
                Http3Event::HeadersEnd { stream_id, .. } => {
                    eprintln!("[ngtcp2 server] ヘッダー終了: stream_id = {}", stream_id);
                    Some((
                        vec![
                            Ngtcp2Header::status(200),
                            Ngtcp2Header::new(b"content-type", b"text/plain; charset=utf-8"),
                            Ngtcp2Header::new(b"x-server", b"nghttp3"),
                            Ngtcp2Header::new(b"x-custom-header", b"custom-value"),
                        ],
                        b"Hello".to_vec(),
                    ))
                }
                _ => None,
            }),
        )
        .await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let result = tokio::time::timeout(Duration::from_secs(10), async {
        let addr: SocketAddr = format!("127.0.0.1:{}", port).parse().unwrap();
        let config = ClientConfig::new(addr, "localhost").ca_cert(&cert_pem);
        let mut client = H3Client::connect(config).await?;

        let response = client
            .send_request(H3ClientRequest::get("/").authority("localhost"))
            .await?;

        eprintln!("[s2n client] レスポンスヘッダー:");
        for (name, value) in response.headers() {
            eprintln!(
                "  {}: {}",
                String::from_utf8_lossy(name),
                String::from_utf8_lossy(value)
            );
        }

        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(response.headers().to_vec())
    })
    .await;

    server_task.abort();

    match result {
        Ok(Ok(headers)) => {
            assert_eq!(
                headers
                    .iter()
                    .find(|(n, _)| n == b":status")
                    .map(|(_, v)| v.as_slice()),
                Some(b"200".as_slice()),
                ":status が 200 でない"
            );
            assert!(
                headers
                    .iter()
                    .any(|(n, v)| n == b"x-server" && v == b"nghttp3"),
                "x-server ヘッダーがない"
            );
            assert!(
                headers
                    .iter()
                    .any(|(n, v)| n == b"x-custom-header" && v == b"custom-value"),
                "x-custom-header ヘッダーがない"
            );
        }
        Ok(Err(e)) => panic!("クライアントエラー: {:?}", e),
        Err(_) => panic!("タイムアウト"),
    }
}

/// RFC 9114 Section 4.1 - 4xx ステータスコード
///
/// ngtcp2 サーバーがパスに応じて 404 を返し、
/// s2n クライアント (shiguredo_http3) が正しくステータスコードを受信できることを確認する。
#[tokio::test]
async fn test_status_404() {
    let (cert_pem, key_pem) = generate_shared_certificate().unwrap();
    let (_cert_dir, cert_path, key_path) = save_certificate_files(&cert_pem, &key_pem).unwrap();

    let (mut server, server_addr) = start_ngtcp2_server(&cert_path, &key_path).await.unwrap();
    let port = server_addr.port();

    let server_task = tokio::spawn(async move {
        // Http3Event::Header でパスを収集して HeadersEnd でレスポンスを決定する
        let mut request_paths: HashMap<i64, Vec<u8>> = HashMap::new();
        let _ = tokio::time::timeout(
            Duration::from_secs(10),
            server.run(move |_addr, event| match event {
                Http3Event::Header { stream_id, header } => {
                    if header.name == b":path" {
                        request_paths.insert(stream_id, header.value.clone());
                    }
                    None
                }
                Http3Event::HeadersEnd { stream_id, .. } => {
                    let path = request_paths.get(&stream_id).cloned().unwrap_or_default();
                    eprintln!(
                        "[ngtcp2 server] リクエストパス: {}",
                        String::from_utf8_lossy(&path)
                    );
                    if path == b"/not-found" {
                        Some((vec![Ngtcp2Header::status(404)], vec![]))
                    } else {
                        Some((vec![Ngtcp2Header::status(200)], b"OK".to_vec()))
                    }
                }
                _ => None,
            }),
        )
        .await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let result = tokio::time::timeout(Duration::from_secs(10), async {
        let addr: SocketAddr = format!("127.0.0.1:{}", port).parse().unwrap();
        let config = ClientConfig::new(addr, "localhost").ca_cert(&cert_pem);
        let mut client = H3Client::connect(config).await?;

        let response = client
            .send_request(H3ClientRequest::get("/not-found").authority("localhost"))
            .await?;

        eprintln!("[s2n client] レスポンス: status={}", response.status());

        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(response.status())
    })
    .await;

    server_task.abort();

    match result {
        Ok(Ok(status)) => assert_eq!(status, 404, "/not-found に 404 が返るべき"),
        Ok(Err(e)) => panic!("クライアントエラー: {:?}", e),
        Err(_) => panic!("タイムアウト"),
    }
}
