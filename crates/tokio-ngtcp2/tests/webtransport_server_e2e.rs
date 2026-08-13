//! ServerWebTransportSession の堅牢性テスト
//!
//! 不正パケットで WebTransport サーバーが停止しないことと、
//! コネクション ID 指定の公開 API 群が動作することを検証する。
//!
//! `recv_once` は 1 回の I/O を処理するため、テスト側でサーバーを
//! 手動駆動しながら内部状態を検証できる。

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use shiguredo_ngtcp2::Http3Event;
use tokio::time::timeout;
use tokio_ngtcp2::{ClientWebTransportSession, ServerWebTransportSession};

/// テスト用の証明書と秘密鍵を動的に生成する
fn generate_test_certs() -> (PathBuf, PathBuf) {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let unique_id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let temp_dir = std::env::temp_dir().join(format!(
        "webtransport_server_e2e_test_{}_{}",
        std::process::id(),
        unique_id
    ));
    std::fs::create_dir_all(&temp_dir).expect("test must succeed");

    let cert_path = temp_dir.join("cert.pem");
    let key_path = temp_dir.join("key.pem");

    // 証明書パラメータを設定
    let params =
        rcgen::CertificateParams::new(vec!["localhost".to_string()]).expect("test must succeed");

    // 鍵ペアを生成して自己署名証明書を作成
    let key_pair = rcgen::KeyPair::generate().expect("test must succeed");
    let cert = params.self_signed(&key_pair).expect("test must succeed");

    // PEM 形式で保存
    std::fs::write(&cert_path, cert.pem()).expect("test must succeed");
    std::fs::write(&key_path, key_pair.serialize_pem()).expect("test must succeed");

    (cert_path, key_path)
}

/// 不正なパケットを送りつけても WebTransport サーバーが継続することを確認する
///
/// 不正パケットの後に WebTransport セッションを確立し、コネクション ID 指定の
/// 公開 API (`get_established_conn_ids` / `open_bidi_stream_by_conn_id` /
/// `send_stream_data_by_conn_id`) でデータを送信できることを検証する。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_webtransport_server_survives_malformed_packets() {
    let (cert_path, key_path) = generate_test_certs();

    let mut server = ServerWebTransportSession::bind(
        "127.0.0.1:0".parse().expect("test must succeed"),
        &cert_path,
        &key_path,
    )
    .await
    .expect("サーバー作成失敗");

    let server_addr = server.local_addr();

    // 不正パケットを送りつける (サーバー起動直後・稼働中の両方で耐えること)
    let attacker = std::net::UdpSocket::bind("127.0.0.1:0").expect("test must succeed");

    // 1. 完全なガベージデータ
    attacker
        .send_to(&[0u8; 64], server_addr)
        .expect("test must succeed");

    // 2. DCID が未登録でペイロードが破損した Initial
    let mut initial = vec![0xC0, 0, 0, 0, 1, 16];
    initial.extend_from_slice(&[0xAB; 16]);
    initial.push(8);
    initial.extend_from_slice(&[0xCD; 8]);
    initial.extend_from_slice(&[0xFF; 1200]);
    attacker
        .send_to(&initial, server_addr)
        .expect("test must succeed");

    // 3. Short header で DCID が未登録のパケット
    let mut short = vec![0x40];
    short.extend_from_slice(&[0xEE; 16]);
    short.extend_from_slice(&[0xFF; 32]);
    attacker
        .send_to(&short, server_addr)
        .expect("test must succeed");

    // クライアントタスク: WebTransport セッションを確立してサーバーのデータを受信する
    let client_task = tokio::spawn(async move {
        timeout(Duration::from_secs(10), async {
            let mut session =
                ClientWebTransportSession::connect_insecure(server_addr, "localhost", "/wt")
                    .await
                    .expect("クライアント作成失敗");

            session.handshake().await.expect("ハンドシェイク失敗");

            session
                .open_session(&format!("localhost:{}", server_addr.port()), "/wt")
                .await
                .expect("セッション確立失敗");

            // サーバーからのデータを受信する
            let mut received = None;
            for _ in 0..50 {
                session
                    .recv(Duration::from_millis(100))
                    .await
                    .expect("受信失敗");

                while let Some(event) = session.poll() {
                    if let Http3Event::WebTransportData { data, .. } = event {
                        received = Some(data);
                    }
                }

                if received.is_some() {
                    break;
                }
            }

            received
        })
        .await
    });

    // サーバーを手動駆動する: セッション確立後にコネクション ID 指定の API で送信する
    let mut session_established = false;
    let mut data_sent = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);

    loop {
        let mut handler =
            |_addr: std::net::SocketAddr, _session_id: i64, event: Http3Event| -> bool {
                if let Http3Event::HeadersEnd { .. } = &event {
                    session_established = true;
                    return true;
                }
                false
            };

        server
            .recv_once(Duration::from_millis(100), &mut handler)
            .await
            .expect("recv_once 失敗");

        // セッション確立後にコネクション ID 指定の API で双方向ストリームを開いて送信する
        if session_established && !data_sent {
            let conn_ids = server.get_established_conn_ids();
            if let Some(conn_id) = conn_ids.first() {
                let stream_id = server
                    .open_bidi_stream_by_conn_id(conn_id)
                    .expect("サーバー bidi 作成失敗");
                server
                    .send_stream_data_by_conn_id(conn_id, stream_id, b"server-data", true)
                    .expect("データ送信失敗");
                server.flush().await.expect("フラッシュ失敗");
                data_sent = true;
            }
        }

        if client_task.is_finished() || tokio::time::Instant::now() >= deadline {
            break;
        }
    }

    // クライアントがサーバーからのデータを受信できたことを確認する
    let received = client_task
        .await
        .expect("クライアントタスク失敗")
        .expect("クライアントがタイムアウトした")
        .expect("サーバーからデータを受信するべき");

    assert_eq!(
        received, b"server-data",
        "コネクション ID 指定の API で送信したデータを受信するべき"
    );

    // セッションが確立できた = 不正パケットでサーバーが停止していない
    assert!(
        session_established,
        "不正パケットの後でも WebTransport セッションが確立するべき"
    );
}
