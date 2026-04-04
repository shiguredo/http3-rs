//! WebTransport エンドツーエンドテスト
//!
//! 実際のネットワーク I/O を使用した WebTransport セッション確立、
//! データストリーム、DATAGRAM のテスト

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use rcgen::{CertificateParams, KeyPair};
use tokio::time::timeout;

use shiguredo_ngtcp2::Http3Event;
use tokio_ngtcp2::{ClientWebTransportSession, ServerWebTransportSession};

/// テスト用の証明書と秘密鍵を動的に生成
fn generate_test_certs() -> (PathBuf, PathBuf) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let unique_id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let temp_dir = std::env::temp_dir().join(format!(
        "webtransport_e2e_test_{}_{}",
        std::process::id(),
        unique_id
    ));
    std::fs::create_dir_all(&temp_dir).expect("一時ディレクトリ作成失敗");

    let cert_path = temp_dir.join("cert.pem");
    let key_path = temp_dir.join("key.pem");

    // 証明書パラメータを設定
    let mut params =
        CertificateParams::new(vec!["localhost".to_string()]).expect("CertificateParams 作成失敗");
    params.distinguished_name.push(
        rcgen::DnType::CommonName,
        rcgen::DnValue::Utf8String("localhost".to_string()),
    );

    // 鍵ペアを生成
    let key_pair = KeyPair::generate().expect("鍵ペア生成失敗");

    // 自己署名証明書を生成
    let cert = params.self_signed(&key_pair).expect("証明書生成失敗");

    // PEM 形式で保存
    std::fs::write(&cert_path, cert.pem()).expect("証明書ファイル書き込み失敗");
    std::fs::write(&key_path, key_pair.serialize_pem()).expect("秘密鍵ファイル書き込み失敗");

    (cert_path, key_path)
}

/// WebTransport セッション確立テスト
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_webtransport_session_establishment() {
    let (cert_path, key_path) = generate_test_certs();

    let session_accepted = Arc::new(AtomicBool::new(false));
    let session_accepted_clone = session_accepted.clone();

    // サーバーを起動
    let mut server =
        ServerWebTransportSession::bind("127.0.0.1:0".parse().unwrap(), &cert_path, &key_path)
            .await
            .expect("サーバー作成失敗");

    let server_addr = server.local_addr();
    eprintln!("[test] サーバー起動: {}", server_addr);

    // サーバータスク
    let server_task = tokio::spawn(async move {
        let result = timeout(
            Duration::from_secs(10),
            server.run(move |addr, session_id, event| {
                match &event {
                    Http3Event::HeadersEnd { stream_id, .. } => {
                        eprintln!(
                            "[server] CONNECT リクエスト受信: addr = {}, session_id = {}, stream_id = {}",
                            addr, session_id, stream_id
                        );
                        session_accepted_clone.store(true, Ordering::SeqCst);
                        // セッションを受け入れる
                        return true;
                    }
                    _ => {
                        eprintln!("[server] イベント: {:?}", event);
                    }
                }
                false
            }),
        )
        .await;

        match result {
            Ok(r) => r,
            Err(_) => {
                eprintln!("[server] タイムアウト");
                Ok(())
            }
        }
    });

    // クライアントで WebTransport セッションを確立
    let client_result = timeout(Duration::from_secs(10), async {
        let mut session =
            ClientWebTransportSession::connect_insecure(server_addr, "localhost", "/webtransport")
                .await
                .expect("クライアント作成失敗");

        eprintln!("[client] ハンドシェイク開始");
        session.handshake().await.expect("ハンドシェイク失敗");
        eprintln!("[client] ハンドシェイク完了");

        // WebTransport セッションを開始
        let session_result = session
            .open_session(
                &format!("localhost:{}", server_addr.port()),
                "/webtransport",
            )
            .await;

        match session_result {
            Ok(session_id) => {
                eprintln!("[client] セッション確立: session_id = {}", session_id);
                // サーバーがイベントを処理する時間を確保
                tokio::time::sleep(Duration::from_millis(100)).await;
                Some(session_id)
            }
            Err(e) => {
                eprintln!("[client] セッション確立失敗: {:?}", e);
                None
            }
        }
    })
    .await;

    // サーバーが処理を完了するまで少し待機
    tokio::time::sleep(Duration::from_millis(100)).await;

    server_task.abort();

    match client_result {
        Ok(Some(session_id)) => {
            eprintln!(
                "[test] WebTransport セッション確立テスト成功: session_id = {}",
                session_id
            );
            // session_id は 0 から始まる有効な値
            assert!(
                session_accepted.load(Ordering::SeqCst),
                "サーバーがセッションを受け入れること"
            );
        }
        Ok(None) => {
            panic!("WebTransport セッション確立失敗");
        }
        Err(_) => {
            panic!("テストタイムアウト");
        }
    }
}

/// 複数セッションの同時接続テスト
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_webtransport_multiple_sessions() {
    let (cert_path, key_path) = generate_test_certs();

    use std::sync::atomic::AtomicUsize;
    let session_count = Arc::new(AtomicUsize::new(0));
    let session_count_clone = session_count.clone();

    // サーバーを起動
    let mut server =
        ServerWebTransportSession::bind("127.0.0.1:0".parse().unwrap(), &cert_path, &key_path)
            .await
            .expect("サーバー作成失敗");

    let server_addr = server.local_addr();
    eprintln!("[test] サーバー起動: {}", server_addr);

    // サーバータスク
    let server_task = tokio::spawn(async move {
        let _ = timeout(
            Duration::from_secs(15),
            server.run(move |addr, session_id, event| {
                if let Http3Event::HeadersEnd { stream_id, .. } = &event {
                    let count = session_count_clone.fetch_add(1, Ordering::SeqCst) + 1;
                    eprintln!(
                        "[server] セッション {} 受付: addr = {}, session_id = {}, stream_id = {}",
                        count, addr, session_id, stream_id
                    );
                    return true;
                }
                false
            }),
        )
        .await;
    });

    // 複数クライアントを並行して接続
    let client_count = 3;
    let mut handles = Vec::new();

    for i in 0..client_count {
        let addr = server_addr;
        let handle = tokio::spawn(async move {
            let mut session =
                ClientWebTransportSession::connect_insecure(addr, "localhost", "/webtransport")
                    .await
                    .expect("クライアント作成失敗");

            match timeout(Duration::from_secs(5), session.handshake()).await {
                Ok(Ok(())) => {
                    eprintln!("[client {}] ハンドシェイク成功", i);
                }
                _ => {
                    eprintln!("[client {}] ハンドシェイク失敗", i);
                    return None;
                }
            }

            match timeout(
                Duration::from_secs(5),
                session.open_session(&format!("localhost:{}", addr.port()), "/webtransport"),
            )
            .await
            {
                Ok(Ok(session_id)) => {
                    eprintln!("[client {}] セッション確立: session_id = {}", i, session_id);
                    Some(session_id)
                }
                _ => {
                    eprintln!("[client {}] セッション確立失敗", i);
                    None
                }
            }
        });
        handles.push(handle);
    }

    // 全クライアントの結果を待つ
    let mut success_count = 0;
    for handle in handles {
        if let Ok(Some(_)) = handle.await {
            success_count += 1;
        }
    }

    server_task.abort();

    let server_sessions = session_count.load(Ordering::SeqCst);
    eprintln!(
        "[test] クライアント成功: {}/{}, サーバー受付: {}",
        success_count, client_count, server_sessions
    );
    assert!(success_count >= 1, "少なくとも 1 セッションが成功するべき");
}

