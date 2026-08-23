//! Server の堅牢性テスト
//!
//! 不正パケット・同一アドレスからの複数接続・アイドルタイムアウト・
//! 不正な HTTP/3 フレーム・ピアからの CONNECTION_CLOSE でサーバーが
//! 停止しないことを検証する。
//!
//! `Server::run` は無限ループのため、`tokio::select!` でテスト本体と
//! 並行駆動し、`get_conn_ids()` でサーバー内部の接続状態を検証する。

// 1 つの UDP ソケットで複数 QUIC 接続を駆動するテスト用クライアント。
// テスト間で共有するヘルパーは tests/helpers/ に置くが、`mod helpers;` で
// モジュール全体を取り込むと未使用部分が dead code になるため、
// 必要なファイルだけを明示的に取り込む。
#[path = "helpers/multi_conn_client.rs"]
mod multi_conn_client;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use multi_conn_client::MultiConnClient;
use shiguredo_ngtcp2::{ConnectionId, Header, Http3Event, TransportParams};
use tokio::time::timeout;
use tokio_ngtcp2::{Client, Server};

/// テスト用の証明書と秘密鍵を動的に生成する
fn generate_test_certs() -> (PathBuf, PathBuf) {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let unique_id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let temp_dir = std::env::temp_dir().join(format!(
        "server_e2e_test_{}_{}",
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

/// サーバーを bind してアドレスを返す
async fn bind_server(
    transport_params: Option<ngtcp2_sys::ngtcp2_transport_params>,
) -> (Server, std::net::SocketAddr) {
    let (cert_path, key_path) = generate_test_certs();

    let server = Server::bind(
        "127.0.0.1:0".parse().expect("test must succeed"),
        &cert_path,
        &key_path,
        transport_params,
        None,
    )
    .await
    .expect("test must succeed");

    let server_addr = server.local_addr();
    (server, server_addr)
}

/// サーバーを駆動しながらクライアントのハンドシェイクを実行する
///
/// サーバーが停止した場合は panic する。
async fn drive_server_and_handshake(
    server: &mut Server,
    server_addr: std::net::SocketAddr,
) -> Client {
    tokio::select! {
        biased;
        r = server.run(|_addr, _event| None) => {
            panic!("サーバーが停止するべきではない: {:?}", r);
        }
        result = timeout(Duration::from_secs(10), async {
            let mut client = Client::connect_insecure_default(server_addr, "localhost")
                .await
                .expect("クライアント作成失敗");
            client
                .handshake()
                .await
                .expect("ハンドシェイク失敗");
            client
        }) => {
            result.expect("ハンドシェイクがタイムアウトした")
        }
    }
}

/// 同一 SocketAddr から 2 接続を張ってもサーバーが継続することを確認する
///
/// 旧実装は 1 アドレス 1 接続しか保持できず、2 接続目は確立できなかった。
/// DCID ルーティング (RFC 9000 Section 5.1) により、1 つの UDP ソケットから
/// 複数の QUIC 接続を張れることを検証する。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_server_two_connections_same_socket() {
    let (mut server, server_addr) = bind_server(None).await;

    // サーバーを駆動しながら 2 接続を張る
    // ハンドラはリクエストに対して固定のレスポンスを返す
    tokio::select! {
        biased;
        r = server.run(move |_addr, event| match event {
            Http3Event::HeadersEnd { .. } => {
                Some((vec![Header::status(200)], b"ok".to_vec()))
            }
            _ => None,
        }) => {
            panic!("サーバーが停止するべきではない: {:?}", r);
        }
        result = timeout(Duration::from_secs(20), async {
            // 1 つの UDP ソケットで 2 接続を張るテスト用クライアント
            let mut client = MultiConnClient::new(server_addr)
                .await
                .expect("テスト用クライアント作成失敗");

            let conn1 = client.add_connection().expect("接続 1 作成失敗");
            let conn2 = client.add_connection().expect("接続 2 作成失敗");

            // 1 接続目を確立する
            client
                .handshake(&conn1, Duration::from_secs(10))
                .await
                .expect("接続 1 ハンドシェイク失敗");

            // 同一アドレスからの 2 接続目も確立できる
            client
                .handshake(&conn2, Duration::from_secs(10))
                .await
                .expect("接続 2 ハンドシェイク失敗");

            // 1 接続目が維持されたままであることをリクエスト / レスポンスで確認する
            client
                .send_request(&conn1, "GET", "/first")
                .expect("リクエスト送信失敗");
            let (status, body) = client
                .recv_response(&conn1, Duration::from_secs(10))
                .await
                .expect("応答受信失敗");
            assert_eq!(status, 200, "1 接続目のレスポンスステータス");
            assert_eq!(body, b"ok", "1 接続目のレスポンスボディ");

            // 2 接続目も確立していることをリクエスト / レスポンスで確認する
            client
                .send_request(&conn2, "GET", "/second")
                .expect("リクエスト送信失敗");
            let (status, body) = client
                .recv_response(&conn2, Duration::from_secs(10))
                .await
                .expect("応答受信失敗");
            assert_eq!(status, 200, "2 接続目のレスポンスステータス");
            assert_eq!(body, b"ok", "2 接続目のレスポンスボディ");

            (conn1, conn2, client)
        }) => {
            let (_conn1, _conn2, client) = result.expect("テストがタイムアウトした");
            // サーバー側に 2 接続が共存していることを確認する
            assert_eq!(
                server.get_conn_ids().len(),
                2,
                "サーバーは 2 接続を保持するべき"
            );

            // 旧 API (アドレス指定) は同一アドレスの複数接続で一意に特定できない
            assert!(
                server.send_response(client.local_addr(), 0, &[]).is_err(),
                "同一アドレスの複数接続では send_response はエラーになるべき"
            );
        }
    }
}

/// 不正なパケットを送りつけてもサーバーが継続することを確認する
///
/// DCID 不一致の Initial・破損した Initial・ヘッダーが途中で切れたパケットなど、
/// 任意の不正パケットでサーバーが停止しないことと、接続状態が残らないことを
/// 検証する。不正 Initial の破棄は RFC 9000 Section 11.1 で許可されている。
///
/// `run_by_conn_id` のハンドラはコネクション ID を受け取る。コネクション ID の
/// 対応する接続にリクエストが処理されることを検証する。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_server_run_by_conn_id_handles_two_connections() {
    let (mut server, server_addr) = bind_server(None).await;

    // 各コネクション ID から見えたイベントを収集する
    let conn_ids: std::sync::Arc<std::sync::Mutex<Vec<ConnectionId>>> =
        std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen_ids = conn_ids.clone();

    tokio::select! {
        biased;
        r = server.run_by_conn_id(move |conn_id, _addr, event| {
            {
                let mut ids = seen_ids.lock().expect("test mutex should not be poisoned");
                if !ids.contains(&conn_id) {
                    ids.push(conn_id.clone());
                }
            }
            match event {
                Http3Event::HeadersEnd { .. } => {
                    Some((vec![Header::status(200)], b"ok".to_vec()))
                }
                _ => None,
            }
        }) => {
            panic!("サーバーが停止するべきではない: {:?}", r);
        }
        result = timeout(Duration::from_secs(20), async {
            let mut client = MultiConnClient::new(server_addr)
                .await
                .expect("テスト用クライアント作成失敗");

            let conn1 = client.add_connection().expect("接続 1 作成失敗");
            let conn2 = client.add_connection().expect("接続 2 作成失敗");

            client
                .handshake(&conn1, Duration::from_secs(10))
                .await
                .expect("接続 1 ハンドシェイク失敗");
            client
                .handshake(&conn2, Duration::from_secs(10))
                .await
                .expect("接続 2 ハンドシェイク失敗");

            client
                .send_request(&conn1, "GET", "/first")
                .expect("リクエスト送信失敗");
            let (status, body) = client
                .recv_response(&conn1, Duration::from_secs(10))
                .await
                .expect("応答受信失敗");
            assert_eq!(status, 200, "1 接続目のレスポンスステータス");
            assert_eq!(body, b"ok", "1 接続目のレスポンスボディ");

            client
                .send_request(&conn2, "GET", "/second")
                .expect("リクエスト送信失敗");
            let (status, body) = client
                .recv_response(&conn2, Duration::from_secs(10))
                .await
                .expect("応答受信失敗");
            assert_eq!(status, 200, "2 接続目のレスポンスステータス");
            assert_eq!(body, b"ok", "2 接続目のレスポンスボディ");

            client
        }) => {
            let _client = result.expect("テストがタイムアウトした");

            // ハンドラに 2 つの異なるコネクション ID が渡されることを検証する
            let ids = conn_ids.lock().expect("test mutex should not be poisoned");
            assert_eq!(ids.len(), 2, "2 つのコネクション ID が渡されるべき");
            assert_ne!(ids[0], ids[1], "各接続のコネクション ID は異なるべき");
        }
    }
}

/// 不正なパケットを送りつけてもサーバーが継続することを確認する
///
/// DCID 不一致の Initial・破損した Initial・ヘッダーが途中で切れたパケットなど、
/// 任意の不正パケットでサーバーが停止しないことと、接続状態が残らないことを
/// 検証する。不正 Initial の破棄は RFC 9000 Section 11.1 で許可されている。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_server_survives_malformed_packets() {
    let (mut server, server_addr) = bind_server(None).await;

    // サーバーを駆動しながら不正パケットを送りつける
    tokio::select! {
        biased;
        r = server.run(|_addr, _event| None) => {
            panic!("不正パケットでサーバーが停止するべきではない: {:?}", r);
        }
        _ = async {
            // 攻撃者ソケット (std::net::UdpSocket で十分)
            let attacker =
                std::net::UdpSocket::bind("127.0.0.1:0").expect("test must succeed");

            // 1. 完全なガベージデータ
            attacker
                .send_to(&[0u8; 64], server_addr)
                .expect("test must succeed");

            // 2. Long header に見えるが DCID が未登録でペイロードが破損した Initial
            let mut initial = vec![0xC0, 0, 0, 0, 1, 16]; // Initial, QUIC v1, DCID Length = 16
            initial.extend_from_slice(&[0xAB; 16]); // 未登録の DCID
            initial.push(8); // SCID Length = 8
            initial.extend_from_slice(&[0xCD; 8]); // SCID
            initial.extend_from_slice(&[0xFF; 1200]); // 破損したペイロード
            attacker
                .send_to(&initial, server_addr)
                .expect("test must succeed");

            // 3. ヘッダーが途中で切れたパケット
            attacker
                .send_to(&[0xC0, 0, 0, 0, 1], server_addr)
                .expect("test must succeed");

            // 4. Short header で DCID が未登録のパケット
            let mut short = vec![0x40]; // Short header
            short.extend_from_slice(&[0xEE; 16]); // 未登録の DCID
            short.extend_from_slice(&[0xFF; 32]); // ペイロード
            attacker
                .send_to(&short, server_addr)
                .expect("test must succeed");

            // 5. サポート外の QUIC バージョンの Initial
            let mut bad_version = vec![0xC0, 0xDE, 0xAD, 0xBE, 0xEF, 16]; // 不明なバージョン
            bad_version.extend_from_slice(&[0xAB; 16]);
            bad_version.push(8);
            bad_version.extend_from_slice(&[0xCD; 8]);
            bad_version.extend_from_slice(&[0xFF; 1200]);
            attacker
                .send_to(&bad_version, server_addr)
                .expect("test must succeed");

            // 6. 0-RTT パケット (RFC 9000 Section 5.2.2 によりサーバーは破棄してよい)
            let mut zero_rtt = vec![0xD0, 0, 0, 0, 1, 16]; // 0-RTT, QUIC v1
            zero_rtt.extend_from_slice(&[0xAB; 16]);
            zero_rtt.push(8);
            zero_rtt.extend_from_slice(&[0xCD; 8]);
            zero_rtt.extend_from_slice(&[0xFF; 1200]);
            attacker
                .send_to(&zero_rtt, server_addr)
                .expect("test must succeed");

            // サーバーがパケットを処理するのを待つ
            tokio::time::sleep(Duration::from_millis(300)).await;
        } => {}
    }

    // 不正パケットで接続状態が残っていないことを確認する
    // (不正 Initial は ngtcp2 の read_pkt が DROP_CONN 等のエラーを返すため、
    //  ルーティングテーブルに登録される前に処理が打ち切られる)
    assert!(
        server.get_conn_ids().is_empty(),
        "不正パケットで接続状態が残るべきではない"
    );

    // 実クライアントが接続できる = サーバー継続
    let _client = drive_server_and_handshake(&mut server, server_addr).await;
}

