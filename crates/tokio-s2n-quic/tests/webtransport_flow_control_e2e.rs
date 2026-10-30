//! WebTransport 送信側フロー制御の実 QUIC 統合テスト
//!
//! ループバック接続でサーバー・クライアントを立ち上げ、以下を確認する
//! (draft-ietf-webtrans-http3-16 Section 5.1, 5.5, 5.6.2, 5.6.4):
//! - ピア SETTINGS の WT 初期上限がストリーム開設・データ送信の上限として効くこと
//! - WT_MAX_STREAMS カプセルで上限が更新されること
//! - 増加しない WT_MAX_STREAMS カプセルで WT_FLOW_CONTROL_ERROR のセッション終了になること
//!
//! モック・スタブは使用しない (実 QUIC 接続を利用する)。

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use bytes::Bytes;
use shiguredo_http3::webtransport::capsule::Capsule;
use shiguredo_http3::{VarInt, webtransport};
use tokio::sync::oneshot;
use tokio_s2n_quic::{
    ClientConfig, Error, FlowControlResource, ServerConfig, WebTransportEvent, WtClient, WtServer,
};

// テスト間で共有するヘルパーは tests/helpers/ に置き、必要なファイルだけを
// 明示的に取り込む (モジュール全体を取り込むと未使用部分が dead code になるため)
#[path = "helpers/certs.rs"]
mod certs;
// 本テストでは `RawWtClient` の一部のメソッドしか使わないため、
// このテストバイナリに限ってヘルパー内の未使用項目の dead_code 警告を抑止する
#[expect(dead_code)]
#[path = "helpers/wt_raw_client.rs"]
mod wt_raw_client;

use certs::generate_certificate;
use wt_raw_client::{RawWtClient, build_wt_settings};

/// テスト用: WT_FLOW_CONTROL_ERROR のコード
/// (draft-ietf-webtrans-http3-16 Section 9)
const WT_FLOW_CONTROL_ERROR: u64 = 0x045d4487;

/// テスト用: ループバックのリッスンアドレスを作る
fn listen_addr() -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0)
}

/// テスト用: 値が VarInt 範囲内であることを確認して返す
fn v(value: u64) -> VarInt {
    VarInt::new(value).expect("テスト用の値が VarInt 範囲内であること")
}

/// サーバー側: ピア SETTINGS の初期上限がストリーム開設とデータ送信に効くこと
///
/// サーバーは双方向ストリーム上限のみ 1 を広告する (単方向上限・データ上限は 0)。
/// クライアントは SETTINGS の値を送信側上限として扱い、超過操作を拒否すること。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn peer_settings_limits_are_enforced_on_stream_open() {
    let (cert_pem, key_pem) = generate_certificate();

    // サーバーは双方向ストリーム上限 1 のみ広告する
    let server_wt_settings = webtransport::Settings::new()
        .wt_enabled(VarInt::from_static(1))
        .wt_initial_max_streams_bidi(v(1));
    let config = ServerConfig::new(listen_addr(), &cert_pem, key_pem)
        .enable_webtransport(server_wt_settings);
    let mut server = WtServer::bind(config).expect("サーバー bind に成功すること");
    let addr = server.local_addr();

    // クライアント側の検証が終わるまでセッションを保持する
    let (done_tx, done_rx) = oneshot::channel::<()>();
    let server_task = tokio::spawn(async move {
        let request = server
            .accept()
            .await
            .expect("サーバー側の accept に成功すること");
        let session = request
            .accept()
            .await
            .expect("セッション確立に成功すること");
        let _ = done_rx.await;
        drop(session);
    });

    // クライアントはフロー制御を宣言する (初期上限は非 0)
    let client_wt_settings = build_wt_settings();
    let config = ClientConfig::new(addr, "localhost")
        .ca_cert(cert_pem)
        .enable_webtransport(client_wt_settings);
    let mut session = WtClient::connect(config, "/")
        .await
        .expect("クライアント接続に成功すること");

    // ピア SETTINGS の双方向上限 1 までは開設できる
    let mut first = session
        .open_bi_stream()
        .await
        .expect("上限内の双方向ストリーム開設に成功すること");

    // 上限を超える双方向ストリーム開設は拒否される
    match session.open_bi_stream().await {
        Ok(_) => panic!("上限を超える双方向ストリーム開設は拒否されること"),
        Err(Error::FlowControlBlocked {
            resource: FlowControlResource::BidirectionalStream,
        }) => {}
        Err(e) => panic!("双方向ストリーム上限超過で FlowControlBlocked になること: {e:?}"),
    }

    // ピア SETTINGS の単方向上限 0 のため単方向ストリームは開設できない
    match session.open_uni_stream().await {
        Ok(_) => panic!("単方向ストリーム開設は拒否されること"),
        Err(Error::FlowControlBlocked {
            resource: FlowControlResource::UnidirectionalStream,
        }) => {}
        Err(e) => panic!("単方向ストリーム上限超過で FlowControlBlocked になること: {e:?}"),
    }

    // ピア SETTINGS のデータ上限 0 のためデータ送信はできない
    let send_result = first.send(b"hello").await;
    assert!(
        matches!(
            send_result,
            Err(Error::FlowControlBlocked {
                resource: FlowControlResource::Data
            })
        ),
        "データ上限超過で FlowControlBlocked になること: {send_result:?}"
    );

    let _ = done_tx.send(());
    server_task.await.expect("サーバータスクが正常終了すること");
}