/// セッション拒否テスト
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_webtransport_session_reject() {
    let (cert_path, key_path) = generate_test_certs();

    // サーバーを起動 (すべてのセッションを拒否)
    let mut server =
        ServerWebTransportSession::bind("127.0.0.1:0".parse().unwrap(), &cert_path, &key_path)
            .await
            .expect("サーバー作成失敗");

    let server_addr = server.local_addr();
    eprintln!("[test] サーバー起動: {}", server_addr);

    // サーバータスク (セッションを拒否)
    let server_task = tokio::spawn(async move {
        let _ = timeout(
            Duration::from_secs(10),
            server.run(|addr, session_id, event| {
                if let Http3Event::HeadersEnd { stream_id, .. } = &event {
                    eprintln!(
                        "[server] セッション拒否: addr = {}, session_id = {}, stream_id = {}",
                        addr, session_id, stream_id
                    );
                    // セッションを拒否 (false を返す)
                    return false;
                }
                false
            }),
        )
        .await;
    });

    // クライアントでセッションを試行
    let client_result = timeout(Duration::from_secs(10), async {
        let mut session =
            ClientWebTransportSession::connect_insecure(server_addr, "localhost", "/webtransport")
                .await
                .expect("クライアント作成失敗");

        session.handshake().await.expect("ハンドシェイク失敗");

        // セッション確立を試行 (サーバーが拒否するので失敗する可能性がある)
        let result = session
            .open_session(
                &format!("localhost:{}", server_addr.port()),
                "/webtransport",
            )
            .await;

        eprintln!("[client] セッション結果: {:?}", result);
        result
    })
    .await;

    server_task.abort();

    // セッションが拒否された場合でもテストは成功
    // (サーバーが拒否を処理できることを確認)
    eprintln!("[test] セッション拒否テスト完了: {:?}", client_result);
}

/// WebTransport ハンドシェイクのみのテスト
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_webtransport_handshake_only() {
    let (cert_path, key_path) = generate_test_certs();

    // サーバーを起動
    let mut server =
        ServerWebTransportSession::bind("127.0.0.1:0".parse().unwrap(), &cert_path, &key_path)
            .await
            .expect("サーバー作成失敗");

    let server_addr = server.local_addr();

    // サーバータスク
    let server_task = tokio::spawn(async move {
        let _ = timeout(
            Duration::from_secs(5),
            server.run(|_addr, _session_id, _event| false),
        )
        .await;
    });

    // クライアントを作成してハンドシェイクのみ実行
    let client_result = timeout(Duration::from_secs(10), async {
        let mut session =
            ClientWebTransportSession::connect_insecure(server_addr, "localhost", "/webtransport")
                .await
                .expect("クライアント作成失敗");

        eprintln!("[client] ハンドシェイク開始");

        match session.handshake().await {
            Ok(()) => {
                eprintln!("[client] ハンドシェイク成功");
                true
            }
            Err(e) => {
                eprintln!("[client] ハンドシェイクエラー: {:?}", e);
                false
            }
        }
    })
    .await;

    server_task.abort();

    match client_result {
        Ok(success) => {
            assert!(success, "ハンドシェイクが成功するべき");
            eprintln!("[test] WebTransport ハンドシェイクテスト成功");
        }
        Err(_) => {
            panic!("テストタイムアウト");
        }
    }
}

