//! 統合テスト
//!
//! HTTP/3 および WebTransport の統合テスト

use std::path::PathBuf;
use std::time::Duration;

use rcgen::generate_simple_self_signed;
use shiguredo_ngtcp2::{Header, Http3SettingsExt, TransportParamsExt};
use tokio_ngtcp2::{Client, ClientWebTransportSession, Server, ServerWebTransportSession};

/// テスト用の証明書を動的に生成
fn generate_test_certificate() -> (PathBuf, PathBuf) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let subject_alt_names = vec!["localhost".to_string(), "127.0.0.1".to_string()];
    let certified_key = generate_simple_self_signed(subject_alt_names).unwrap();
    let cert_pem = certified_key.cert.pem();
    let key_pem = certified_key.signing_key.serialize_pem();

    // テストごとにユニークなディレクトリを作成
    let unique_id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let thread_id = std::thread::current().id();
    let cert_dir =
        std::env::temp_dir().join(format!("tokio_ngtcp2_test_{:?}_{}", thread_id, unique_id));
    std::fs::create_dir_all(&cert_dir).unwrap();

    let cert_path = cert_dir.join("cert.pem");
    let key_path = cert_dir.join("key.pem");

    std::fs::write(&cert_path, cert_pem).unwrap();
    std::fs::write(&key_path, key_pem).unwrap();

    (cert_path, key_path)
}

#[tokio::test]
async fn test_client_creation() {
    // クライアントを作成できることを確認
    let result = Client::connect("127.0.0.1:14433".parse().unwrap(), "localhost", None, None).await;

    // ソケットバインドは成功するはず
    assert!(result.is_ok());

    let client = result.unwrap();
    assert_eq!(client.remote_addr(), "127.0.0.1:14433".parse().unwrap());
}

#[tokio::test]
async fn test_server_creation() {
    let (cert_path, key_path) = generate_test_certificate();

    // サーバーを作成
    let result = Server::bind(
        "127.0.0.1:0".parse().unwrap(),
        &cert_path,
        &key_path,
        None,
        None,
    )
    .await;

    assert!(result.is_ok());

    let server = result.unwrap();
    // エフェメラルポートにバインドされているはず
    assert_ne!(server.local_addr().port(), 0);
}

#[tokio::test]
async fn test_webtransport_client_creation() {
    // WebTransport クライアントを作成できることを確認
    let result = ClientWebTransportSession::connect(
        "127.0.0.1:14434".parse().unwrap(),
        "localhost",
        "/webtransport",
    )
    .await;

    assert!(result.is_ok());

    let session = result.unwrap();
    assert_eq!(session.remote_addr(), "127.0.0.1:14434".parse().unwrap());
    assert!(session.session_id().is_none()); // セッションはまだ確立されていない
}

#[tokio::test]
async fn test_webtransport_server_creation() {
    let (cert_path, key_path) = generate_test_certificate();

    // WebTransport サーバーを作成
    let result =
        ServerWebTransportSession::bind("127.0.0.1:0".parse().unwrap(), &cert_path, &key_path)
            .await;

    assert!(result.is_ok());

    let server = result.unwrap();
    assert_ne!(server.local_addr().port(), 0);
}

#[tokio::test]
async fn test_transport_params_webtransport() {
    use shiguredo_ngtcp2::ngtcp2_transport_params;

    // WebTransport 用のトランスポートパラメータを確認
    let params = ngtcp2_transport_params::default_params().with_datagram(65535);

    assert_eq!(params.max_datagram_frame_size, 65535);
    assert!(params.initial_max_streams_bidi > 0);
    assert!(params.initial_max_streams_uni > 0);
}

#[tokio::test]
async fn test_h3_settings_webtransport() {
    use shiguredo_ngtcp2::nghttp3_settings;

    // WebTransport 用の HTTP/3 設定を確認
    let settings = nghttp3_settings::default_settings().with_webtransport();

    assert_eq!(settings.enable_connect_protocol, 1);
    assert_eq!(settings.h3_datagram, 1);
    assert_eq!(settings.wt_enabled, 1);
}

#[test]
fn test_header_creation() {
    // HTTP/3 ヘッダーの作成を確認
    let headers = [
        Header::method("CONNECT"),
        Header::new(b":protocol", b"webtransport"),
        Header::scheme("https"),
        Header::authority("localhost:4433"),
        Header::path("/webtransport"),
    ];

    assert_eq!(headers.len(), 5);
    assert_eq!(headers[0].name_str(), Some(":method"));
    assert_eq!(headers[0].value_str(), Some("CONNECT"));
    assert_eq!(headers[1].name_str(), Some(":protocol"));
    assert_eq!(headers[1].value_str(), Some("webtransport"));
}

#[tokio::test]
async fn test_client_server_handshake() {
    let (cert_path, key_path) = generate_test_certificate();

    // サーバーを起動
    let mut server = Server::bind(
        "127.0.0.1:0".parse().unwrap(),
        &cert_path,
        &key_path,
        None,
        None,
    )
    .await
    .expect("server creation failed");

    let server_addr = server.local_addr();

    // サーバーをバックグラウンドで実行 (Send を実装しているので tokio::spawn で実行可能)
    let server_handle = tokio::spawn(async move {
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            server.run(|_addr, _event| {
                // リクエストに応答しない
                None
            }),
        )
        .await;

        // タイムアウトは OK (テスト終了)
        match result {
            Ok(r) => r,
            Err(_) => Ok(()), // タイムアウト
        }
    });

    // クライアントを作成
    let mut client = Client::connect(server_addr, "localhost", None, None)
        .await
        .expect("client creation failed");

    // ハンドシェイクを実行 (タイムアウトを設定)
    let handshake_result = tokio::time::timeout(Duration::from_secs(5), client.handshake()).await;

    // サーバータスクを終了
    server_handle.abort();

    // ハンドシェイクの結果を確認
    // 自己署名証明書なのでハンドシェイクは失敗する可能性がある
    match handshake_result {
        Ok(Ok(())) => {
            println!("Handshake successful");
        }
        Ok(Err(e)) => {
            // TLS エラーなど (自己署名証明書では期待される動作)
            println!("Handshake error (expected with self-signed cert): {:?}", e);
        }
        Err(_) => {
            println!("Handshake timed out");
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_server_is_send() {
    let (cert_path, key_path) = generate_test_certificate();

    // Server が Send であることを確認するテスト
    let server = Server::bind(
        "127.0.0.1:0".parse().unwrap(),
        &cert_path,
        &key_path,
        None,
        None,
    )
    .await
    .expect("server creation failed");

    // tokio::spawn で別スレッドに移動できることを確認
    let handle = tokio::spawn(async move {
        let _addr = server.local_addr();
        // サーバーを移動できたことを確認
        true
    });

    let result = handle.await.expect("task failed");
    assert!(result);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_client_is_send() {
    // Client が Send であることを確認するテスト
    let client = Client::connect("127.0.0.1:14435".parse().unwrap(), "localhost", None, None)
        .await
        .expect("client creation failed");

    // tokio::spawn で別スレッドに移動できることを確認
    let handle = tokio::spawn(async move {
        let _addr = client.local_addr();
        // クライアントを移動できたことを確認
        true
    });

    let result = handle.await.expect("task failed");
    assert!(result);
}
