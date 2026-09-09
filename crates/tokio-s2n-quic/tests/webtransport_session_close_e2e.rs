//! CONNECT ストリーム受信タスクの実 QUIC 統合テスト
//!
//! ループバック接続でサーバー・クライアントを立ち上げ、`WtSession::close` と
//! CONNECT ストリームのクリーンクローズ (FIN のみ) が `recv_event()` で
//! 検知できることを確認する
//! (draft-ietf-webtrans-http3-16 Section 6)。
//!
//! モック・スタブは使用しない (実 QUIC 接続を利用する)。

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

// テスト間で共有するヘルパーは tests/helpers/ に置き、必要なファイルだけを
// 明示的に取り込む (モジュール全体を取り込むと未使用部分が dead code になるため)
#[path = "helpers/certs.rs"]
mod certs;

use certs::generate_certificate;
use shiguredo_http3::{VarInt, webtransport};
use tokio::sync::oneshot;
use tokio_s2n_quic::{ClientConfig, ServerConfig, WebTransportEvent, WtClient, WtServer};

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

/// サーバーを起動し、リッスンアドレス、WtServer、サーバー証明書 PEM を返す
async fn start_server() -> (WtServer, SocketAddr, String) {
    let (cert_pem, key_pem) = generate_certificate();
    // ポート 0 を指定して OS に空きポートを割り当てさせる
    let listen_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0);
    let config =
        ServerConfig::new(listen_addr, &cert_pem, key_pem).enable_webtransport(build_wt_settings());
    let server = WtServer::bind(config).expect("サーバー bind に成功すること");
    let addr = server.local_addr();
    (server, addr, cert_pem)
}

/// クライアント設定を構築する (サーバーの自己署名証明書を CA として渡す)
fn build_client_config(server_addr: SocketAddr, ca_cert_pem: String) -> ClientConfig {
    ClientConfig::new(server_addr, "localhost")
        .ca_cert(ca_cert_pem)
        .enable_webtransport(build_wt_settings())
}

/// サーバー側で `WtSession::close` を呼ぶと、クライアント側の `recv_event()` で
/// `SessionClosed { close_error_code, close_message, .. }` が届くことを検証する
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_close_delivers_session_closed_to_client() {
    let (mut server, server_addr, ca_cert_pem) = start_server().await;

    // クライアント接続が完了したことをサーバー側に通知するチャネル
    let (client_ready_tx, client_ready_rx) = oneshot::channel::<()>();

    let server_task = tokio::spawn(async move {
        let request = server
            .accept()
            .await
            .expect("サーバー側の accept に成功すること");
        let mut session = request
            .accept()
            .await
            .expect("セッション確立に成功すること");
        // クライアント側の受信タスクが起動する前に close を送ると
        // pending リレー経路をたどるため、明示的にクライアント準備完了を待つ
        client_ready_rx
            .await
            .expect("クライアント接続完了通知を受信できること");
        session
            .close(42, "server bye")
            .await
            .expect("サーバー側の close に成功すること");
        session
    });

    // クライアント接続
    let mut client_session = WtClient::connect(build_client_config(server_addr, ca_cert_pem), "/")
        .await
        .expect("クライアント接続に成功すること");
    // 受信タスクが起動しイベント受信可能な状態でサーバーの close を促す
    client_ready_tx
        .send(())
        .expect("サーバー側にクライアント準備完了を通知できること");

    // クライアント側で SessionClosed イベントを待つ
    let event = tokio::time::timeout(Duration::from_secs(5), client_session.recv_event())
        .await
        .expect("イベント受信のタイムアウト待ちが完了すること")
        .expect("SessionClosed イベントが届くこと");

    match event {
        WebTransportEvent::SessionClosed {
            close_error_code,
            close_message,
            ..
        } => {
            assert_eq!(
                close_error_code, 42,
                "サーバーが送信した close_error_code (42) と一致すること"
            );
            assert_eq!(
                close_message, "server bye",
                "サーバーが送信した close_message と一致すること"
            );
        }
        other => panic!("SessionClosed 以外のイベントが届いた: {other:?}"),
    }

    // 以降 recv_event は None を返す (受信タスクが終了している)
    let next = tokio::time::timeout(Duration::from_secs(2), client_session.recv_event())
        .await
        .expect("None 受信のタイムアウト待ちが完了すること");
    assert!(
        next.is_none(),
        "SessionClosed の後は recv_event が None を返すこと: {next:?}"
    );

    let _server_session = server_task
        .await
        .expect("サーバータスクの終了に成功すること");
}