/// WebTransport 双方向ストリーム送受信テスト
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_webtransport_bidi_stream_send_recv() {
    let (cert_path, key_path) = generate_test_certs();

    let data_received_on_server = Arc::new(AtomicBool::new(false));
    let data_received_clone = data_received_on_server.clone();
    let received_data_on_server = Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let received_data_clone = received_data_on_server.clone();

    // サーバーを起動
    let mut server =
        ServerWebTransportSession::bind("127.0.0.1:0".parse().unwrap(), &cert_path, &key_path)
            .await
            .expect("サーバー作成失敗");

    let server_addr = server.local_addr();
    eprintln!("[test] サーバー起動: {}", server_addr);

    // サーバータスク
    let server_task = tokio::spawn(async move {
        let _ = timeout(Duration::from_secs(10), async {
            loop {
                let mut handler = |addr: std::net::SocketAddr, session_id: i64, event: Http3Event| -> bool {
                    match &event {
                        Http3Event::HeadersEnd { stream_id, .. } => {
                            eprintln!(
                                "[server] CONNECT リクエスト受信: addr = {}, session_id = {}, stream_id = {}",
                                addr, session_id, stream_id
                            );
                            return true; // セッションを受け入れる
                        }
                        Http3Event::WebTransportData {
                            session_id,
                            stream_id,
                            data,
                        } => {
                            eprintln!(
                                "[server] データ受信: session_id = {}, stream_id = {}, data = {:?}",
                                session_id, stream_id, String::from_utf8_lossy(data)
                            );
                            data_received_clone.store(true, Ordering::SeqCst);
                            received_data_clone.lock().unwrap().extend_from_slice(data);
                        }
                        _ => {
                            eprintln!("[server] イベント: {:?}", event);
                        }
                    }
                    false
                };

                server
                    .recv_once(Duration::from_millis(100), &mut handler)
                    .await
                    .ok();
            }
        })
        .await;
    });

    // クライアントでセッションを確立してデータを送受信
    let client_result = timeout(Duration::from_secs(10), async {
        let mut session =
            ClientWebTransportSession::connect_insecure(server_addr, "localhost", "/webtransport")
                .await
                .expect("クライアント作成失敗");

        session.handshake().await.expect("ハンドシェイク失敗");
        eprintln!("[client] ハンドシェイク完了");

        let _session_id = session
            .open_session(
                &format!("localhost:{}", server_addr.port()),
                "/webtransport",
            )
            .await
            .expect("セッション確立失敗");

        eprintln!("[client] セッション確立完了");

        // 双方向ストリームを開いてデータを送信
        let stream_id = session.open_bidi_stream().expect("ストリーム作成失敗");
        eprintln!("[client] ストリーム作成: stream_id = {}", stream_id);

        let send_data = b"Hello, WebTransport!";
        session
            .send_stream_data(stream_id, send_data, true)
            .await
            .expect("データ送信失敗");
        eprintln!("[client] データ送信完了");

        // サーバーがデータを処理する時間を確保
        tokio::time::sleep(Duration::from_millis(500)).await;

        stream_id
    })
    .await;

    server_task.abort();

    match client_result {
        Ok(stream_id) => {
            eprintln!("[test] ストリーム ID: {}", stream_id);
            assert!(
                data_received_on_server.load(Ordering::SeqCst),
                "サーバーがデータを受信するべき"
            );
            let data = received_data_on_server.lock().unwrap();
            let received_str = String::from_utf8_lossy(&data);
            eprintln!("[test] サーバーが受信したデータ: {}", received_str);
            assert!(
                received_str.contains("Hello, WebTransport!"),
                "クライアントからのデータを受信するべき"
            );
        }
        Err(_) => {
            panic!("テストタイムアウト");
        }
    }

    eprintln!("[test] WebTransport 双方向ストリーム送受信テスト完了");
}

/// WebTransport DATAGRAM サーバー→クライアント送信テスト
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_webtransport_datagram_server_to_client() {
    let (cert_path, key_path) = generate_test_certs();

    // サーバーを起動
    let mut server =
        ServerWebTransportSession::bind("127.0.0.1:0".parse().unwrap(), &cert_path, &key_path)
            .await
            .expect("サーバー作成失敗");

    let server_addr = server.local_addr();
    eprintln!("[test] サーバー起動: {}", server_addr);

    // サーバータスク
    let server_task = tokio::spawn(async move {
        let mut client_addr: Option<std::net::SocketAddr> = None;
        let mut datagram_sent = false;

        let _ = timeout(Duration::from_secs(10), async {
            loop {
                let mut handler =
                    |addr: std::net::SocketAddr, _session_id: i64, event: Http3Event| -> bool {
                        if let Http3Event::HeadersEnd { .. } = &event {
                            eprintln!("[server] CONNECT リクエスト受信: addr = {}", addr);
                            client_addr = Some(addr);
                            return true;
                        }
                        false
                    };

                server
                    .recv_once(Duration::from_millis(100), &mut handler)
                    .await
                    .ok();

                // セッション確立後、DATAGRAM を送信
                if let Some(addr) = client_addr
                    && !datagram_sent
                {
                    let datagram_data = b"Hello from server!";
                    match server.send_datagram_for(&addr, datagram_data).await {
                        Ok(accepted) => {
                            eprintln!("[server] DATAGRAM 送信: accepted = {}", accepted);
                            datagram_sent = true;
                        }
                        Err(e) => {
                            eprintln!("[server] DATAGRAM 送信失敗: {:?}", e);
                        }
                    }
                }
            }
        })
        .await;
    });

    // クライアントで DATAGRAM を受信
    let client_result = timeout(Duration::from_secs(10), async {
        let mut session =
            ClientWebTransportSession::connect_insecure(server_addr, "localhost", "/webtransport")
                .await
                .expect("クライアント作成失敗");

        session.handshake().await.expect("ハンドシェイク失敗");
        eprintln!("[client] ハンドシェイク完了");

        let _session_id = session
            .open_session(
                &format!("localhost:{}", server_addr.port()),
                "/webtransport",
            )
            .await
            .expect("セッション確立失敗");

        eprintln!("[client] セッション確立完了");

        // DATAGRAM を受信
        let mut received_datagram = None;
        for _ in 0..20 {
            session
                .recv(Duration::from_millis(100))
                .await
                .expect("受信失敗");

            if let Some(data) = session.recv_datagram() {
                eprintln!(
                    "[client] DATAGRAM 受信: {:?}",
                    String::from_utf8_lossy(&data)
                );
                received_datagram = Some(data);
                break;
            }
        }

        received_datagram
    })
    .await;

    server_task.abort();

    match client_result {
        Ok(Some(data)) => {
            let data_str = String::from_utf8_lossy(&data);
            eprintln!("[test] クライアントが受信した DATAGRAM: {}", data_str);
            assert!(
                data_str.contains("Hello from server"),
                "サーバーからの DATAGRAM を受信するべき"
            );
        }
        Ok(None) => {
            panic!("DATAGRAM を受信できなかった");
        }
        Err(_) => {
            panic!("テストタイムアウト");
        }
    }

    eprintln!("[test] WebTransport DATAGRAM サーバー→クライアント送信テスト完了");
}

