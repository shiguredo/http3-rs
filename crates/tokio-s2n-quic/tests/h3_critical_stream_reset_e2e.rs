//! H3 クリティカルストリームの RESET_STREAM 受信処理の実 QUIC 統合テスト
//!
//! 生の s2n-quic クライアントで接続し、制御 / QPACK ストリームを RESET_STREAM して、
//! サーバーの `accept_request` が H3_CLOSED_CRITICAL_STREAM をラッチしたエラーで
//! 終了することを確認する (RFC 9114 Section 6.2.1 / RFC 9204 Section 4.2)。
//!
//! 本テストはクリティカルストリームのラッチ挙動を確認するものである。接続エラーを
//! FIN として誤伝達しないことの回帰検知は `internal` モジュールの単体テスト
//! (`classify_uni_recv_connection_error_is_ignored` /
//! `test_apply_uni_recv_action_ignore_does_not_feed_fin`) が担う。
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
use shiguredo_http3::{Error as H3Error, ErrorCode};
use tokio_s2n_quic::{Error, H3Server, ServerConfig};

/// サーバーの uni タスクが SETTINGS / ストリームタイプを処理するのを待つ時間
///
/// 処理前に RESET_STREAM が届くと s2n-quic が受信バッファを破棄し、制御 / QPACK
/// ストリームとして認識されないまま RESET が処理されるため、認識後に RESET する。
const STREAM_TYPE_SETTLE_DELAY: Duration = Duration::from_millis(200);

/// サーバーを起動し、`accept` → `accept_request` を実行するタスクを返す
async fn start_server() -> (
    SocketAddr,
    String,
    tokio::task::JoinHandle<Result<(), Error>>,
) {
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

    (server_addr, cert_pem, server_task)
}

/// 生の s2n-quic クライアントで接続し、制御ストリームに SETTINGS を送る
///
/// 制御ストリーム送信端を返す (drop すると FIN になり別のエラー経路に入るため、
/// 呼び出し側で保持または reset する)。
async fn connect_and_send_settings(
    server_addr: SocketAddr,
    cert_pem: &str,
) -> (s2n_quic::Connection, s2n_quic::stream::SendStream) {
    let client = s2n_quic::Client::builder()
        .with_tls(cert_pem)
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

    (connection, control)
}

/// 受信結果が H3_CLOSED_CRITICAL_STREAM ラッチであることを検証する
fn assert_closed_critical_stream(result: Result<(), Error>) {
    match result {
        Err(Error::Http3(H3Error::ConnectionError(ErrorCode::ClosedCriticalStream))) => {}
        other => panic!("H3_CLOSED_CRITICAL_STREAM を期待したが別の結果: {other:?}"),
    }
}

/// 制御ストリームの RESET_STREAM で H3_CLOSED_CRITICAL_STREAM がラッチされる
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn control_stream_reset_latches_closed_critical_stream() {
    let (server_addr, cert_pem, server_task) = start_server().await;

    let (mut connection, mut control) = connect_and_send_settings(server_addr, &cert_pem).await;

    // accept_request が bidi ストリームを待つため、リクエストストリームを開く
    let _request_stream = connection
        .open_bidirectional_stream()
        .await
        .expect("リクエストストリームのオープンに成功すること");

    // サーバーが制御ストリームの SETTINGS を処理するのを待つ
    tokio::time::sleep(STREAM_TYPE_SETTLE_DELAY).await;

    // 制御ストリームを RESET_STREAM する
    control
        .reset(
            s2n_quic::application::Error::new(0)
                .expect("アプリケーションエラーコード 0 は VarInt 範囲内"),
        )
        .expect("RESET_STREAM の送信に成功すること");

    let result = tokio::time::timeout(Duration::from_secs(5), server_task)
        .await
        .expect("accept_request のタイムアウト待ちが完了すること")
        .expect("サーバータスクの終了に成功すること");
    assert_closed_critical_stream(result);
}

/// QPACK エンコーダーストリームの RESET_STREAM で H3_CLOSED_CRITICAL_STREAM がラッチされる
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn qpack_encoder_stream_reset_latches_closed_critical_stream() {
    let (server_addr, cert_pem, server_task) = start_server().await;

    let (mut connection, _control) = connect_and_send_settings(server_addr, &cert_pem).await;

    // accept_request が bidi ストリームを待つため、リクエストストリームを開く
    let _request_stream = connection
        .open_bidirectional_stream()
        .await
        .expect("リクエストストリームのオープンに成功すること");

    // QPACK エンコーダーストリーム (0x02) を開いてから RESET_STREAM する
    let mut encoder = connection
        .open_send_stream()
        .await
        .expect("エンコーダーストリームのオープンに成功すること");
    encoder
        .send(Bytes::from_static(&[0x02]))
        .await
        .expect("エンコーダーストリームタイプの送信に成功すること");

    // サーバーがエンコーダーストリームタイプを処理するのを待つ
    tokio::time::sleep(STREAM_TYPE_SETTLE_DELAY).await;

    encoder
        .reset(
            s2n_quic::application::Error::new(0)
                .expect("アプリケーションエラーコード 0 は VarInt 範囲内"),
        )
        .expect("RESET_STREAM の送信に成功すること");

    let result = tokio::time::timeout(Duration::from_secs(5), server_task)
        .await
        .expect("accept_request のタイムアウト待ちが完了すること")
        .expect("サーバータスクの終了に成功すること");
    assert_closed_critical_stream(result);
}
