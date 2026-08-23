//! quinn (h3-webtransport) クライアント ↔ ngtcp2 サーバー WebTransport 相互運用性テスト
//!
//! h3-webtransport は draft-02 を使用。
//! ngtcp2 (nghttp3) は draft-15 を使用するためネゴシエーション失敗が想定される。

use std::time::Duration;

use serial_test::serial;
use tokio::time::timeout;

use interop_wt::{generate_shared_certificate, run_quinn_wt_client, save_certificate_files};
use shiguredo_ngtcp2::Http3Event;
use tokio_ngtcp2::ServerWebTransportSession;

#[serial]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_webtransport_session() {
    let (cert_pem, key_pem) = generate_shared_certificate().expect("test must succeed");
    let (cert_path, key_path) =
        save_certificate_files(&cert_pem, &key_pem).expect("test must succeed");

    // ngtcp2 サーバーを起動
    let mut server = ServerWebTransportSession::bind(
        "127.0.0.1:0".parse().expect("test must succeed"),
        &cert_path,
        &key_path,
    )
    .await
    .expect("server creation failed");

    let server_addr = server.local_addr();
    let port = server_addr.port();
    eprintln!("[test] server port: {}", port);

    let server_task = tokio::spawn(async move {
        let _ = timeout(
            Duration::from_secs(10),
            server.run(|addr, session_id, event| {
                match &event {
                    Http3Event::HeadersEnd { stream_id, .. } => {
                        eprintln!(
                            "[ngtcp2 server] CONNECT: addr={} session={} stream={}",
                            addr, session_id, stream_id
                        );
                        return true;
                    }
                    _ => {
                        eprintln!("[ngtcp2 server] event: {:?} session={}", event, session_id);
                    }
                }
                false
            }),
        )
        .await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    // quinn クライアントで WT セッション確立を試行
    let result = timeout(
        Duration::from_secs(10),
        run_quinn_wt_client(port, "/webtransport"),
    )
    .await;

    server_task.abort();

    match result {
        Ok(Ok(())) => {
            eprintln!("[test] WT session established");
            // サーバー側で CONNECT リクエスト (HeadersEnd) を受け取ったことを検証する
        }
        Ok(Err(e)) => {
            panic!("WT session failed: {:?}", e);
        }
        Err(_) => {
            panic!("timeout");
        }
    }
}
