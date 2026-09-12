//! WT_CLOSE_SESSION 受信後の追加 DATA に対する H3_MESSAGE_ERROR reset の実 QUIC 統合テスト
//!
//! 生の s2n-quic クライアントで CONNECT ストリームを直接操作し、WT_CLOSE_SESSION
//! 送信後に追加 DATA を送ると、受信側が RESET_STREAM(H3_MESSAGE_ERROR) を返すことを
//! 確認する。2xx 応答前に追加 DATA を送る楽観的カプセル送信経路も検証する
//! (draft-ietf-webtrans-http3-16 Section 6)。
//!
//! モック・スタブは使用しない (実 QUIC 接続を利用する)。

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use bytes::Bytes;
use shiguredo_http3::WebTransportEvent;
use shiguredo_http3::webtransport::capsule::Capsule;
use tokio_s2n_quic::{ServerConfig, WtServer};

// テスト間で共有するヘルパーは tests/helpers/ に置き、必要なファイルだけを
// 明示的に取り込む (モジュール全体を取り込むと未使用部分が dead code になるため)
#[path = "helpers/certs.rs"]
mod certs;
#[path = "helpers/wt_raw_client.rs"]
mod wt_raw_client;

use certs::generate_certificate;
use wt_raw_client::{RawWtClient, build_wt_settings};

/// サーバーを起動し、リッスンアドレスとサーバー証明書 PEM を返す
async fn start_server() -> (WtServer, SocketAddr, String) {
    let (cert_pem, key_pem) = generate_certificate();
    let listen_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0);
    let config =
        ServerConfig::new(listen_addr, &cert_pem, key_pem).enable_webtransport(build_wt_settings());
    let server = WtServer::bind(config).expect("サーバー bind に成功すること");
    let addr = server.local_addr();
    (server, addr, cert_pem)
}

/// WT_CLOSE_SESSION カプセルを H3 DATA フレームとしてエンコードする
fn close_session_data_frame() -> Vec<u8> {
    let mut buf = Vec::new();
    Capsule::CloseSession {
        error_code: 0,
        message: String::new(),
    }
    .encode_as_data_frame(&mut buf)
    .expect("テスト用カプセルのエンコードは成功する");
    buf
}

/// WT_CLOSE_SESSION カプセルに同一 DATA フレーム内の後続バイトを付けてエンコードする
fn close_session_data_frame_with_trailing(trailing: &[u8]) -> Vec<u8> {
    let mut capsule = Vec::new();
    Capsule::CloseSession {
        error_code: 0,
        message: String::new(),
    }
    .encode(&mut capsule)
    .expect("テスト用カプセルのエンコードは成功する");
    let mut data = vec![0x00, (capsule.len() + trailing.len()) as u8];
    data.extend_from_slice(&capsule);
    data.extend_from_slice(trailing);
    data
}

/// 追加データを表す H3 DATA フレーム (type=0x00, length=1) を返す
fn additional_data_frame() -> Vec<u8> {
    vec![0x00, 0x01, 0xAA]
}

/// 受信した RESET_STREAM が H3_MESSAGE_ERROR (0x10e) であることを検証する
fn assert_message_error_reset(err: &s2n_quic::stream::Error) {
    match err {
        s2n_quic::stream::Error::StreamReset { error, .. } => {
            assert_eq!(
                **error, 0x10e,
                "H3_MESSAGE_ERROR (0x10e) で reset されること"
            );
        }
        other => panic!("StreamReset を期待したが別のエラーが返った: {other}"),
    }
}

/// CONNECT ストリームの受信で RESET_STREAM を観測する (200 レスポンスは読み飛ばす)
async fn observe_reset(client: &mut RawWtClient) -> s2n_quic::stream::Error {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match client.recv.receive().await {
                Ok(Some(_)) => continue,
                Ok(None) => panic!("FIN ではなく RESET_STREAM を期待した"),
                Err(e) => break e,
            }
        }
    })
    .await
    .expect("RESET_STREAM のタイムアウト待ちが完了すること")
}

