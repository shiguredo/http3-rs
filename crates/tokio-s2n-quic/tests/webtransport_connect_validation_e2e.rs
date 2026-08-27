//! WebTransport CONNECT 検証の実 QUIC 統合テスト
//!
//! ループバック接続で以下を検証する:
//! - サーバーが `WtSessionRequest::reject(405)` で拒否した場合、クライアントが
//!   セッション確立失敗 (`Error::ConnectionClosed`) として扱う
//!   (draft-ietf-webtrans-http3-16 Section 3.2)
//!
//! `:method` / `:protocol` 検証は sans-I/O 層の `ConnectRequest::from_headers` に
//! 委譲しており、そちらの単体テスト (`shiguredo_http3::webtransport::connect::tests`)
//! で `test_connect_request_from_headers_invalid_method` /
//! `test_connect_request_from_headers_invalid_protocol` として担保されている。
//!
//! モック・スタブは使用しない (実 QUIC 接続を利用する)。

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use rcgen::generate_simple_self_signed;
use shiguredo_http3::{VarInt, webtransport};
use tokio_s2n_quic::{ClientConfig, Error, ServerConfig, WtClient, WtServer};

fn generate_certificate() -> (String, String) {
    let subject_alt_names = vec!["localhost".to_string(), "127.0.0.1".to_string()];
    let certified_key =
        generate_simple_self_signed(subject_alt_names).expect("自己署名証明書生成に成功すること");
    (
        certified_key.cert.pem(),
        certified_key.signing_key.serialize_pem(),
    )
}

fn build_wt_settings() -> webtransport::Settings {
    let v =
        |value: u64| VarInt::new(value).expect("WT settings のバリューが VarInt 範囲内であること");
    webtransport::Settings::new()
        .wt_enabled(VarInt::from_static(1))
        .wt_initial_max_streams_bidi(v(100))
        .wt_initial_max_streams_uni(v(100))
        .wt_initial_max_data(v(1_048_576))
}

async fn start_server() -> (WtServer, SocketAddr, String) {
    let (cert_pem, key_pem) = generate_certificate();
    let listen_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0);
    let config =
        ServerConfig::new(listen_addr, &cert_pem, key_pem).enable_webtransport(build_wt_settings());
    let server = WtServer::bind(config).expect("サーバー bind に成功すること");
    let addr = server.local_addr();
    (server, addr, cert_pem)
}

fn build_client_config(server_addr: SocketAddr, ca_cert_pem: String) -> ClientConfig {
    ClientConfig::new(server_addr, "localhost")
        .ca_cert(ca_cert_pem)
        .enable_webtransport(build_wt_settings())
}

/// サーバーが `reject(405)` を呼ぶとクライアント側の `WtClient::connect` が
/// `Error::ConnectionClosed` を返すことを検証する
///
/// (draft-ietf-webtrans-http3-16 Section 3.2: セッション確立は 2xx 受信時のみ)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_reject_causes_client_error() {
    let (mut server, server_addr, ca_cert_pem) = start_server().await;

    // サーバー側で accept 後に reject(405) を呼ぶ
    let server_task = tokio::spawn(async move {
        let request = server
            .accept()
            .await
            .expect("サーバー側の accept に成功すること");
        request
            .reject(405)
            .await
            .expect("サーバー側の reject に成功すること");
    });

    // クライアント接続は失敗する
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        WtClient::connect(build_client_config(server_addr, ca_cert_pem), "/"),
    )
    .await
    .expect("クライアント接続タイムアウト待ちが完了すること");

    match result {
        Err(Error::ConnectionClosed) => {}
        Err(other) => panic!(
            "ConnectionClosed 以外のエラーが返った (405 拒否では ConnectionClosed になるべき): {other:?}"
        ),
        Ok(_) => panic!("405 拒否でもセッション確立成功が返った (許容できない)"),
    }

    server_task
        .await
        .expect("サーバータスクの終了に成功すること");
}