/// クライアント切断 (アイドルタイムアウト) でサーバーが継続することを確認する
///
/// ハンドシェイク後にクライアントが沈黙すると、サーバー側で
/// NGTCP2_ERR_IDLE_CLOSE が発生する。このエラーでサーバーが停止せず、
/// 接続が除去されることを検証する。デフォルトの max_idle_timeout は 30 秒のため、
/// `with_max_idle_timeout` で 1 秒に短縮する。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_server_survives_idle_timeout() {
    // アイドルタイムアウトを 1 秒に短縮する (ナノ秒指定)
    let transport_params = TransportParams::new()
        .with_max_idle_timeout(1_000_000_000)
        .into_raw();

    let (mut server, server_addr) = bind_server(Some(transport_params)).await;

    // クライアントを接続してハンドシェイクだけ行い、その後沈黙する
    tokio::select! {
        biased;
        r = server.run(|_addr, _event| None) => {
            panic!("アイドルタイムアウトでサーバーが停止するべきではない: {:?}", r);
        }
        result = timeout(Duration::from_secs(10), async {
            let mut client = Client::connect_insecure_default(server_addr, "localhost")
                .await
                .expect("クライアント作成失敗");
            client
                .handshake()
                .await
                .expect("ハンドシェイク失敗");
            // クライアントをドロップして通信を止める
            drop(client);

            // サーバー側のアイドルタイムアウト (1 秒) が発火するのを待つ
            tokio::time::sleep(Duration::from_secs(3)).await;
        }) => {
            result.expect("テストがタイムアウトした");
        }
    }

    // アイドルタイムアウトで接続が除去されていることを確認する
    // (除去されない場合、接続が残り続けるかタイマー計算がビジーループになる)
    assert!(
        server.get_conn_ids().is_empty(),
        "アイドルタイムアウトで接続が除去されるべき"
    );

    // 新しいクライアントが接続できる = サーバー継続
    let _client = drive_server_and_handshake(&mut server, server_addr).await;
}

