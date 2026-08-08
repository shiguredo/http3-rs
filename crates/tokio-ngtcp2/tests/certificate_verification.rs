//! 証明書検証のテスト
//!
//! `verify_peer` による証明書チェーン検証とホスト名検証を検証する
//! (RFC 9114 Section 3.1 / RFC 9001 Section 4.4)。

mod helpers;

use std::net::SocketAddr;
use std::time::Duration;

use helpers::certs::{TestCa, generate_self_signed, write_temp_pem};
use tokio_ngtcp2::{Client, ClientWebTransportSession, Server};

/// テスト用のサーバーを起動してアドレスを返す
async fn start_server(cert_path: &std::path::Path, key_path: &std::path::Path) -> SocketAddr {
    let mut server = Server::bind(
        "127.0.0.1:0".parse().expect("test must succeed"),
        cert_path,
        key_path,
        None,
        None,
    )
    .await
    .expect("test must succeed");
    let server_addr = server.local_addr();
    tokio::spawn(async move {
        let _ = server.run(|_addr, _event| None).await;
    });
    server_addr
}

/// ハンドシェイクを実行して成功することを確認する
async fn assert_handshake_succeeds(
    connect: impl std::future::Future<Output = Result<Client, shiguredo_ngtcp2::Error>>,
) {
    let mut client = connect.await.expect("client creation must succeed");
    let result = tokio::time::timeout(Duration::from_secs(5), client.handshake()).await;
    assert!(matches!(result, Ok(Ok(()))), "ハンドシェイクが成功すること");
}

/// ハンドシェイクが失敗 (即時 Err) することを確認する
///
/// 証明書検証の失敗は TLS アラートとして即時に Err になる。タイムアウトは
/// サーバー不調等を意味し、検証失敗としてみなさない (回帰検出力を保つため)。
async fn assert_handshake_fails(
    connect: impl std::future::Future<Output = Result<Client, shiguredo_ngtcp2::Error>>,
) {
    let mut client = connect.await.expect("client creation must succeed");
    let result = tokio::time::timeout(Duration::from_secs(5), client.handshake()).await;
    assert!(
        matches!(result, Ok(Err(_))),
        "ハンドシェイクが即時に失敗すること: {result:?}"
    );
}

#[tokio::test]
async fn test_verify_peer_fails_with_self_signed_cert() {
    // verify_peer=true では自己署名証明書のサーバーへの接続が失敗する
    // (チェーン検証が効いていることの確認)
    let (cert_pem, key_pem) = generate_self_signed(&["localhost".to_string()]);
    let (cert_path, key_path) = write_temp_pem(&cert_pem, &key_pem);
    let server_addr = start_server(&cert_path, &key_path).await;

    assert_handshake_fails(Client::connect(server_addr, "localhost", None, None)).await;
}

#[tokio::test]
async fn test_verify_peer_with_ca_succeeds_on_hostname_match() {
    // verify_peer=true + カスタム CA ロードで、CA が署名した証明書かつ
    // ホスト名一致のサーバーへの接続が成功する
    let ca = TestCa::new();
    let (cert_pem, key_pem) = ca.issue_server_cert(&["localhost".to_string()]);
    let (cert_path, key_path) = write_temp_pem(&cert_pem, &key_pem);
    let server_addr = start_server(&cert_path, &key_path).await;

    assert_handshake_succeeds(Client::connect_with_ca(
        server_addr,
        "localhost",
        ca.cert_pem(),
        None,
        None,
    ))
    .await;
}

#[tokio::test]
async fn test_verify_peer_with_ca_fails_on_hostname_mismatch() {
    // verify_peer=true + カスタム CA ロードで、ホスト名不一致のサーバーへの
    // 接続が失敗する (ホスト名検証が効いていることの確認)
    let ca = TestCa::new();
    // 証明書は example.com 用に発行し、localhost に接続する
    let (cert_pem, key_pem) = ca.issue_server_cert(&["example.com".to_string()]);
    let (cert_path, key_path) = write_temp_pem(&cert_pem, &key_pem);
    let server_addr = start_server(&cert_path, &key_path).await;

    assert_handshake_fails(Client::connect_with_ca(
        server_addr,
        "localhost",
        ca.cert_pem(),
        None,
        None,
    ))
    .await;
}

