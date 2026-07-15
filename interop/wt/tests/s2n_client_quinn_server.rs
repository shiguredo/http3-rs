//! s2n-quic クライアント ↔ quinn (h3-webtransport) サーバー WebTransport 相互運用性テスト
//!
//! セッション確立 (RFC draft-ietf-webtrans-http3 Section 3) のテスト
//! h3-webtransport は draft-02 を使用、s2n-quic (shiguredo_http3) は draft-02 対応

use std::time::Duration;

use serial_test::serial;
use tokio::sync::mpsc;
use tokio::time::timeout;

use interop_wt::{generate_shared_certificate, start_quinn_wt_server};
use tokio_s2n_quic::{ClientConfig, DraftVersion, WtClient};

#[serial]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_webtransport_session() {
    let (cert_pem, key_pem) = generate_shared_certificate().expect("test must succeed");

    let (port_tx, port_rx) = std::sync::mpsc::channel();
    let (shutdown_tx, shutdown_rx) = mpsc::channel(1);

    let cert_pem_for_client = cert_pem.clone();

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

    // s2n-quic クライアントで WT セッション確立
    let client_result = timeout(Duration::from_secs(10), async {
        let addr: std::net::SocketAddr = format!("127.0.0.1:{}", port)
            .parse()
            .expect("test must succeed");
        let config = ClientConfig::new(addr, "localhost")
            .ca_cert(&cert_pem_for_client)
            .enable_webtransport(interop_wt::test_wt_settings())
            .wt_draft_version(DraftVersion::Draft02);
        let session = WtClient::connect(config, "/webtransport").await?;
        eprintln!(
            "[s2n client] WT session established: session_id = {}",
            session.session_id()
        );
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(session.session_id())
    })
    .await;

    let _ = shutdown_tx.send(()).await;
    server_task.abort();

    match client_result {
        Ok(Ok(session_id)) => {
            eprintln!("[test] WT session established: session_id = {}", session_id);
        }
        Ok(Err(e)) => {
            panic!("client error: {:?}", e);
        }
        Err(_) => {
            panic!("timeout");
        }
    }
}