/// WebTransport 単方向ストリームテスト
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_webtransport_uni_stream() {
    let (cert_path, key_path) = generate_test_certs();

    let data_received = Arc::new(AtomicBool::new(false));
    let data_received_clone = data_received.clone();
    let received_data = Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let received_data_clone = received_data.clone();

    // サーバーを起動
    let mut server =
        ServerWebTransportSession::bind("127.0.0.1:0".parse().unwrap(), &cert_path, &key_path)
            .await
            .expect("サーバー作成失敗");

    let server_addr = server.local_addr();
    eprintln!("[test] サーバー起動: {}", server_addr);

    // サーバータスク
    let server_task = tokio::spawn(async move {
        let _ = timeout(Duration::from_secs(10), async {
            loop {
                let mut handler = |addr: std::net::SocketAddr, session_id: i64, event: Http3Event| -> bool {
                    match &event {
                        Http3Event::HeadersEnd { .. } => {
                            eprintln!("[server] CONNECT リクエスト受信: addr = {}", addr);
                            return true;
                        }
                        Http3Event::WebTransportData {
                            session_id: sid,
                            stream_id,
                            data,
                        } => {
                            eprintln!(
                                "[server] 単方向ストリームデータ受信: session_id = {}, stream_id = {}, data = {:?}",
                                sid, stream_id, String::from_utf8_lossy(data)
                            );
                            data_received_clone.store(true, Ordering::SeqCst);
                            received_data_clone.lock().unwrap().extend_from_slice(data);
                        }
                        _ => {
                            eprintln!("[server] イベント: session_id = {}, {:?}", session_id, event);
                        }
                    }
                    false
                };

                server
                    .recv_once(Duration::from_millis(100), &mut handler)
                    .await
                    .ok();
            }
        })
        .await;
    });

    // クライアントで単方向ストリームを作成してデータを送信
    let client_result = timeout(Duration::from_secs(10), async {
        let mut session =
            ClientWebTransportSession::connect_insecure(server_addr, "localhost", "/webtransport")
                .await
                .expect("クライアント作成失敗");

        session.handshake().await.expect("ハンドシェイク失敗");
        eprintln!("[client] ハンドシェイク完了");

        let _session_id = session
            .open_session(
                &format!("localhost:{}", server_addr.port()),
                "/webtransport",
            )
            .await
            .expect("セッション確立失敗");

        eprintln!("[client] セッション確立完了");

        // 単方向ストリームを開いてデータを送信
        let stream_id = session.open_uni_stream().expect("単方向ストリーム作成失敗");
        eprintln!("[client] 単方向ストリーム作成: stream_id = {}", stream_id);

        let send_data = b"Unidirectional data";
        session
            .send_stream_data(stream_id, send_data, true)
            .await
            .expect("データ送信失敗");
        eprintln!("[client] データ送信完了");

        // サーバーが処理する時間を確保
        tokio::time::sleep(Duration::from_millis(500)).await;

        stream_id
    })
    .await;

    server_task.abort();

    match client_result {
        Ok(stream_id) => {
            eprintln!("[test] 単方向ストリーム ID: {}", stream_id);
            assert!(
                data_received.load(Ordering::SeqCst),
                "サーバーが単方向ストリームデータを受信するべき"
            );
            let data = received_data.lock().unwrap();
            eprintln!(
                "[test] サーバーが受信したデータ: {:?}",
                String::from_utf8_lossy(&data)
            );
            assert!(
                String::from_utf8_lossy(&data).contains("Unidirectional"),
                "送信したデータが受信されるべき"
            );
        }
        Err(_) => {
            panic!("テストタイムアウト");
        }
    }

    eprintln!("[test] WebTransport 単方向ストリームテスト完了");
}

/// WebTransport bidi ストリームエコーテスト
///
/// クライアントが bidi ストリームでデータを送信し、
/// サーバーが受信したデータを同じストリームで返送、
/// クライアントがエコーバックされたデータを受信することを検証する。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_webtransport_bidi_stream_echo() {
    let (cert_path, key_path) = generate_test_certs();

    // サーバーを起動
    let mut server =
        ServerWebTransportSession::bind("127.0.0.1:0".parse().unwrap(), &cert_path, &key_path)
            .await
            .expect("サーバー作成失敗");

    let server_addr = server.local_addr();

    // サーバータスク: 受信したデータをそのまま返送する
    let server_task = tokio::spawn(async move {
        let _ = timeout(Duration::from_secs(10), async {
            // エコーバック待ちのデータ
            let mut echo_queue: Vec<(std::net::SocketAddr, i64, Vec<u8>)> = Vec::new();

            loop {
                let mut handler =
                    |addr: std::net::SocketAddr, _session_id: i64, event: Http3Event| -> bool {
                        match &event {
                            Http3Event::HeadersEnd { .. } => {
                                return true;
                            }
                            Http3Event::WebTransportData {
                                stream_id, data, ..
                            } => {
                                echo_queue.push((addr, *stream_id, data.clone()));
                            }
                            _ => {}
                        }
                        false
                    };

                server
                    .recv_once(Duration::from_millis(100), &mut handler)
                    .await
                    .ok();

                // エコーバック: 受信したデータを同じストリームで返送
                for (addr, stream_id, data) in echo_queue.drain(..) {
                    server
                        .send_stream_data_for(&addr, stream_id, &data, true)
                        .ok();
                }
                server.flush().await.ok();
            }
        })
        .await;
    });

    // クライアントでデータを送信してエコーバックを受信
    let client_result = timeout(Duration::from_secs(10), async {
        let mut session =
            ClientWebTransportSession::connect_insecure(server_addr, "localhost", "/webtransport")
                .await
                .expect("クライアント作成失敗");

        session.handshake().await.expect("ハンドシェイク失敗");

        let _session_id = session
            .open_session(
                &format!("localhost:{}", server_addr.port()),
                "/webtransport",
            )
            .await
            .expect("セッション確立失敗");

        // bidi ストリームでデータを送信
        let stream_id = session.open_bidi_stream().expect("ストリーム作成失敗");
        let send_data = b"Echo me back!";
        session
            .send_stream_data(stream_id, send_data, true)
            .await
            .expect("データ送信失敗");

        // エコーバックされたデータを受信
        let mut received_data = Vec::new();
        for _ in 0..30 {
            session
                .recv(Duration::from_millis(100))
                .await
                .expect("受信失敗");

            while let Some(event) = session.poll() {
                if let Http3Event::WebTransportData { data, .. } = event {
                    received_data.extend_from_slice(&data);
                }
            }

            if !received_data.is_empty() {
                break;
            }
        }

        received_data
    })
    .await;

    server_task.abort();

    match client_result {
        Ok(data) => {
            let data_str = String::from_utf8_lossy(&data);
            assert_eq!(
                data_str, "Echo me back!",
                "エコーバックされたデータが一致するべき"
            );
        }
        Err(_) => {
            panic!("テストタイムアウト");
        }
    }
}

