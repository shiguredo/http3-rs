//! WebTransport フロー制御カプセルの統合 E2E テスト
//!
//! `WtServer` と `WtClient` を実 QUIC 接続で繋ぎ、セッション確立直後に
//! サーバーが送出する初期フロー制御カプセル (`WT_MAX_STREAMS` / `WT_MAX_DATA`) を
//! クライアントが受信できることを検証する。
//!
//! Safari 26.4 は draft-07 の SETTINGS で接続しつつ draft-14 のカプセルベース
//! フロー制御を使うため、セッション確立後にこれらのカプセルを受信できないと
//! 初期クレジット 0 のままストリームを開けず、通信が始まらない
//! (docs/SAFARI_WT.md / draft-ietf-webtrans-http3-14 Section 5)。

use std::time::Duration;

use rcgen::generate_simple_self_signed;
use shiguredo_http3::VarInt;
use shiguredo_http3::WebTransportEvent;
use shiguredo_http3::webtransport::{Capsule, DraftVersion, Settings as WtSettings};
use tokio_s2n_quic::{ClientConfig, ServerConfig, WtClient, WtServer};

fn vi(v: u64) -> VarInt {
    VarInt::new(v).expect("テスト用の値は VarInt 範囲内")
}

/// Safari 26.4 形の SETTINGS を作成する
///
/// draft-07 の `SETTINGS_WEBTRANSPORT_MAX_SESSIONS` と draft-13/14 の
/// `WT_INITIAL_MAX_*` を併送するハイブリッド実装を再現する。
fn safari_shape_settings() -> WtSettings {
    WtSettings::new()
        .webtransport_max_sessions_draft07(vi(100))
        .wt_initial_max_streams_uni(vi(100))
        .wt_initial_max_streams_bidi(vi(100))
        .wt_initial_max_data(vi(8 * 1024 * 1024))
}

/// サーバーを起動し、アドレスとサーバータスクを返す
async fn start_server(
    settings: WtSettings,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>, String) {
    let names = vec!["localhost".to_string(), "127.0.0.1".to_string()];
    let certified = generate_simple_self_signed(names).expect("テスト用証明書の生成は成功する");
    let cert_pem = certified.cert.pem();
    let key_pem = certified.signing_key.serialize_pem();

    let config = ServerConfig::new(
        "127.0.0.1:0".parse().expect("テスト用アドレスは有効"),
        &cert_pem,
        &key_pem,
    )
    .enable_webtransport(settings);
    let mut server = WtServer::bind(config).expect("WtServer の bind は成功する");
    let addr = server.local_addr();

    let task = tokio::spawn(async move {
        if let Ok(request) = server.accept().await {
            // セッションを維持する (カプセル送出はセッション確立時に行われる)
            if let Ok(_session) = request.accept().await {
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    });

    (addr, task, cert_pem)
}

/// サーバーから届く WebTransport カプセルを収集する
async fn collect_capsules(
    session: &mut tokio_s2n_quic::WtSession,
    want: usize,
    timeout: Duration,
) -> Vec<Capsule> {
    let mut received = Vec::new();
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline && received.len() < want {
        match tokio::time::timeout(Duration::from_millis(500), session.recv_event()).await {
            Ok(Some(WebTransportEvent::Capsule { capsule, .. })) => received.push(capsule),
            Ok(Some(_)) => {}
            Ok(None) => break,
            Err(_) => {}
        }
    }
    received
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_server_sends_initial_flow_control_capsules() {
    // Safari 形クライアントでセッションを確立し、初期クレジットのカプセルを
    // 3 件 (WT_MAX_STREAMS bidi / uni, WT_MAX_DATA) 受信できること
    let settings = safari_shape_settings();
    let (addr, server_task, cert_pem) = start_server(settings).await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    let config = ClientConfig::new(addr, "localhost")
        .ca_cert(&cert_pem)
        .enable_webtransport(settings)
        .wt_draft_version(DraftVersion::Draft07);
    let mut session = WtClient::connect(config, "/wt")
        .await
        .expect("WebTransport セッションの確立は成功する");

    assert!(
        session.flow_control_enabled(),
        "Safari 形クライアントでフロー制御が有効にならない"
    );

    let received = collect_capsules(&mut session, 3, Duration::from_secs(3)).await;
    server_task.abort();

    assert!(
        received.iter().any(|c| matches!(
            c,
            Capsule::MaxStreams {
                bidirectional: true,
                maximum: 100
            }
        )),
        "WT_MAX_STREAMS (bidi) を受信していない: {received:?}"
    );
    assert!(
        received.iter().any(|c| matches!(
            c,
            Capsule::MaxStreams {
                bidirectional: false,
                maximum: 100
            }
        )),
        "WT_MAX_STREAMS (uni) を受信していない: {received:?}"
    );
    assert!(
        received
            .iter()
            .any(|c| matches!(c, Capsule::MaxData { maximum } if *maximum == 8 * 1024 * 1024)),
        "WT_MAX_DATA を受信していない: {received:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_initial_capsules_do_not_close_session() {
    // 双方が同じ WT_INITIAL_MAX_* を広告する場合、初期カプセルの値は
    // SETTINGS と同値になる。これを「増加しない」と誤判定して
    // WT_FLOW_CONTROL_ERROR でセッションを閉じてはいけない
    // (draft-ietf-webtrans-http3-16 Section 5.6.2 / 5.6.4)。
    let settings = safari_shape_settings();
    let (addr, server_task, cert_pem) = start_server(settings).await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    let config = ClientConfig::new(addr, "localhost")
        .ca_cert(&cert_pem)
        .enable_webtransport(settings)
        .wt_draft_version(DraftVersion::Draft07);
    let mut session = WtClient::connect(config, "/wt")
        .await
        .expect("WebTransport セッションの確立は成功する");

    // カプセルを 3 件受信した後もセッションが生存していること
    let received = collect_capsules(&mut session, 3, Duration::from_secs(3)).await;
    assert_eq!(received.len(), 3, "初期カプセルが 3 件でない: {received:?}");

    // セッションが閉じていないことを、双方向ストリームの開設で確認する
    // (ピアの WT_MAX_STREAMS を受信済みなので開設できる)
    let open_result =
        tokio::time::timeout(Duration::from_secs(3), session.accept_bi_stream()).await;
    server_task.abort();
    // ピアがストリームを開かないためタイムアウトするが、セッションが閉じていれば
    // `accept_bi_stream` は Err で即座に返る。ここでは「即座に Err にならない」
    // ことをセッション生存の証拠とする。
    assert!(
        open_result.is_err(),
        "セッションが閉じている (即座に Err が返った)"
    );
}