/// クライアント側で `WtSession::close` を呼ぶと、サーバー側の `recv_event()` で
/// `SessionClosed { close_error_code, close_message, .. }` が届くことを検証する
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn client_close_delivers_session_closed_to_server() {
    let (mut server, server_addr, ca_cert_pem) = start_server().await;

    // サーバー側のセッション確立が完了したことをクライアント側に通知するチャネル
    let (server_ready_tx, server_ready_rx) = oneshot::channel::<()>();

    let client_task = tokio::spawn(async move {
        let mut client_session =
            WtClient::connect(build_client_config(server_addr, ca_cert_pem), "/")
                .await
                .expect("クライアント接続に成功すること");
        // サーバー側の受信タスクが起動する前に close を送るとレースになるため
        // 明示的にサーバー準備完了を待つ
        server_ready_rx
            .await
            .expect("サーバー準備完了通知を受信できること");
        client_session
            .close(7, "client bye")
            .await
            .expect("クライアント側の close に成功すること");
        client_session
    });

    // サーバー側でセッション確立
    let request = server
        .accept()
        .await
        .expect("サーバー側の accept に成功すること");
    let mut server_session = request
        .accept()
        .await
        .expect("セッション確立に成功すること");
    // 受信タスクが起動しイベント受信可能な状態でクライアントの close を促す
    server_ready_tx
        .send(())
        .expect("クライアント側にサーバー準備完了を通知できること");

    let event = tokio::time::timeout(Duration::from_secs(5), server_session.recv_event())
        .await
        .expect("イベント受信のタイムアウト待ちが完了すること")
        .expect("SessionClosed イベントが届くこと");

    match event {
        WebTransportEvent::SessionClosed {
            close_error_code,
            close_message,
            ..
        } => {
            assert_eq!(
                close_error_code, 7,
                "クライアントが送信した close_error_code (7) と一致すること"
            );
            assert_eq!(
                close_message, "client bye",
                "クライアントが送信した close_message と一致すること"
            );
        }
        other => panic!("SessionClosed 以外のイベントが届いた: {other:?}"),
    }

    let _client_session = client_task
        .await
        .expect("クライアントタスクの終了に成功すること");
}

/// クライアント側で `WtSession` を drop するとサーバー側の CONNECT ストリームには
/// FIN が届き (WT_CLOSE_SESSION なし)、サーバー側の `recv_event()` で
/// `SessionClosed { close_error_code: 0, close_message: "" }` 相当が届くことを検証する
///
/// (draft-ietf-webtrans-http3-16 Section 6: "Cleanly terminating a CONNECT stream
/// without a WT_CLOSE_SESSION capsule SHALL be semantically equivalent to
/// terminating it with a WT_CLOSE_SESSION capsule that has an error code of 0
/// and an empty error string.")
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn client_drop_delivers_clean_close_to_server() {
    let (mut server, server_addr, ca_cert_pem) = start_server().await;

    // サーバー側のセッション確立完了通知
    let (server_ready_tx, server_ready_rx) = oneshot::channel::<()>();

    let client_task = tokio::spawn(async move {
        let client_session = WtClient::connect(build_client_config(server_addr, ca_cert_pem), "/")
            .await
            .expect("クライアント接続に成功すること");
        server_ready_rx
            .await
            .expect("サーバー準備完了通知を受信できること");
        // WtSession を drop する (CONNECT ストリームの送信端が閉じられ FIN が送出される)
        drop(client_session);
    });

    let request = server
        .accept()
        .await
        .expect("サーバー側の accept に成功すること");
    let mut server_session = request
        .accept()
        .await
        .expect("セッション確立に成功すること");
    server_ready_tx
        .send(())
        .expect("クライアント側にサーバー準備完了を通知できること");

    let event = tokio::time::timeout(Duration::from_secs(5), server_session.recv_event())
        .await
        .expect("イベント受信のタイムアウト待ちが完了すること")
        .expect("SessionClosed イベントが届くこと");

    match event {
        WebTransportEvent::SessionClosed {
            close_error_code,
            close_message,
            ..
        } => {
            // FIN のみの終了は WT_CLOSE_SESSION(error_code=0, message="") と等価
            assert_eq!(
                close_error_code, 0,
                "FIN のみのクリーンクローズは close_error_code=0 になること"
            );
            assert!(
                close_message.is_empty(),
                "FIN のみのクリーンクローズは close_message が空になること: {close_message:?}"
            );
        }
        other => panic!("SessionClosed 以外のイベントが届いた: {other:?}"),
    }

    client_task
        .await
        .expect("クライアントタスクの終了に成功すること");
}