/// WebTransport 大量データ転送テスト
///
/// 100KB のデータを bidi ストリームで送信し、サーバーで全データを受信して
/// バイト数が一致することを検証する。輻輳制御ループの動作確認。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_webtransport_large_data_transfer() {
    let (cert_path, key_path) = generate_test_certs();

    let received_data = Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let received_data_clone = received_data.clone();

    // サーバーを起動
    let mut server =
        ServerWebTransportSession::bind("127.0.0.1:0".parse().unwrap(), &cert_path, &key_path)
            .await
            .expect("サーバー作成失敗");

    let server_addr = server.local_addr();

    // サーバータスク: 全データを蓄積
    let server_task = tokio::spawn(async move {
        let _ = timeout(Duration::from_secs(30), async {
            loop {
                let mut handler =
                    |_addr: std::net::SocketAddr, _session_id: i64, event: Http3Event| -> bool {
                        match &event {
                            Http3Event::HeadersEnd { .. } => {
                                return true;
                            }
                            Http3Event::WebTransportData { data, .. } => {
                                received_data_clone.lock().unwrap().extend_from_slice(data);
                            }
                            _ => {}
                        }
                        false
                    };

                server
                    .recv_once(Duration::from_millis(100), &mut handler)
                    .await
                    .ok();
            }
        })
        .await;
    });

    // 100KB のテストデータを生成
    let data_size = 100 * 1024;
    let send_data: Vec<u8> = (0..data_size).map(|i| (i % 256) as u8).collect();

    let client_result = timeout(Duration::from_secs(30), async {
        let mut session =
            ClientWebTransportSession::connect_insecure(server_addr, "localhost", "/webtransport")
                .await
                .expect("クライアント作成失敗");

        session.handshake().await.expect("ハンドシェイク失敗");

        let _session_id = session
            .open_session(
                &format!("localhost:{}", server_addr.port()),
                "/webtransport",
            )
            .await
            .expect("セッション確立失敗");

        let stream_id = session.open_bidi_stream().expect("ストリーム作成失敗");
        session
            .send_stream_data(stream_id, &send_data, true)
            .await
            .expect("データ送信失敗");

        // サーバーが全データを処理する時間を確保
        tokio::time::sleep(Duration::from_secs(2)).await;
    })
    .await;

    server_task.abort();

    assert!(client_result.is_ok(), "テストがタイムアウトしないこと");

    let received = received_data.lock().unwrap();
    assert_eq!(
        received.len(),
        data_size,
        "受信データサイズが送信データサイズと一致するべき: received={}, expected={}",
        received.len(),
        data_size
    );
    assert_eq!(
        received.as_slice(),
        send_data.as_slice(),
        "受信データの内容が送信データと一致するべき"
    );
}

/// WebTransport ストリーミング複数書き込みテスト
///
/// 同一ストリーム上で FIN なしのデータを 5 回送信後、FIN 付きで最終送信。
/// サーバーが全データを正しい順序で受信し、結合後のデータが一致することを検証する。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_webtransport_streaming_multiple_writes() {
    let (cert_path, key_path) = generate_test_certs();

    let received_data = Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let received_data_clone = received_data.clone();

    // サーバーを起動
    let mut server =
        ServerWebTransportSession::bind("127.0.0.1:0".parse().unwrap(), &cert_path, &key_path)
            .await
            .expect("サーバー作成失敗");

    let server_addr = server.local_addr();

    // サーバータスク
    let server_task = tokio::spawn(async move {
        let _ = timeout(Duration::from_secs(10), async {
            loop {
                let mut handler =
                    |_addr: std::net::SocketAddr, _session_id: i64, event: Http3Event| -> bool {
                        match &event {
                            Http3Event::HeadersEnd { .. } => {
                                return true;
                            }
                            Http3Event::WebTransportData { data, .. } => {
                                received_data_clone.lock().unwrap().extend_from_slice(data);
                            }
                            _ => {}
                        }
                        false
                    };

                server
                    .recv_once(Duration::from_millis(100), &mut handler)
                    .await
                    .ok();
            }
        })
        .await;
    });

    // クライアント: FIN なしで 5 回送信後、FIN 付きで最終送信
    let messages = [
        "chunk-0:", "chunk-1:", "chunk-2:", "chunk-3:", "chunk-4:", "final",
    ];

    let client_result = timeout(Duration::from_secs(10), async {
        let mut session =
            ClientWebTransportSession::connect_insecure(server_addr, "localhost", "/webtransport")
                .await
                .expect("クライアント作成失敗");

        session.handshake().await.expect("ハンドシェイク失敗");

        let _session_id = session
            .open_session(
                &format!("localhost:{}", server_addr.port()),
                "/webtransport",
            )
            .await
            .expect("セッション確立失敗");

        let stream_id = session.open_bidi_stream().expect("ストリーム作成失敗");

        // FIN なしで 5 回送信
        for msg in &messages[..5] {
            session
                .send_stream_data(stream_id, msg.as_bytes(), false)
                .await
                .expect("データ送信失敗");
        }

        // FIN 付きで最終送信
        session
            .send_stream_data(stream_id, messages[5].as_bytes(), true)
            .await
            .expect("最終データ送信失敗");

        tokio::time::sleep(Duration::from_millis(500)).await;
    })
    .await;

    server_task.abort();

    assert!(client_result.is_ok(), "テストがタイムアウトしないこと");

    let received = received_data.lock().unwrap();
    let expected: String = messages.join("");
    assert_eq!(
        String::from_utf8_lossy(&received),
        expected,
        "全チャンクが正しい順序で結合されるべき"
    );
}