/// WT_CLOSE_SESSION と追加 DATA フレームを同一 write にまとめて送る
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn same_write_additional_data_triggers_message_error_reset() {
    let (mut server, server_addr, cert_pem) = start_server().await;

    let server_task = tokio::spawn(async move {
        let request = server.accept().await.expect("accept に成功すること");
        let mut session = request
            .accept()
            .await
            .expect("セッション確立に成功すること");
        let event = tokio::time::timeout(Duration::from_secs(5), session.recv_event())
            .await
            .expect("SessionClosed のタイムアウト待ちが完了すること")
            .expect("SessionClosed が届くこと");
        // 終端イベントは二重配送されない
        let next = tokio::time::timeout(Duration::from_secs(2), session.recv_event())
            .await
            .expect("None 受信のタイムアウト待ちが完了すること");
        assert!(
            next.is_none(),
            "SessionClosed の後は recv_event が None を返すこと"
        );
        (session, event)
    });

    let mut client = RawWtClient::connect(server_addr, &cert_pem).await;

    let mut buf = close_session_data_frame();
    buf.extend_from_slice(&additional_data_frame());
    client
        .send
        .send(Bytes::from(buf))
        .await
        .expect("カプセルと追加データの送信に成功すること");

    let err = observe_reset(&mut client).await;
    assert_message_error_reset(&err);

    let (_session, event) = server_task
        .await
        .expect("サーバータスクの終了に成功すること");
    assert!(
        matches!(event, WebTransportEvent::SessionClosed { .. }),
        "サーバー側で SessionClosed が届くこと: {event:?}"
    );
}

/// 同一 DATA フレーム内で WT_CLOSE_SESSION に続く追加バイトで RESET_STREAM される
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn same_data_frame_trailing_triggers_message_error_reset() {
    let (mut server, server_addr, cert_pem) = start_server().await;

    let server_task = tokio::spawn(async move {
        let request = server.accept().await.expect("accept に成功すること");
        let mut session = request
            .accept()
            .await
            .expect("セッション確立に成功すること");
        let event = tokio::time::timeout(Duration::from_secs(5), session.recv_event())
            .await
            .expect("SessionClosed のタイムアウト待ちが完了すること")
            .expect("SessionClosed が届くこと");
        (session, event)
    });

    let mut client = RawWtClient::connect(server_addr, &cert_pem).await;

    client
        .send
        .send(Bytes::from(close_session_data_frame_with_trailing(&[0xAA])))
        .await
        .expect("カプセルと後続バイトの送信に成功すること");

    let err = observe_reset(&mut client).await;
    assert_message_error_reset(&err);

    let (_session, event) = server_task
        .await
        .expect("サーバータスクの終了に成功すること");
    assert!(
        matches!(event, WebTransportEvent::SessionClosed { .. }),
        "サーバー側で SessionClosed が届くこと: {event:?}"
    );
}

/// WT_CLOSE_SESSION 送信後に別 write で追加 DATA を送る
///
/// サーバーが SessionClosed を観測したことを通知してから追加 DATA を送ることで、
/// WT_CLOSE_SESSION と追加 DATA が別 receive チャンクで届くことを確定させる。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn separate_write_additional_data_triggers_message_error_reset() {
    let (mut server, server_addr, cert_pem) = start_server().await;

    let (closed_tx, closed_rx) = tokio::sync::oneshot::channel::<()>();
    let server_task = tokio::spawn(async move {
        let request = server.accept().await.expect("accept に成功すること");
        let mut session = request
            .accept()
            .await
            .expect("セッション確立に成功すること");
        let event = tokio::time::timeout(Duration::from_secs(5), session.recv_event())
            .await
            .expect("SessionClosed のタイムアウト待ちが完了すること")
            .expect("SessionClosed が届くこと");
        // クライアントへ SessionClosed 観測を通知し、追加 DATA を送らせる
        let _ = closed_tx.send(());
        // 終端イベントは二重配送されない
        let next = tokio::time::timeout(Duration::from_secs(5), session.recv_event())
            .await
            .expect("None 受信のタイムアウト待ちが完了すること");
        assert!(
            next.is_none(),
            "SessionClosed の後は recv_event が None を返すこと"
        );
        (session, event)
    });

    let mut client = RawWtClient::connect(server_addr, &cert_pem).await;

    // まず WT_CLOSE_SESSION のみを送る
    client
        .send
        .send(Bytes::from(close_session_data_frame()))
        .await
        .expect("カプセルの送信に成功すること");
    // サーバーが SessionClosed を観測するまで待つ (別 receive チャンクを確定させる)
    closed_rx
        .await
        .expect("SessionClosed 観測の通知を受信できること");
    // 別 write で追加 DATA を送る
    client
        .send
        .send(Bytes::from(additional_data_frame()))
        .await
        .expect("追加データの送信に成功すること");

    let err = observe_reset(&mut client).await;
    assert_message_error_reset(&err);

    let (_session, event) = server_task
        .await
        .expect("サーバータスクの終了に成功すること");
    assert!(
        matches!(event, WebTransportEvent::SessionClosed { .. }),
        "サーバー側で SessionClosed が届くこと: {event:?}"
    );
}