/// ハンドシェイク完了後に不正な HTTP/3 フレームを送ってもサーバーが継続することを確認する
///
/// ハンドシェイクを完了させた攻撃者が不正な HTTP/3 フレームを 1 個送っても、
/// その接続が CONNECTION_CLOSE で閉じられるだけでサーバーが停止しないことを
/// 検証する (RFC 9000 Section 11.1)。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_server_survives_malformed_h3_frame() {
    let (mut server, server_addr) = bind_server(None).await;

    // サーバーを駆動しながら不正な HTTP/3 フレームを送る
    tokio::select! {
        biased;
        r = server.run(|_addr, _event| None) => {
            panic!("不正な HTTP/3 フレームでサーバーが停止するべきではない: {:?}", r);
        }
        result = timeout(Duration::from_secs(20), async {
            // テスト用クライアントで接続してハンドシェイクを完了する
            let mut client = MultiConnClient::new(server_addr)
                .await
                .expect("テスト用クライアント作成失敗");
            let conn_key = client.add_connection().expect("接続作成失敗");
            client
                .handshake(&conn_key, Duration::from_secs(10))
                .await
                .expect("ハンドシェイク失敗");

            // ハンドシェイク完了後に、リクエスト用のクライアント開始双方向ストリーム
            // に HTTP/3 として不正なフレームを流す
            let stream_id = client.open_stream(&conn_key).expect("ストリーム開設失敗");
            client
                .send_raw_stream_data(&conn_key, stream_id, &[0xDE, 0xAD, 0xBE, 0xEF], true)
                .await
                .expect("不正フレーム送信失敗");

            // サーバーが不正フレームを処理して CONNECTION_CLOSE を送信するまで待つ
            for _ in 0..20 {
                client
                    .pump(Duration::from_millis(50))
                    .await
                    .expect("ポンプ失敗");
                if client.is_connection_closed(&conn_key) {
                    break;
                }
            }
            client.is_connection_closed(&conn_key)
        }) => {
            let closed = result.expect("テストがタイムアウトした");
            assert!(
                closed,
                "サーバーは不正な HTTP/3 フレームを受けた接続に CONNECTION_CLOSE を送るべき"
            );
        }
    }

    // サーバー側でも接続が除去されていることを確認する
    // (サーバーは CONNECTION_CLOSE 送信と同一ループ内で接続を除去するため、
    //  クライアントが close を観測できた時点で除去は完了している。
    //  biased select! でサーバーの停止検出を優先している)
    assert!(
        server.get_conn_ids().is_empty(),
        "不正な HTTP/3 フレームを受けた接続は除去されるべき"
    );

    // 新しいクライアントが接続できる = サーバー継続
    let _client = drive_server_and_handshake(&mut server, server_addr).await;
}