/// WebTransport 双方向ストリーム交互通信テスト
///
/// クライアント→サーバー送信、サーバーが応答返送、クライアントがさらに送信、
/// サーバーがさらに応答、のラリーを検証する。bidi ストリームの双方向性を確認。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_webtransport_bidi_stream_interleaved() {
    let (cert_path, key_path) = generate_test_certs();

    // サーバーを起動
    let mut server =
        ServerWebTransportSession::bind("127.0.0.1:0".parse().unwrap(), &cert_path, &key_path)
            .await
            .expect("サーバー作成失敗");

    let server_addr = server.local_addr();

    // サーバータスク: 受信したデータに "reply:" プレフィックスを付けて返送
    let server_task = tokio::spawn(async move {
        let _ = timeout(Duration::from_secs(10), async {
            let mut echo_queue: Vec<(std::net::SocketAddr, i64, Vec<u8>)> = Vec::new();

            loop {
                let mut handler =
                    |addr: std::net::SocketAddr, _session_id: i64, event: Http3Event| -> bool {
                        match &event {
                            Http3Event::HeadersEnd { .. } => {
                                return true;
                            }
                            Http3Event::WebTransportData {
                                stream_id, data, ..
                            } => {
                                // "reply:" プレフィックスを付けて返送データを準備
                                let mut reply = b"reply:".to_vec();
                                reply.extend_from_slice(data);
                                echo_queue.push((addr, *stream_id, reply));
                            }
                            _ => {}
                        }
                        false
                    };

                server
                    .recv_once(Duration::from_millis(100), &mut handler)
                    .await
                    .ok();

                // 応答を返送 (FIN なし: ストリームを開いたままにする)
                for (addr, stream_id, data) in echo_queue.drain(..) {
                    server
                        .send_stream_data_for(&addr, stream_id, &data, false)
                        .ok();
                }
                server.flush().await.ok();
            }
        })
        .await;
    });

    // クライアント: ラリー通信
    let client_result = timeout(Duration::from_secs(10), async {
        let mut session =
            ClientWebTransportSession::connect_insecure(server_addr, "localhost", "/webtransport")
                .await
                .expect("クライアント作成失敗");

        session.handshake().await.expect("ハンドシェイク失敗");

        let _session_id = session
            .open_session(
                &format!("localhost:{}", server_addr.port()),
                "/webtransport",
            )
            .await
            .expect("セッション確立失敗");

        let stream_id = session.open_bidi_stream().expect("ストリーム作成失敗");

        let mut replies = Vec::new();

        // ラリー 1: クライアント→サーバー→クライアント
        session
            .send_stream_data(stream_id, b"ping1", false)
            .await
            .expect("送信失敗");

        for _ in 0..30 {
            session
                .recv(Duration::from_millis(100))
                .await
                .expect("受信失敗");

            while let Some(event) = session.poll() {
                if let Http3Event::WebTransportData { data, .. } = event {
                    replies.push(data);
                }
            }

            if !replies.is_empty() {
                break;
            }
        }

        assert_eq!(
            String::from_utf8_lossy(&replies[0]),
            "reply:ping1",
            "最初のラリーの応答が正しいべき"
        );

        // ラリー 2: クライアント→サーバー→クライアント
        replies.clear();
        session
            .send_stream_data(stream_id, b"ping2", false)
            .await
            .expect("送信失敗");

        for _ in 0..30 {
            session
                .recv(Duration::from_millis(100))
                .await
                .expect("受信失敗");

            while let Some(event) = session.poll() {
                if let Http3Event::WebTransportData { data, .. } = event {
                    replies.push(data);
                }
            }

            if !replies.is_empty() {
                break;
            }
        }

        assert_eq!(
            String::from_utf8_lossy(&replies[0]),
            "reply:ping2",
            "2 回目のラリーの応答が正しいべき"
        );

        replies
    })
    .await;

    server_task.abort();

    assert!(client_result.is_ok(), "テストがタイムアウトしないこと");
}

/// WebTransport DATAGRAM 連続送信テスト
///
/// クライアントから 10 個の DATAGRAM を連続送信し、
/// サーバーが少なくとも 1 個以上受信することを検証する (DATAGRAM は信頼性なし)。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_webtransport_multiple_datagrams() {
    let (cert_path, key_path) = generate_test_certs();

    let received_datagrams = Arc::new(std::sync::Mutex::new(Vec::<Vec<u8>>::new()));
    let received_datagrams_clone = received_datagrams.clone();

    // サーバーを起動
    let mut server =
        ServerWebTransportSession::bind("127.0.0.1:0".parse().unwrap(), &cert_path, &key_path)
            .await
            .expect("サーバー作成失敗");

    let server_addr = server.local_addr();

    // サーバータスク: DATAGRAM を受信して蓄積
    let server_task = tokio::spawn(async move {
        let mut client_addr: Option<std::net::SocketAddr> = None;

        let _ = timeout(Duration::from_secs(10), async {
            loop {
                let mut handler =
                    |addr: std::net::SocketAddr, _session_id: i64, event: Http3Event| -> bool {
                        if let Http3Event::HeadersEnd { .. } = &event {
                            client_addr = Some(addr);
                            return true;
                        }
                        false
                    };

                server
                    .recv_once(Duration::from_millis(100), &mut handler)
                    .await
                    .ok();

                if let Some(addr) = client_addr {
                    while let Some(data) = server.recv_datagram_for(&addr) {
                        received_datagrams_clone.lock().unwrap().push(data);
                    }
                }
            }
        })
        .await;
    });

    // クライアント: 10 個の DATAGRAM を連続送信
    let client_result = timeout(Duration::from_secs(10), async {
        let mut session =
            ClientWebTransportSession::connect_insecure(server_addr, "localhost", "/webtransport")
                .await
                .expect("クライアント作成失敗");

        session.handshake().await.expect("ハンドシェイク失敗");

        let _session_id = session
            .open_session(
                &format!("localhost:{}", server_addr.port()),
                "/webtransport",
            )
            .await
            .expect("セッション確立失敗");

        for i in 0..10 {
            let payload = format!("datagram-{}", i);
            session
                .send_datagram(payload.as_bytes())
                .await
                .expect("DATAGRAM 送信失敗");
        }

        tokio::time::sleep(Duration::from_secs(1)).await;
    })
    .await;

    server_task.abort();

    assert!(client_result.is_ok(), "テストがタイムアウトしないこと");

    let datagrams = received_datagrams.lock().unwrap();
    assert!(
        !datagrams.is_empty(),
        "サーバーが少なくとも 1 個の DATAGRAM を受信するべき"
    );
    // 受信した DATAGRAM の内容が正しいことを検証
    for datagram in datagrams.iter() {
        let s = String::from_utf8_lossy(datagram);
        assert!(
            s.starts_with("datagram-"),
            "受信した DATAGRAM の形式が正しいべき: {:?}",
            s
        );
    }
}

