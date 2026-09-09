//! H3 リクエストストリームの RESET_STREAM 受信処理の実 QUIC 統合テスト
//!
//! 生の s2n-quic クライアントで接続し、制御ストリーム (SETTINGS) を送ったあと
//! 双方向ストリームを RESET_STREAM して、サーバーの `accept_request` が
//! 受信ループの Err 分岐でエラー終了することを確認する。
//!
//! 本テストは Err 分岐を実行するスモークテストである。`h3_conn.stream_reset`
//! 呼び出しが StreamReset イベントを生成することは `internal::connection_state`
//! の単体テストで検証する。
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
use tokio_s2n_quic::{H3Server, ServerConfig};

/// ピアの RESET_STREAM 受信で `accept_request` がエラー終了する
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accept_request_returns_error_on_stream_reset() {
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
        connection.accept_request().await
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

    // 制御ストリーム (0x00) に SETTINGS フレーム (type=4, length=0) を送る
    let mut control = connection
        .open_send_stream()
        .await
        .expect("制御ストリームのオープンに成功すること");
    control
        .send(Bytes::from_static(&[0x00, 0x04, 0x00]))
        .await
        .expect("SETTINGS の送信に成功すること");

    // リクエストストリームを開いて RESET_STREAM する
    let stream = connection
        .open_bidirectional_stream()
        .await
        .expect("双方向ストリームのオープンに成功すること");
    let (_recv, mut send) = stream.split();
    send.reset(
        s2n_quic::application::Error::new(0)
            .expect("アプリケーションエラーコード 0 は VarInt 範囲内"),
    )
    .expect("RESET_STREAM の送信に成功すること");

    let result = tokio::time::timeout(Duration::from_secs(5), server_task)
        .await
        .expect("accept_request のタイムアウト待ちが完了すること")
        .expect("サーバータスクの終了に成功すること");
    // 受信ループの `Err(e) => return Err(crate::Error::transport(e))` 経路を通ること
    match result {
        Err(tokio_s2n_quic::Error::Transport(_)) => {}
        Err(other) => panic!("Transport エラーを期待したが別のエラーが返った: {other}"),
        Ok(_) => panic!("RESET_STREAM 受信で accept_request がエラーになること"),
    }
}