#[tokio::test]
async fn test_verify_peer_without_ca_fails_with_ca_signed_cert() {
    // verify_peer=true でもカスタム CA をロードしなければ、その CA が署名した
    // 証明書のサーバーへの接続は失敗する (CA ロードが機能していることの確認)
    let ca = TestCa::new();
    let (cert_pem, key_pem) = ca.issue_server_cert(&["localhost".to_string()]);
    let (cert_path, key_path) = write_temp_pem(&cert_pem, &key_pem);
    let server_addr = start_server(&cert_path, &key_path).await;

    assert_handshake_fails(Client::connect(server_addr, "localhost", None, None)).await;
}

#[tokio::test]
async fn test_verify_peer_false_succeeds_with_self_signed_cert() {
    // verify_peer=false では自己署名証明書のサーバーへの接続が成功する
    // (既存挙動の維持)
    let (cert_pem, key_pem) = generate_self_signed(&["localhost".to_string()]);
    let (cert_path, key_path) = write_temp_pem(&cert_pem, &key_pem);
    let server_addr = start_server(&cert_path, &key_path).await;

    assert_handshake_succeeds(Client::connect_insecure(
        server_addr,
        "localhost",
        None,
        None,
    ))
    .await;
}

#[tokio::test]
async fn test_webtransport_connect_with_ca_succeeds() {
    // ClientWebTransportSession::connect_with_ca でもカスタム CA ロードと
    // ホスト名検証が機能することを検証する
    let ca = TestCa::new();
    let (cert_pem, key_pem) = ca.issue_server_cert(&["localhost".to_string()]);
    let (cert_path, key_path) = write_temp_pem(&cert_pem, &key_pem);
    let server_addr = start_server(&cert_path, &key_path).await;

    let mut client = ClientWebTransportSession::connect_with_ca(
        server_addr,
        "localhost",
        "/webtransport",
        ca.cert_pem(),
    )
    .await
    .expect("client creation must succeed");
    let result = tokio::time::timeout(Duration::from_secs(5), client.handshake()).await;
    assert!(
        matches!(result, Ok(Ok(()))),
        "WebTransport クライアントのハンドシェイクが成功すること"
    );
}

#[tokio::test]
async fn test_server_name_rejects_ip_address() {
    // server_name に IP アドレスを渡すと接続作成時にエラーになる
    // (ホスト名検証は DNS 名限定のため)
    let result = Client::connect(
        "127.0.0.1:14433".parse().expect("test must succeed"),
        "127.0.0.1",
        None,
        None,
    )
    .await;
    assert!(
        result.is_err(),
        "IP アドレスの server_name は拒否されること"
    );
}

#[tokio::test]
async fn test_server_name_rejects_empty_string() {
    // server_name に空文字列を渡すと接続作成時にエラーになる
    // (ホスト名検証がサイレントにスキップされるのを防ぐ)
    let result = Client::connect(
        "127.0.0.1:14433".parse().expect("test must succeed"),
        "",
        None,
        None,
    )
    .await;
    assert!(result.is_err(), "空文字列の server_name は拒否されること");
}

#[tokio::test]
async fn test_server_name_rejects_wildcard() {
    // server_name にワイルドカードを渡すと接続作成時にエラーになる
    // (SNI の HostName は FQDN 限定のため。RFC 6066 Section 3)
    let result = Client::connect(
        "127.0.0.1:14433".parse().expect("test must succeed"),
        "*.example.com",
        None,
        None,
    )
    .await;
    assert!(
        result.is_err(),
        "ワイルドカードの server_name は拒否されること"
    );
}

#[tokio::test]
async fn test_server_name_rejects_overlong_name() {
    // server_name に 255 文字超の名前を渡すと接続作成時にエラーになる
    // (DNS 名 (FQDN) の長さ制限は 255 オクテット。RFC 1035 Section 2.3.4)
    let long_name = format!("{}.example.com", "a".repeat(256));
    let result = Client::connect(
        "127.0.0.1:14433".parse().expect("test must succeed"),
        &long_name,
        None,
        None,
    )
    .await;
    assert!(result.is_err(), "255 文字超の server_name は拒否されること");
}
