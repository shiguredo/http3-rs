//! 設定フィールドの配線を実 QUIC 接続で確認する統合テスト
//!
//! `ServerConfig::alpn` が TLS プロバイダの ALPN に反映されることを、
//! ALPN を意図的に食い違わせたハンドシェイク失敗で確認する。
//! s2n-quic のデフォルト TLS プロバイダは `h3` を自動設定するため、
//! 設定値が無視されていればハンドシェイクが成功してしまう。
//!
//! モック・スタブは使用しない (実 QUIC 接続を利用する)。

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use shiguredo_http3::{VarInt, webtransport};
use tokio_s2n_quic::{ClientConfig, ServerConfig, WtClient, WtServer};

#[path = "helpers/certs.rs"]
mod certs;

use certs::generate_certificate;

/// テスト用の WebTransport 設定 (draft-15) を返す
fn build_wt_settings() -> webtransport::Settings {
    let v =
        |value: u64| VarInt::new(value).expect("WT settings のバリューが VarInt 範囲内であること");
    webtransport::Settings::new()
        .wt_enabled(VarInt::from_static(1))
        .wt_initial_max_streams_bidi(v(100))
        .wt_initial_max_streams_uni(v(100))
        .wt_initial_max_data(v(1_048_576))
}

/// サーバーの ALPN を既定の h3 から変更すると、h3 を提示するクライアントとの
/// ハンドシェイクが失敗すること (`ServerConfig::alpn` が配線されていることの確認)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_alpn_is_applied_to_tls_handshake() {
    let (cert_pem, key_pem) = generate_certificate();
    let listen_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0);

    let mut config =
        ServerConfig::new(listen_addr, &cert_pem, key_pem).enable_webtransport(build_wt_settings());
    // サーバーの ALPN を既定の h3 から変更する (設定値が TLS に配線されていれば
    // h3 を提示するクライアントとのハンドシェイクが成立しない)
    config.alpn = vec![b"h3-alt".to_vec()];
    let mut server = WtServer::bind(config).expect("サーバー bind に成功すること");
    let addr = server.local_addr();

    // サーバーはハンドシェイクが失敗するためセッションリクエストを受け付けない。
    // テスト終了時にタスクを abort する。
    let server_task = tokio::spawn(async move {
        let _ = server.accept().await;
    });

    let config = ClientConfig::new(addr, "localhost")
        .ca_cert(cert_pem)
        .enable_webtransport(build_wt_settings());
    let result = tokio::time::timeout(Duration::from_secs(15), WtClient::connect(config, "/"))
        .await
        .expect("ALPN 不一致のハンドシェイクがタイムアウトしないこと");
    assert!(
        result.is_err(),
        "ALPN 不一致では接続に失敗すること (ServerConfig::alpn が無視されている可能性)"
    );

    server_task.abort();
}