/// 2xx 応答前に届いた WT_CLOSE_SESSION + 追加 DATA で RESET_STREAM(H3_MESSAGE_ERROR) が返る
///
/// 楽観的カプセル送信経路 (2xx 応答前に CONNECT ストリームへ DATA が届く場合) を検証する。
/// CONNECT リクエストと DATA を同一 write で送るため、通常はセッション確立時に
/// バッファリング済みカプセルの検証で H3_MESSAGE_ERROR になる。QUIC のチャンク分割次第で
/// 確立後の受信タスク経路になった場合も、どちらでも RESET_STREAM(H3_MESSAGE_ERROR) が
/// 送出される。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn optimistic_pre_response_data_triggers_message_error_reset() {
    let (mut server, server_addr, cert_pem) = start_server().await;

    let server_task = tokio::spawn(async move {
        let request = server.accept().await.expect("accept に成功すること");
        match request.accept().await {
            Ok(mut session) => {
                // 確立後の受信タスク経路になった場合は、受信タスクが reset を送出して
                // 終了する (recv_event が None になる) までセッションを保持する
                let _ = tokio::time::timeout(Duration::from_secs(5), session.recv_event()).await;
                let _ = tokio::time::timeout(Duration::from_secs(5), session.recv_event()).await;
                Ok(())
            }
            Err(e) => Err(e),
        }
    });

    let mut data = close_session_data_frame();
    data.extend_from_slice(&additional_data_frame());
    let mut client =
        RawWtClient::connect_with_optimistic_payload(server_addr, &cert_pem, &data).await;

    let err = observe_reset(&mut client).await;
    assert_message_error_reset(&err);

    let result = tokio::time::timeout(Duration::from_secs(5), server_task)
        .await
        .expect("accept のタイムアウト待ちが完了すること")
        .expect("サーバータスクの終了に成功すること");
    // 確立時のエラー (楽観的カプセル経路) または確立成功 (確立後の受信タスク経路) の
    // いずれでも、ピアへの RESET_STREAM(H3_MESSAGE_ERROR) は上で確認済み
    match result {
        Ok(()) => {}
        Err(tokio_s2n_quic::Error::Http3(shiguredo_http3::Error::StreamError(
            shiguredo_http3::ErrorCode::MessageError,
        ))) => {}
        Err(other) => panic!("楽観的カプセルの処理で予期しないエラーが返った: {other}"),
    }
}

/// 確立前に WT_CLOSE_SESSION を受け取り、確立後に追加 DATA が届いても reset される
///
/// 2xx 前に WT_CLOSE_SESSION のみがバッファリングされるケースでは、2xx 応答後の
/// 受信タスクが SessionClosed を転送したあとも FIN / Err まで読み続ける必要がある。
/// 早期に終了すると後続の追加 DATA を読めず、H3_MESSAGE_ERROR の reset
/// (draft-ietf-webtrans-http3-16 Section 6 の MUST) に到達できない。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pending_session_closed_then_additional_data_triggers_message_error_reset() {
    let (mut server, server_addr, cert_pem) = start_server().await;

    let (closed_tx, closed_rx) = tokio::sync::oneshot::channel::<()>();
    let server_task = tokio::spawn(async move {
        let request = server.accept().await.expect("accept に成功すること");
        let mut session = request
            .accept()
            .await
            .expect("セッション確立に成功すること");
        let event = tokio::time::timeout(Duration::from_secs(5), session.recv_event())
            .await
            .expect("SessionClosed のタイムアウト待ちが完了すること")
            .expect("SessionClosed が届くこと");
        // クライアントへ SessionClosed 観測を通知し、追加 DATA を送らせる
        let _ = closed_tx.send(());
        // 受信タスクが追加 DATA を reset するまでセッションを保持する
        let next = tokio::time::timeout(Duration::from_secs(5), session.recv_event())
            .await
            .expect("None 受信のタイムアウト待ちが完了すること");
        assert!(
            next.is_none(),
            "SessionClosed の後は recv_event が None を返すこと"
        );
        (session, event)
    });

    // CONNECT リクエスト + WT_CLOSE_SESSION をまとめて送る (2xx 前)
    let mut client = RawWtClient::connect_with_optimistic_payload(
        server_addr,
        &cert_pem,
        &close_session_data_frame(),
    )
    .await;

    // サーバーが SessionClosed を観測するまで待つ
    closed_rx
        .await
        .expect("SessionClosed 観測の通知を受信できること");
    // 確立後に追加 DATA を送る (受信タスクが SessionClosed 転送後も読み続けることの検証)
    client
        .send
        .send(Bytes::from(additional_data_frame()))
        .await
        .expect("追加データの送信に成功すること");

    let err = observe_reset(&mut client).await;
    assert_message_error_reset(&err);

    let (_session, event) = tokio::time::timeout(Duration::from_secs(5), server_task)
        .await
        .expect("サーバータスクのタイムアウト待ちが完了すること")
        .expect("サーバータスクの終了に成功すること");
    assert!(
        matches!(event, WebTransportEvent::SessionClosed { .. }),
        "サーバー側で SessionClosed が届くこと: {event:?}"
    );
}