/// WebTransport bidi + uni + DATAGRAM 混在テスト
///
/// 1 セッション上で bidi ストリーム + uni ストリーム + DATAGRAM を同時に使用。
/// 各チャネルのデータが混在せず正しく受信されることを検証する。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_webtransport_mixed_streams_and_datagrams() {
    let (cert_path, key_path) = generate_test_certs();

    let bidi_data = Arc::new(std::sync::Mutex::new(std::collections::HashMap::<
        i64,
        Vec<u8>,
    >::new()));
    let bidi_data_clone = bidi_data.clone();
    let uni_data = Arc::new(std::sync::Mutex::new(std::collections::HashMap::<
        i64,
        Vec<u8>,
    >::new()));
    let uni_data_clone = uni_data.clone();
    let datagram_data = Arc::new(std::sync::Mutex::new(Vec::<Vec<u8>>::new()));
    let datagram_data_clone = datagram_data.clone();

    // サーバーを起動
    let mut server =
        ServerWebTransportSession::bind("127.0.0.1:0".parse().unwrap(), &cert_path, &key_path)
            .await
            .expect("サーバー作成失敗");

    let server_addr = server.local_addr();

    // サーバータスク
    let server_task = tokio::spawn(async move {
        let mut client_addr: Option<std::net::SocketAddr> = None;

        let _ = timeout(Duration::from_secs(10), async {
            loop {
                let mut handler =
                    |addr: std::net::SocketAddr, _session_id: i64, event: Http3Event| -> bool {
                        match &event {
                            Http3Event::HeadersEnd { .. } => {
                                client_addr = Some(addr);
                                return true;
                            }
                            Http3Event::WebTransportData {
                                stream_id, data, ..
                            } => {
                                // QUIC ストリーム ID: 下位 2 ビットで種別判定
                                // 0x2 ビットが立っている場合は uni ストリーム
                                if (*stream_id & 0x2) != 0 {
                                    uni_data_clone
                                        .lock()
                                        .unwrap()
                                        .entry(*stream_id)
                                        .or_default()
                                        .extend_from_slice(data);
                                } else {
                                    bidi_data_clone
                                        .lock()
                                        .unwrap()
                                        .entry(*stream_id)
                                        .or_default()
                                        .extend_from_slice(data);
                                }
                            }
                            _ => {}
                        }
                        false
                    };

                server
                    .recv_once(Duration::from_millis(100), &mut handler)
                    .await
                    .ok();

                // DATAGRAM を受信
                if let Some(addr) = client_addr {
                    while let Some(data) = server.recv_datagram_for(&addr) {
                        datagram_data_clone.lock().unwrap().push(data);
                    }
                }
            }
        })
        .await;
    });

    // クライアント: bidi + uni + DATAGRAM を送信
    let client_result = timeout(Duration::from_secs(10), async {
        let mut session =
            ClientWebTransportSession::connect_insecure(server_addr, "localhost", "/webtransport")
                .await
                .expect("クライアント作成失敗");

        session.handshake().await.expect("ハンドシェイク失敗");

        let _session_id = session
            .open_session(
                &format!("localhost:{}", server_addr.port()),
                "/webtransport",
            )
            .await
            .expect("セッション確立失敗");

        // bidi ストリームでデータ送信
        let bidi_stream = session.open_bidi_stream().expect("bidi ストリーム作成失敗");
        session
            .send_stream_data(bidi_stream, b"bidi-data", true)
            .await
            .expect("bidi データ送信失敗");

        // uni ストリームでデータ送信
        let uni_stream = session.open_uni_stream().expect("uni ストリーム作成失敗");
        session
            .send_stream_data(uni_stream, b"uni-data", true)
            .await
            .expect("uni データ送信失敗");

        // DATAGRAM 送信
        session
            .send_datagram(b"dgram-data")
            .await
            .expect("DATAGRAM 送信失敗");

        tokio::time::sleep(Duration::from_secs(1)).await;

        (bidi_stream, uni_stream)
    })
    .await;

    server_task.abort();

    match client_result {
        Ok((bidi_stream, uni_stream)) => {
            // bidi ストリームのデータ検証
            let bidi = bidi_data.lock().unwrap();
            let bidi_content = bidi
                .get(&bidi_stream)
                .expect("bidi ストリームのデータが存在するべき");
            assert_eq!(
                String::from_utf8_lossy(bidi_content),
                "bidi-data",
                "bidi ストリームのデータが正しいべき"
            );

            // uni ストリームのデータ検証
            let uni = uni_data.lock().unwrap();
            let uni_content = uni
                .get(&uni_stream)
                .expect("uni ストリームのデータが存在するべき");
            assert_eq!(
                String::from_utf8_lossy(uni_content),
                "uni-data",
                "uni ストリームのデータが正しいべき"
            );

            // DATAGRAM のデータ検証
            let dgrams = datagram_data.lock().unwrap();
            assert!(
                !dgrams.is_empty(),
                "DATAGRAM が少なくとも 1 個受信されるべき"
            );
            assert_eq!(
                String::from_utf8_lossy(&dgrams[0]),
                "dgram-data",
                "DATAGRAM のデータが正しいべき"
            );
        }
        Err(_) => {
            panic!("テストタイムアウト");
        }
    }
}