/// サーバー側: WT_MAX_STREAMS カプセルで上限が更新され、
/// 増加しないカプセルで WT_FLOW_CONTROL_ERROR のセッション終了になること
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn flow_control_capsules_update_limits_and_non_increasing_closes_session() {
    let (cert_pem, key_pem) = generate_certificate();

    // サーバーの広告値は本テストでは使わない (クライアントの SETTINGS が
    // サーバーの送信側上限になる)
    let config = ServerConfig::new(listen_addr(), &cert_pem, key_pem)
        .enable_webtransport(build_wt_settings());
    let mut server = WtServer::bind(config).expect("サーバー bind に成功すること");
    let addr = server.local_addr();

    // クライアントが広告する WT 初期上限: 双方向 1 / 単方向 1 / データ 1024
    let client_wt_settings = webtransport::Settings::new()
        .wt_enabled(VarInt::from_static(1))
        .wt_initial_max_streams_bidi(v(1))
        .wt_initial_max_streams_uni(v(1))
        .wt_initial_max_data(v(1024));

    // サーバータスクからクライアントへ、カプセル送出のタイミングを指示する
    let (first_capsule_tx, first_capsule_rx) = oneshot::channel::<()>();
    let (second_capsule_tx, second_capsule_rx) = oneshot::channel::<()>();

    let server_task = tokio::spawn(async move {
        let request = server
            .accept()
            .await
            .expect("サーバー側の accept に成功すること");
        let mut session = request
            .accept()
            .await
            .expect("セッション確立に成功すること");

        // SETTINGS の双方向上限 1 までは開設できる
        let first = session
            .open_bi_stream()
            .await
            .expect("上限内の双方向ストリーム開設に成功すること");
        drop(first);

        // 上限を超える開設は拒否される
        match session.open_bi_stream().await {
            Ok(_) => panic!("上限を超える双方向ストリーム開設は拒否されること"),
            Err(Error::FlowControlBlocked { .. }) => {}
            Err(e) => panic!("双方向ストリーム上限超過で FlowControlBlocked になること: {e:?}"),
        }

        // 最初の WT_MAX_STREAMS (1 → 2) をクライアントへ送らせる
        first_capsule_tx
            .send(())
            .expect("最初のカプセル送出指示を送信できること");

        // 上限更新が反映されるまでストリーム開設を再試行する
        // (CONNECT ストリームの受信タスクがカプセルを処理するまで待つ)
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            match session.open_bi_stream().await {
                Ok(stream) => {
                    drop(stream);
                    break;
                }
                Err(Error::FlowControlBlocked { .. }) => {
                    assert!(
                        tokio::time::Instant::now() < deadline,
                        "WT_MAX_STREAMS カプセルによる上限更新が反映されること"
                    );
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Err(e) => panic!("ストリーム開設で予期しないエラー: {e:?}"),
            }
        }

        // 増加しない WT_MAX_STREAMS (2 → 2) をクライアントへ送らせる
        second_capsule_tx
            .send(())
            .expect("2 番目のカプセル送出指示を送信できること");

        // 単調性違反により WT_FLOW_CONTROL_ERROR でセッションが終了すること
        let event = tokio::time::timeout(Duration::from_secs(5), session.recv_event())
            .await
            .expect("セッション終了イベントが届くこと")
            .expect("受信タスクが終了していないこと");
        match event {
            WebTransportEvent::SessionClosed {
                error_code,
                close_error_code,
                ..
            } => {
                assert_eq!(
                    error_code, WT_FLOW_CONTROL_ERROR,
                    "RESET_STREAM 用のエラーコードが WT_FLOW_CONTROL_ERROR であること"
                );
                assert_eq!(
                    close_error_code, WT_FLOW_CONTROL_ERROR as u32,
                    "WT_CLOSE_SESSION のエラーコードが WT_FLOW_CONTROL_ERROR であること"
                );
            }
            other => panic!("SessionClosed 以外のイベントが届いた: {other:?}"),
        }
    });

    // 生の s2n-quic クライアントで CONNECT ストリームへカプセルを直接送出する
    let mut raw = RawWtClient::connect_with_wt_settings(addr, &cert_pem, client_wt_settings).await;

    // 1 回目: WT_MAX_STREAMS の双方向上限を 1 → 2 へ増加させる
    first_capsule_rx
        .await
        .expect("最初のカプセル送出指示を受信できること");
    let mut buf = Vec::new();
    Capsule::MaxStreams {
        bidirectional: true,
        maximum: 2,
    }
    .encode_as_data_frame(&mut buf);
    raw.send
        .send(Bytes::from(buf))
        .await
        .expect("WT_MAX_STREAMS の送信に成功すること");

    // 2 回目: 増加しない WT_MAX_STREAMS (2 → 2) を送る
    second_capsule_rx
        .await
        .expect("2 番目のカプセル送出指示を受信できること");
    let mut buf = Vec::new();
    Capsule::MaxStreams {
        bidirectional: true,
        maximum: 2,
    }
    .encode_as_data_frame(&mut buf);
    raw.send
        .send(Bytes::from(buf))
        .await
        .expect("増加しない WT_MAX_STREAMS の送信に成功すること");

    server_task.await.expect("サーバータスクが正常終了すること");
}
