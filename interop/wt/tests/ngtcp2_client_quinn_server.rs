//! ngtcp2 クライアント ↔ quinn (h3-webtransport) サーバー WebTransport 相互運用性テスト
//!
//! h3-webtransport は draft-02 を使用。
//! ngtcp2 (nghttp3) は draft-15 を使用するためネゴシエーション失敗が想定される。

use std::time::Duration;

use serial_test::serial;
use tokio::sync::mpsc;
use tokio::time::timeout;

use interop_wt::{generate_shared_certificate, start_quinn_wt_server};
use tokio_ngtcp2::ClientWebTransportSession;

#[serial]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_webtransport_session() {
    let (cert_pem, key_pem) = generate_shared_certificate().expect("test must succeed");

    let (port_tx, port_rx) = std::sync::mpsc::channel();
    let (shutdown_tx, shutdown_rx) = mpsc::channel(1);

    // quinn WT サーバーを起動
    let server_task = tokio::spawn(async move {
        if let Err(e) = start_quinn_wt_server(cert_pem, key_pem, port_tx, shutdown_rx).await {
            eprintln!("[test] server error: {:?}", e);
        }
    });

    let port = port_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("test must succeed");
    eprintln!("[test] server port: {}", port);

    tokio::time::sleep(Duration::from_millis(100)).await;

    let server_addr: std::net::SocketAddr = format!("127.0.0.1:{}", port)
        .parse()
        .expect("test must succeed");

    // ngtcp2 クライアントで WT セッション確立を試行
    let client_result = timeout(Duration::from_secs(10), async {
        let mut session =
            ClientWebTransportSession::connect_insecure(server_addr, "localhost", "/webtransport")
                .await
                .expect("client creation failed");

        eprintln!("[ngtcp2 client] handshake start");
        session.handshake().await.expect("handshake failed");
        eprintln!("[ngtcp2 client] handshake complete");

        let session_result = session
            .open_session(&format!("localhost:{}", port), "/webtransport")
            .await;

        match session_result {
            Ok(session_id) => {
                eprintln!(
                    "[ngtcp2 client] WT session established: session_id = {}",
                    session_id
                );
                Some(session_id)
            }
            Err(e) => {
                eprintln!("[ngtcp2 client] WT session failed: {:?}", e);
                None
            }
        }
    })
    .await;

    let _ = shutdown_tx.send(()).await;
    server_task.abort();

    match client_result {
        Ok(Some(session_id)) => {
            eprintln!("[test] WT session established: session_id = {}", session_id);
            assert_eq!(session_id, 0, "最初のセッション ID は 0 であること");
        }
        Ok(None) => {
            panic!("WT session failed (expected: success)");
        }
        Err(_) => {
            panic!("timeout");
        }
    }
}