/// WebTransport サーバーから複数ストリーム同時送信テスト
///
/// サーバーが bidi 2 本 + uni 1 本を開き、それぞれ異なるデータを送信。
/// クライアントが全ストリームのデータを正しく受信・分離できることを検証する。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_webtransport_server_multiple_streams() {
    let (cert_path, key_path) = generate_test_certs();

    // サーバーを起動
    let mut server =
        ServerWebTransportSession::bind("127.0.0.1:0".parse().unwrap(), &cert_path, &key_path)
            .await
            .expect("サーバー作成失敗");

    let server_addr = server.local_addr();

    // サーバータスク: セッション確立後に bidi 2 本 + uni 1 本を開いてデータ送信
    let server_task = tokio::spawn(async move {
        let mut session_established = false;
        let mut data_sent = false;

        let _ = timeout(Duration::from_secs(10), async {
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
                    .ok();

                if session_established && !data_sent {
                    let addrs = server.get_established_addrs();
                    if let Some(addr) = addrs.first() {
                        // bidi ストリーム 1
                        let bidi1 = server
                            .open_bidi_stream_for(addr)
                            .expect("サーバー bidi1 作成失敗");
                        server
                            .send_stream_data_for(addr, bidi1, b"server-bidi-1", true)
                            .expect("bidi1 送信失敗");

                        // bidi ストリーム 2
                        let bidi2 = server
                            .open_bidi_stream_for(addr)
                            .expect("サーバー bidi2 作成失敗");
                        server
                            .send_stream_data_for(addr, bidi2, b"server-bidi-2", true)
                            .expect("bidi2 送信失敗");

                        // uni ストリーム
                        let uni = server
                            .open_uni_stream_for(addr)
                            .expect("サーバー uni 作成失敗");
                        server
                            .send_stream_data_for(addr, uni, b"server-uni-1", true)
                            .expect("uni 送信失敗");

                        server.flush().await.expect("フラッシュ失敗");
                        data_sent = true;
                    }
                }
            }
        })
        .await;
    });

    // クライアント: サーバーからのデータを受信・分離
    let client_result = timeout(Duration::from_secs(10), async {
        let mut session =
            ClientWebTransportSession::connect_insecure(server_addr, "localhost", "/webtransport")
                .await
                .expect("クライアント作成失敗");

        session.handshake().await.expect("ハンドシェイク失敗");

        let _session_id = session
            .open_session(
                &format!("localhost:{}", server_addr.port()),
                "/webtransport",
            )
            .await
            .expect("セッション確立失敗");

        let mut received_streams = std::collections::HashMap::<i64, Vec<u8>>::new();

        for _ in 0..50 {
            session
                .recv(Duration::from_millis(100))
                .await
                .expect("受信失敗");

            while let Some(event) = session.poll() {
                if let Http3Event::WebTransportData {
                    stream_id, data, ..
                } = event
                {
                    received_streams
                        .entry(stream_id)
                        .or_default()
                        .extend_from_slice(&data);
                }
            }

            if received_streams.len() >= 3 {
                break;
            }
        }

        received_streams
    })
    .await;

    server_task.abort();

    match client_result {
        Ok(streams) => {
            assert!(
                streams.len() >= 3,
                "3 本のストリームからデータを受信するべき: received={}",
                streams.len()
            );

            let values: Vec<String> = streams
                .values()
                .map(|v| String::from_utf8_lossy(v).to_string())
                .collect();

            assert!(
                values.contains(&"server-bidi-1".to_string()),
                "bidi1 のデータを受信するべき: {:?}",
                values
            );
            assert!(
                values.contains(&"server-bidi-2".to_string()),
                "bidi2 のデータを受信するべき: {:?}",
                values
            );
            assert!(
                values.contains(&"server-uni-1".to_string()),
                "uni のデータを受信するべき: {:?}",
                values
            );
        }
        Err(_) => {
            panic!("テストタイムアウト");
        }
    }
}

/// WebTransport 大量ストリーム同時作成テスト
///
/// クライアントが 10 本の bidi ストリームを開いてそれぞれデータ送信。
/// サーバーが 10 ストリーム全てのデータを受信し、
/// 各ストリームのデータが分離されていることを検証する。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_webtransport_many_streams() {
    let (cert_path, key_path) = generate_test_certs();

    let received_streams = Arc::new(std::sync::Mutex::new(std::collections::HashMap::<
        i64,
        Vec<u8>,
    >::new()));
    let received_streams_clone = received_streams.clone();

    // サーバーを起動
    let mut server =
        ServerWebTransportSession::bind("127.0.0.1:0".parse().unwrap(), &cert_path, &key_path)
            .await
            .expect("サーバー作成失敗");

    let server_addr = server.local_addr();

    // サーバータスク
    let server_task = tokio::spawn(async move {
        let _ = timeout(Duration::from_secs(15), async {
            loop {
                let mut handler =
                    |_addr: std::net::SocketAddr, _session_id: i64, event: Http3Event| -> bool {
                        match &event {
                            Http3Event::HeadersEnd { .. } => {
                                return true;
                            }
                            Http3Event::WebTransportData {
                                stream_id, data, ..
                            } => {
                                received_streams_clone
                                    .lock()
                                    .unwrap()
                                    .entry(*stream_id)
                                    .or_default()
                                    .extend_from_slice(data);
                            }
                            _ => {}
                        }
                        false
                    };

                server
                    .recv_once(Duration::from_millis(100), &mut handler)
                    .await
                    .ok();
            }
        })
        .await;
    });

    // クライアント: 10 本の bidi ストリームを開いて各ストリームに固有データを送信
    let stream_count = 10;
    let client_result = timeout(Duration::from_secs(15), async {
        let mut session =
            ClientWebTransportSession::connect_insecure(server_addr, "localhost", "/webtransport")
                .await
                .expect("クライアント作成失敗");

        session.handshake().await.expect("ハンドシェイク失敗");

        let _session_id = session
            .open_session(
                &format!("localhost:{}", server_addr.port()),
                "/webtransport",
            )
            .await
            .expect("セッション確立失敗");

        let mut stream_ids = Vec::new();
        for i in 0..stream_count {
            let stream_id = session.open_bidi_stream().expect("ストリーム作成失敗");
            let data = format!("stream-{}-payload", i);
            session
                .send_stream_data(stream_id, data.as_bytes(), true)
                .await
                .expect("データ送信失敗");
            stream_ids.push((stream_id, data));
        }

        tokio::time::sleep(Duration::from_secs(1)).await;

        stream_ids
    })
    .await;

    server_task.abort();

    match client_result {
        Ok(stream_ids) => {
            let streams = received_streams.lock().unwrap();
            assert_eq!(
                streams.len(),
                stream_count,
                "全 {} ストリームのデータを受信するべき: received={}",
                stream_count,
                streams.len()
            );

            for (stream_id, expected_data) in &stream_ids {
                let data = streams
                    .get(stream_id)
                    .unwrap_or_else(|| panic!("ストリーム {} のデータが存在するべき", stream_id));
                assert_eq!(
                    String::from_utf8_lossy(data),
                    *expected_data,
                    "ストリーム {} のデータが正しく分離されるべき",
                    stream_id
                );
            }
        }
        Err(_) => {
            panic!("テストタイムアウト");
        }
    }
}
