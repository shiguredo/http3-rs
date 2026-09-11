//! H3Server の WebTransport bidi ストリーム拒否の実 QUIC 統合テスト
//!
//! 生の s2n-quic クライアントで制御ストリーム (SETTINGS) を送ったあと 0x41
//! (WT_STREAM) 始まりの双方向ストリームを開き、`H3ServerConnection::accept_request`
//! がハングせず `Error::InvalidState` で終了し、ピアに
//! WT_BUFFERED_STREAM_REJECTED (0x3994bd84) の RESET_STREAM が返ることを確認する。
//! あわせて、WebTransport を有効化した `ServerConfig` を `H3Server::bind` が
//! 拒否することを確認する (draft-ietf-webtrans-http3-16 Section 4.3 / 4.6)。
//!
//! モック・スタブは使用しない (実 QUIC 接続を利用する)。

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

// テスト間で共有するヘルパーは tests/helpers/ に置き、必要なファイルだけを
// 明示的に取り込む (モジュール全体を取り込むと未使用部分が dead code になるため)
#[path = "helpers/certs.rs"]
mod certs;

use bytes::Bytes;
use certs::generate_certificate;
use s2n_quic::client::Connect;
use shiguredo_http3::{VarInt, webtransport};
use tokio_s2n_quic::{Error, H3Server, ServerConfig};

/// WebTransport を有効化した `ServerConfig` を作る
fn webtransport_enabled_config(
    cert_pem: &str,
    key_pem: &str,
    listen_addr: SocketAddr,
) -> ServerConfig {
    let wt_settings = webtransport::Settings::new().wt_enabled(VarInt::from_static(1));
    ServerConfig::new(listen_addr, cert_pem, key_pem).enable_webtransport(wt_settings)
}

/// ピアの 0x41 bidi ストリームで `accept_request` がハングせずエラー終了する
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accept_request_returns_invalid_state_on_wt_bidi_stream() {
    let (cert_pem, key_pem) = generate_certificate();
    let listen_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0);
    let config = ServerConfig::new(listen_addr, &cert_pem, key_pem);
    let mut server = H3Server::bind(config).expect("サーバー bind に成功すること");
    let server_addr = server.local_addr();

    let server_task = tokio::spawn(async move {
        let mut connection = server
            .accept()
            .await
            .expect("サーバー側の accept に成功すること");
        connection.accept_request().await.map(|_| ())
    });

    // 生の s2n-quic クライアントで接続する
    let client = s2n_quic::Client::builder()
        .with_tls(cert_pem.as_str())
        .expect("クライアント TLS の構築に成功すること")
        .with_io("0.0.0.0:0")
        .expect("クライアント IO の構築に成功すること")
        .start()
        .expect("クライアントの起動に成功すること");
    let mut connection = client
        .connect(Connect::new(server_addr).with_server_name("localhost"))
        .await
        .expect("接続に成功すること");

    // 制御ストリーム (0x00) に SETTINGS フレーム (type=4, length=0) を送る。
    // ピア SETTINGS 受信後は 0x41 bidi が WT 非対応として即座に拒否される
    // (未受信の場合は SETTINGS 受信時の再ディスパッチで拒否される)
    let mut control = connection
        .open_send_stream()
        .await
        .expect("制御ストリームのオープンに成功すること");
    control
        .send(Bytes::from_static(&[0x00, 0x04, 0x00]))
        .await
        .expect("SETTINGS の送信に成功すること");

    // 0x41 (WT_STREAM) は 2 バイト varint で 0x40 0x41。WT bidi ストリームの
    // 先頭シグナルとして送る (draft-ietf-webtrans-http3-16 Section 4.3)
    let stream = connection
        .open_bidirectional_stream()
        .await
        .expect("双方向ストリームのオープンに成功すること");
    let (mut recv, mut send) = stream.split();
    send.send(Bytes::from_static(&[0x40, 0x41]))
        .await
        .expect("WT シグナルの送信に成功すること");

    // サーバーが WT_BUFFERED_STREAM_REJECTED (0x3994bd84) で RESET_STREAM を
    // 返すこと (draft-ietf-webtrans-http3-16 Section 4.6 とイベント契約に基づく)
    let reset = tokio::time::timeout(Duration::from_secs(5), recv.receive())
        .await
        .expect("RESET_STREAM のタイムアウト待ちが完了すること")
        .expect_err("RESET_STREAM を期待した");
    match reset {
        s2n_quic::stream::Error::StreamReset { error, .. } => {
            assert_eq!(
                *error, 0x3994bd84,
                "WT_BUFFERED_STREAM_REJECTED (0x3994bd84) で reset されること"
            );
        }
        other => panic!("StreamReset を期待したが別のエラーが返った: {other}"),
    }

    // ハング (10ms ポーリングの無限ループ) せず InvalidState で終了すること。
    // control はスコープ終了まで保持する (drop すると制御ストリームの FIN が
    // 伝わり、H3_CLOSED_CRITICAL_STREAM で別のエラー経路に入るため)
    let result = tokio::time::timeout(Duration::from_secs(5), server_task)
        .await
        .expect("accept_request のタイムアウト待ちが完了すること")
        .expect("サーバータスクの終了に成功すること");
    match result {
        Err(Error::InvalidState(_)) => {}
        Err(other) => panic!("InvalidState を期待したが別のエラーが返った: {other}"),
        Ok(()) => panic!("0x41 bidi ストリームで accept_request がエラーになること"),
    }
}

/// WebTransport 有効化済みの `ServerConfig` は `H3Server::bind` が拒否する
#[test]
fn bind_rejects_webtransport_enabled_server_config() {
    let (cert_pem, key_pem) = generate_certificate();
    let listen_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0);
    let config = webtransport_enabled_config(&cert_pem, &key_pem, listen_addr);
    match H3Server::bind(config) {
        Err(Error::InvalidState(_)) => {}
        Err(other) => panic!("InvalidState を期待したが別のエラーが返った: {other}"),
        Ok(_) => panic!("WebTransport 有効化済み ServerConfig は拒否されること"),
    }
}