/// サーバー側で `WtSession::close` を呼び出すと、送信側 (サーバー) の `recv_event()` でも
/// 受信側 (クライアント) の close 応答 (FIN) による `SessionClosed` が届くことを検証する
///
/// (draft-ietf-webtrans-http3-16 Section 6: WT_CLOSE_SESSION の受信側は close か reset を返す)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_close_response_is_observed_by_server() {
    let (mut server, server_addr, ca_cert_pem) = start_server().await;

    let (client_ready_tx, client_ready_rx) = oneshot::channel::<()>();

    let server_task = tokio::spawn(async move {
        let request = server
            .accept()
            .await
            .expect("サーバー側の accept に成功すること");
        let mut session = request
            .accept()
            .await
            .expect("セッション確立に成功すること");
        client_ready_rx
            .await
            .expect("クライアント接続完了通知を受信できること");
        session
            .close(42, "server bye")
            .await
            .expect("サーバー側の close に成功すること");
        // 受信側 (クライアント) の close 応答 (FIN) を送信側で観測する
        let event = tokio::time::timeout(Duration::from_secs(5), session.recv_event())
            .await
            .expect("close 応答のタイムアウト待ちが完了すること")
            .expect("close 応答の SessionClosed が届くこと");
        (session, event)
    });

    let mut client_session = WtClient::connect(build_client_config(server_addr, ca_cert_pem), "/")
        .await
        .expect("クライアント接続に成功すること");
    client_ready_tx
        .send(())
        .expect("サーバー側にクライアント準備完了を通知できること");

    // クライアント側でも WT_CLOSE_SESSION 受信による SessionClosed を観測する
    let client_event = tokio::time::timeout(Duration::from_secs(5), client_session.recv_event())
        .await
        .expect("イベント受信のタイムアウト待ちが完了すること")
        .expect("SessionClosed イベントが届くこと");
    assert!(
        matches!(client_event, WebTransportEvent::SessionClosed { .. }),
        "クライアント側で SessionClosed が届くこと: {client_event:?}"
    );

    let (_server_session, event) = server_task
        .await
        .expect("サーバータスクの終了に成功すること");
    match event {
        WebTransportEvent::SessionClosed {
            close_error_code,
            close_message,
            ..
        } => {
            // 受信側の close 応答 (FIN) は error_code=0 / message="" と等価
            assert_eq!(
                close_error_code, 0,
                "close 応答 (FIN) の close_error_code は 0 になること"
            );
            assert!(
                close_message.is_empty(),
                "close 応答 (FIN) の close_message は空になること: {close_message:?}"
            );
        }
        other => panic!("SessionClosed 以外のイベントが届いた: {other:?}"),
    }
}

/// クライアント側で `WtSession::close` を呼び出すと、送信側 (クライアント) の
/// `recv_event()` でも受信側 (サーバー) の close 応答 (FIN) による `SessionClosed` が
/// 届くことを検証する (サーバー発のケースと対称)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn client_close_response_is_observed_by_client() {
    let (mut server, server_addr, ca_cert_pem) = start_server().await;

    let (server_ready_tx, server_ready_rx) = oneshot::channel::<()>();

    let client_task = tokio::spawn(async move {
        let mut client_session =
            WtClient::connect(build_client_config(server_addr, ca_cert_pem), "/")
                .await
                .expect("クライアント接続に成功すること");
        server_ready_rx
            .await
            .expect("サーバー準備完了通知を受信できること");
        client_session
            .close(7, "client bye")
            .await
            .expect("クライアント側の close に成功すること");
        // 受信側 (サーバー) の close 応答 (FIN) を送信側で観測する
        let event = tokio::time::timeout(Duration::from_secs(5), client_session.recv_event())
            .await
            .expect("close 応答のタイムアウト待ちが完了すること")
            .expect("close 応答の SessionClosed が届くこと");
        (client_session, event)
    });

    let request = server
        .accept()
        .await
        .expect("サーバー側の accept に成功すること");
    let mut server_session = request
        .accept()
        .await
        .expect("セッション確立に成功すること");
    server_ready_tx
        .send(())
        .expect("クライアント側にサーバー準備完了を通知できること");

    // サーバー側でも WT_CLOSE_SESSION 受信による SessionClosed を観測する
    let server_event = tokio::time::timeout(Duration::from_secs(5), server_session.recv_event())
        .await
        .expect("イベント受信のタイムアウト待ちが完了すること")
        .expect("SessionClosed イベントが届くこと");
    assert!(
        matches!(server_event, WebTransportEvent::SessionClosed { .. }),
        "サーバー側で SessionClosed が届くこと: {server_event:?}"
    );

    let (_client_session, event) = client_task
        .await
        .expect("クライアントタスクの終了に成功すること");
    match event {
        WebTransportEvent::SessionClosed {
            close_error_code,
            close_message,
            ..
        } => {
            assert_eq!(
                close_error_code, 0,
                "close 応答 (FIN) の close_error_code は 0 になること"
            );
            assert!(
                close_message.is_empty(),
                "close 応答 (FIN) の close_message は空になること: {close_message:?}"
            );
        }
        other => panic!("SessionClosed 以外のイベントが届いた: {other:?}"),
    }
}