/// ピアから CONNECTION_CLOSE を受信してもサーバーが継続することを確認する
///
/// CONNECTION_CLOSE 受信で接続は終了状態 (draining) に移行し、
/// `read_pkt` は NGTCP2_ERR_DRAINING / NGTCP2_ERR_CLOSING (Terminal) を返す。
/// このエラーでサーバーが停止せず、接続が除去されることを検証する。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_server_survives_client_connection_close() {
    let (mut server, server_addr) = bind_server(None).await;

    // サーバーを駆動しながらクライアントから CONNECTION_CLOSE を送る
    tokio::select! {
        biased;
        r = server.run(|_addr, _event| None) => {
            panic!("CONNECTION_CLOSE 受信でサーバーが停止するべきではない: {:?}", r);
        }
        result = timeout(Duration::from_secs(20), async {
            let mut client = MultiConnClient::new(server_addr)
                .await
                .expect("テスト用クライアント作成失敗");
            let conn_key = client.add_connection().expect("接続作成失敗");
            client
                .handshake(&conn_key, Duration::from_secs(10))
                .await
                .expect("ハンドシェイク失敗");

            // クライアントから CONNECTION_CLOSE (NO_ERROR) を送る
            client
                .send_connection_close(&conn_key, 0)
                .await
                .expect("CONNECTION_CLOSE 送信失敗");

            // サーバーが処理するまで少し待つ
            tokio::time::sleep(Duration::from_millis(300)).await;
        }) => {
            result.expect("テストがタイムアウトした");
        }
    }

    // サーバー側で接続が除去されていることを確認する
    assert!(
        server.get_conn_ids().is_empty(),
        "CONNECTION_CLOSE を受けた接続は除去されるべき"
    );

    // 新しいクライアントが接続できる = サーバー継続
    let _client = drive_server_and_handshake(&mut server, server_addr).await;
}
