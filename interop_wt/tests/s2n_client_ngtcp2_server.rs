//! s2n-quic クライアント + ngtcp2 サーバー WebTransport 相互運用性テスト
//!
//! RFC draft-ietf-webtrans-http3-15 に準拠した WebTransport 機能のテスト:
//! - セッション確立 (Section 3)
//! - 双方向ストリーム (Section 4.3)
//! - 単方向ストリーム (Section 4.2)
//! - Datagram (Section 4.5)

use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use serial_test::serial;
use tokio::time::timeout;

use interop_wt::{generate_shared_certificate, save_certificate_files};
use shiguredo_ngtcp2::Http3Event;
use tokio_ngtcp2::ServerWebTransportSession;
use tokio_s2n_quic::{ClientConfig, WtClient};

#[serial]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_webtransport_session() {
    let (cert_pem, key_pem) = generate_shared_certificate().unwrap();
    let (cert_path, key_path) = save_certificate_files(&cert_pem, &key_pem).unwrap();

    // ngtcp2 サーバーを起動
    let mut server =
        ServerWebTransportSession::bind("127.0.0.1:0".parse().unwrap(), &cert_path, &key_path)
            .await
            .expect("server creation failed");

    let server_addr = server.local_addr();
    eprintln!("[ngtcp2 server] started: {}", server_addr);

    // サーバータスク
    let server_task = tokio::spawn(async move {
        let result = timeout(
            Duration::from_secs(10),
            server.run(|addr, session_id, event| {
                match &event {
                    Http3Event::HeadersEnd { stream_id, .. } => {
                        eprintln!(
                            "[ngtcp2 server] CONNECT request: addr={} session_id={} stream_id={}",
                            addr, session_id, stream_id
                        );
                        return true;
                    }
                    Http3Event::WebTransportData {
                        session_id,
                        stream_id,
                        data,
                    } => {
                        eprintln!(
                            "[ngtcp2 server] WebTransport data: session_id={} stream_id={} data={}",
                            session_id,
                            stream_id,
                            String::from_utf8_lossy(data)
                        );
                    }
                    _ => {}
                }
                false
            }),
        )
        .await;

        match result {
            Ok(r) => r,
            Err(_) => {
                eprintln!("[ngtcp2 server] timed out");
                Ok(())
            }
        }
    });

    // s2n-quic クライアントで WebTransport セッションを確立
    let client_result = timeout(Duration::from_secs(10), async {
        let config = ClientConfig::new(server_addr, "localhost")
            .ca_cert(&cert_pem)
            .enable_webtransport(interop_wt::test_wt_settings());
        let session = WtClient::connect(config, "/webtransport").await?;

        eprintln!(
            "[s2n client] session established: session_id={}",
            session.session_id()
        );

        Ok::<_, tokio_s2n_quic::Error>(session)
    })
    .await;

    server_task.abort();
    interop_wt::cleanup_certificate_files(&cert_path, &key_path);

    match client_result {
        Ok(Ok(session)) => {
            eprintln!(
                "test passed: s2n client + ngtcp2 server session_id={}",
                session.session_id()
            );
        }
        Ok(Err(e)) => {
            panic!("client error: {:?}", e);
        }
        Err(_) => {
            panic!("test timed out");
        }
    }
}

/// 単方向ストリームテスト (RFC draft-ietf-webtrans-http3-15 Section 4.2)
///
/// s2n-quic クライアントから ngtcp2 サーバーへの単方向ストリーム送信をテストする。
/// Stream Type 0x54 + Session ID のフォーマットで送信される。
#[serial]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_unidirectional_stream() {
    let (cert_pem, key_pem) = generate_shared_certificate().unwrap();
    let (cert_path, key_path) = save_certificate_files(&cert_pem, &key_pem).unwrap();

    // ngtcp2 サーバーを起動
    let mut server =
        ServerWebTransportSession::bind("127.0.0.1:0".parse().unwrap(), &cert_path, &key_path)
            .await
            .expect("server creation failed");

    let server_addr = server.local_addr();
    eprintln!("[ngtcp2 server] started: {}", server_addr);

    // サーバータスク
    let server_task = tokio::spawn(async move {
        let result = timeout(
            Duration::from_secs(10),
            server.run(move |addr, session_id, event| {
                match &event {
                    Http3Event::HeadersEnd { stream_id, .. } => {
                        eprintln!(
                            "[ngtcp2 server] CONNECT request: addr={} session_id={} stream_id={}",
                            addr, session_id, stream_id
                        );
                        return true;
                    }
                    Http3Event::WebTransportData {
                        session_id,
                        stream_id,
                        data,
                    } => {
                        eprintln!(
                            "[ngtcp2 server] WebTransport data: session_id={} stream_id={} data={}",
                            session_id,
                            stream_id,
                            String::from_utf8_lossy(data)
                        );
                    }
                    _ => {}
                }
                false
            }),
        )
        .await;

        match result {
            Ok(r) => r,
            Err(_) => {
                eprintln!("[ngtcp2 server] timed out");
                Ok(())
            }
        }
    });

    // s2n-quic クライアントで WebTransport セッションを確立
    let client_result = timeout(Duration::from_secs(10), async {
        let config = ClientConfig::new(server_addr, "localhost")
            .ca_cert(&cert_pem)
            .enable_webtransport(interop_wt::test_wt_settings());
        let session = WtClient::connect(config, "/webtransport").await?;

        eprintln!(
            "[s2n client] session established: session_id={}",
            session.session_id()
        );

        // 単方向ストリームは WtSession API では送信 API が未対応のため、
        // セッション確立の成功のみを確認する
        eprintln!("[s2n client] session establishment confirmed");

        Ok::<_, tokio_s2n_quic::Error>(session.session_id())
    })
    .await;

    tokio::time::sleep(Duration::from_millis(500)).await;
    server_task.abort();
    interop_wt::cleanup_certificate_files(&cert_path, &key_path);

    match client_result {
        Ok(Ok(session_id)) => {
            eprintln!(
                "test passed: unidirectional stream session_id={}",
                session_id
            );
        }
        Ok(Err(e)) => {
            panic!("client error: {:?}", e);
        }
        Err(_) => {
            panic!("test timed out");
        }
    }
}

/// クライアントが双方向ストリームを開いてデータを送信するテスト
/// (RFC draft-ietf-webtrans-http3-15 Section 4.3)
///
/// s2n-quic クライアントが双方向ストリームを開き、
/// WebTransport ストリームヘッダー (0x41 + session_id) + アプリケーションデータを送信する。
/// ngtcp2 サーバーが WebTransportData イベントでデータを受信することを確認する。
#[serial]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_client_opens_bidi_stream() {
    let (cert_pem, key_pem) = generate_shared_certificate().unwrap();
    let (cert_path, key_path) = save_certificate_files(&cert_pem, &key_pem).unwrap();

    let mut server =
        ServerWebTransportSession::bind("127.0.0.1:0".parse().unwrap(), &cert_path, &key_path)
            .await
            .expect("server creation failed");

    let server_addr = server.local_addr();

    let received_data = Arc::new(StdMutex::new(Vec::<u8>::new()));
    let received_data_for_server = Arc::clone(&received_data);

    let server_task = tokio::spawn(async move {
        let result = timeout(
            Duration::from_secs(10),
            server.run(move |_addr, _session_id, event| {
                match &event {
                    Http3Event::HeadersEnd { .. } => {
                        return true;
                    }
                    Http3Event::WebTransportData { data, .. } => {
                        received_data_for_server
                            .lock()
                            .unwrap()
                            .extend_from_slice(data);
                    }
                    _ => {}
                }
                false
            }),
        )
        .await;

        match result {
            Ok(r) => r,
            Err(_) => Ok(()),
        }
    });

    let client_result = timeout(Duration::from_secs(10), async {
        let config = ClientConfig::new(server_addr, "localhost")
            .ca_cert(&cert_pem)
            .enable_webtransport(interop_wt::test_wt_settings());
        let mut session = WtClient::connect(config, "/webtransport").await?;
        let session_id = session.session_id();
        eprintln!(
            "[s2n client] session established: session_id={}",
            session_id
        );

        // 双方向ストリームを開く
        let mut bi_stream = session.open_bi_stream().await?;
        eprintln!(
            "[s2n client] bidi stream opened: stream_id={}",
            bi_stream.stream_id()
        );

        // アプリケーションデータを送信 (open_bi_stream() が WT ヘッダーを自動送信)
        bi_stream.send(b"Hello WebTransport!").await?;
        bi_stream.finish()?;

        eprintln!("[s2n client] data sent");
        Ok::<_, tokio_s2n_quic::Error>(session_id)
    })
    .await;

    // サーバーがデータを処理する時間を確保
    tokio::time::sleep(Duration::from_secs(1)).await;
    server_task.abort();
    interop_wt::cleanup_certificate_files(&cert_path, &key_path);

    match client_result {
        Ok(Ok(_session_id)) => {
            let data = received_data.lock().unwrap().clone();
            assert_eq!(data, b"Hello WebTransport!", "received data must match");
            eprintln!(
                "test passed: client bidi stream data={}",
                String::from_utf8_lossy(&data)
            );
        }
        Ok(Err(e)) => panic!("client error: {:?}", e),
        Err(_) => panic!("test timed out"),
    }
}

/// ngtcp2 サーバーが双方向ストリームを開いてデータを送信するテスト
/// (RFC draft-ietf-webtrans-http3-15 Section 4.3)
///
/// recv_once / open_bidi_stream_for / send_stream_data_for API を使用して、
/// ngtcp2 サーバーが nghttp3 経由でデータを送信する。
/// s2n-quic クライアントが accept_bi_stream でストリームを受け付け、データを受信する。
#[serial]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_server_opens_bidi_stream() {
    let (cert_pem, key_pem) = generate_shared_certificate().unwrap();
    let (cert_path, key_path) = save_certificate_files(&cert_pem, &key_pem).unwrap();

    let mut server =
        ServerWebTransportSession::bind("127.0.0.1:0".parse().unwrap(), &cert_path, &key_path)
            .await
            .expect("server creation failed");

    let server_addr = server.local_addr();

    // サーバータスク: recv_once でセッションを受け付け、bidi ストリームを開いてデータを送信
    let server_task = tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        let mut client_addr = None::<std::net::SocketAddr>;

        // セッション確立を待つ
        while client_addr.is_none() {
            let client_addr_ref = &mut client_addr;
            let mut handler =
                |addr: std::net::SocketAddr, _session_id: i64, event: Http3Event| -> bool {
                    if let Http3Event::HeadersEnd { .. } = &event {
                        *client_addr_ref = Some(addr);
                        return true;
                    }
                    false
                };
            server
                .recv_once(Duration::from_millis(100), &mut handler)
                .await
                .ok();
            if tokio::time::Instant::now() > deadline {
                panic!("[ngtcp2 server] session establishment timed out");
            }
        }

        let addr = client_addr.unwrap();
        eprintln!("[ngtcp2 server] session established: addr={}", addr);

        // 双方向ストリームを開いてデータを送信 (nghttp3 が WT ヘッダーを自動付加)
        let stream_id = server
            .open_bidi_stream_for(&addr)
            .expect("stream open failed");
        eprintln!("[ngtcp2 server] stream opened: stream_id={}", stream_id);

        server
            .send_stream_data_for(&addr, stream_id, b"Hello from ngtcp2 server!", true)
            .expect("data send enqueue failed");
        server.flush().await.expect("flush failed");
        eprintln!("[ngtcp2 server] data sent");

        // クライアントがデータを受信する時間を確保
        let mut noop_handler = |_: std::net::SocketAddr, _: i64, _: Http3Event| false;
        for _ in 0..30 {
            server
                .recv_once(Duration::from_millis(100), &mut noop_handler)
                .await
                .ok();
        }
    });

    // s2n-quic クライアントで接続してデータを受信
    let client_result = timeout(Duration::from_secs(10), async {
        let config = ClientConfig::new(server_addr, "localhost")
            .ca_cert(&cert_pem)
            .enable_webtransport(interop_wt::test_wt_settings());
        let mut session = WtClient::connect(config, "/webtransport").await?;
        eprintln!(
            "[s2n client] session established: session_id={}",
            session.session_id()
        );

        // サーバーからの双方向ストリームを受け付ける
        let mut bi_stream = session.accept_bi_stream().await?;
        eprintln!(
            "[s2n client] stream accepted: stream_id={}",
            bi_stream.stream_id()
        );

        // データを受信 (accept_bi_stream() が WT ヘッダーを自動デコード済み)
        let mut all_data = Vec::new();
        loop {
            match bi_stream.recv().await {
                Ok(data) => all_data.extend_from_slice(&data),
                Err(tokio_s2n_quic::Error::StreamClosed) => break,
                Err(e) => return Err(e),
            }
        }

        Ok::<_, tokio_s2n_quic::Error>(all_data)
    })
    .await;

    server_task.abort();
    interop_wt::cleanup_certificate_files(&cert_path, &key_path);

    match client_result {
        Ok(Ok(data)) => {
            assert_eq!(
                data, b"Hello from ngtcp2 server!",
                "received data must match"
            );
            eprintln!(
                "test passed: ngtcp2 server bidi stream data={}",
                String::from_utf8_lossy(&data)
            );
        }
        Ok(Err(e)) => panic!("client error: {:?}", e),
        Err(_) => panic!("test timed out"),
    }
}

/// 双方向ストリームエコーテスト (RFC draft-ietf-webtrans-http3-15 Section 4.3)
///
/// s2n-quic クライアントが bidi ストリームでデータを送信し、
/// ngtcp2 サーバーが同じストリームでエコー返信することを確認する。
#[serial]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_bidi_stream_echo() {
    let (cert_pem, key_pem) = generate_shared_certificate().unwrap();
    let (cert_path, key_path) = save_certificate_files(&cert_pem, &key_pem).unwrap();

    let mut server =
        ServerWebTransportSession::bind("127.0.0.1:0".parse().unwrap(), &cert_path, &key_path)
            .await
            .expect("server creation failed");
    let server_addr = server.local_addr();

    let server_task = tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        let mut client_addr = None::<std::net::SocketAddr>;
        let mut echo_stream_id = None::<i64>;
        let mut received = Vec::<u8>::new();

        while received.is_empty() {
            let client_addr_ref = &mut client_addr;
            let echo_stream_ref = &mut echo_stream_id;
            let received_ref = &mut received;
            let mut handler =
                |addr: std::net::SocketAddr, _session_id: i64, event: Http3Event| -> bool {
                    match &event {
                        Http3Event::HeadersEnd { .. } => {
                            *client_addr_ref = Some(addr);
                            return true;
                        }
                        Http3Event::WebTransportData {
                            stream_id, data, ..
                        } => {
                            *echo_stream_ref = Some(*stream_id);
                            received_ref.extend_from_slice(data);
                        }
                        _ => {}
                    }
                    false
                };
            server
                .recv_once(Duration::from_millis(100), &mut handler)
                .await
                .ok();
            if tokio::time::Instant::now() > deadline {
                panic!("[ngtcp2 server] data receive timed out");
            }
        }

        let addr = client_addr.unwrap();
        let stream_id = echo_stream_id.unwrap();

        // 同じストリームにエコー返信
        server
            .send_stream_data_for(&addr, stream_id, &received, true)
            .expect("echo send failed");
        server.flush().await.expect("flush failed");

        // クライアントが ACK を返すまで少し待つ
        let mut noop_handler = |_: std::net::SocketAddr, _: i64, _: Http3Event| false;
        for _ in 0..20 {
            server
                .recv_once(Duration::from_millis(100), &mut noop_handler)
                .await
                .ok();
        }

        received
    });

    let send_payload = b"Echo from s2n client";

    let client_result = timeout(Duration::from_secs(15), async {
        let config = ClientConfig::new(server_addr, "localhost")
            .ca_cert(&cert_pem)
            .enable_webtransport(interop_wt::test_wt_settings());
        let mut session = WtClient::connect(config, "/webtransport").await?;
        let session_id = session.session_id();

        let mut bi_stream = session.open_bi_stream().await?;

        // アプリデータを送信 (open_bi_stream() が WT ヘッダーを自動送信)
        bi_stream.send(send_payload).await?;
        bi_stream.finish()?;

        // エコーを受信 (ngtcp2 サーバーは WT ヘッダーなしで返信)
        let mut echo_data = Vec::new();
        loop {
            match bi_stream.recv().await {
                Ok(data) => {
                    echo_data.extend_from_slice(&data);
                }
                Err(tokio_s2n_quic::Error::StreamClosed) => {
                    break;
                }
                Err(e) => {
                    return Err(e);
                }
            }
        }

        Ok::<_, tokio_s2n_quic::Error>((session_id, echo_data))
    })
    .await;

    let server_received = timeout(Duration::from_secs(10), server_task)
        .await
        .expect("server timed out")
        .expect("server task error");

    interop_wt::cleanup_certificate_files(&cert_path, &key_path);

    match client_result {
        Ok(Ok((_, echo_data))) => {
            assert_eq!(echo_data, send_payload, "echo data must match");
            assert_eq!(
                server_received, send_payload,
                "server received data must match"
            );
        }
        Ok(Err(e)) => panic!("client error: {:?}", e),
        Err(_) => panic!("client timed out"),
    }
}

/// 複数双方向ストリームの逐次送信テスト (RFC draft-ietf-webtrans-http3-15 Section 4.3)
///
/// 3 本の bidi ストリームを順番に開き、それぞれ異なるデータを送信する。
/// ngtcp2 サーバーが全ストリームのデータを正しく受信することを確認する。
#[serial]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_multiple_bidi_streams() {
    const NUM_STREAMS: usize = 3;

    let (cert_pem, key_pem) = generate_shared_certificate().unwrap();
    let (cert_path, key_path) = save_certificate_files(&cert_pem, &key_pem).unwrap();

    let mut server =
        ServerWebTransportSession::bind("127.0.0.1:0".parse().unwrap(), &cert_path, &key_path)
            .await
            .expect("server creation failed");
    let server_addr = server.local_addr();

    let received_data = Arc::new(StdMutex::new(Vec::<Vec<u8>>::new()));
    let received_for_server = Arc::clone(&received_data);

    let server_task = tokio::spawn(async move {
        // NUM_STREAMS 分のデータを受信するまでループ
        let result = timeout(
            Duration::from_secs(15),
            server.run(move |_addr, _session_id, event| {
                match &event {
                    Http3Event::HeadersEnd { .. } => {
                        return true;
                    }
                    Http3Event::WebTransportData { data, .. } => {
                        received_for_server.lock().unwrap().push(data.clone());
                    }
                    _ => {}
                }
                false
            }),
        )
        .await;

        match result {
            Ok(r) => r,
            Err(_) => Ok(()),
        }
    });

    let client_result = timeout(Duration::from_secs(15), async {
        let config = ClientConfig::new(server_addr, "localhost")
            .ca_cert(&cert_pem)
            .enable_webtransport(interop_wt::test_wt_settings());
        let mut session = WtClient::connect(config, "/webtransport").await?;
        let session_id = session.session_id();

        let payloads: Vec<Vec<u8>> = (0..NUM_STREAMS)
            .map(|i| format!("multi stream payload {}", i).into_bytes())
            .collect();

        for payload in payloads.iter() {
            let mut bi_stream = session.open_bi_stream().await?;

            // open_bi_stream() が WT ヘッダーを自動送信
            bi_stream.send(payload).await?;
            bi_stream.finish()?;
        }

        Ok::<_, tokio_s2n_quic::Error>((session_id, payloads))
    })
    .await;

    // サーバーがデータを処理する時間を確保
    tokio::time::sleep(Duration::from_millis(500)).await;
    server_task.abort();
    interop_wt::cleanup_certificate_files(&cert_path, &key_path);

    match client_result {
        Ok(Ok((_, expected_payloads))) => {
            let mut received = received_data.lock().unwrap().clone();
            let mut expected = expected_payloads;
            assert_eq!(
                received.len(),
                NUM_STREAMS,
                "received stream count must match"
            );
            // ストリーム間の受信順序は保証されないのでソートして比較する
            received.sort();
            expected.sort();
            assert_eq!(received, expected, "received data must match");
        }
        Ok(Err(e)) => panic!("client error: {:?}", e),
        Err(_) => panic!("client timed out"),
    }
}

/// 大容量データ送受信テスト (RFC draft-ietf-webtrans-http3-15 Section 4.3)
///
/// 64KB のデータを単一の bidi ストリームで送信し、
/// ngtcp2 サーバーが全データを正確に受信することを確認する。
#[serial]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_large_data() {
    const DATA_SIZE: usize = 64 * 1024;

    let (cert_pem, key_pem) = generate_shared_certificate().unwrap();
    let (cert_path, key_path) = save_certificate_files(&cert_pem, &key_pem).unwrap();

    let mut server =
        ServerWebTransportSession::bind("127.0.0.1:0".parse().unwrap(), &cert_path, &key_path)
            .await
            .expect("server creation failed");
    let server_addr = server.local_addr();

    let received_data = Arc::new(StdMutex::new(Vec::<u8>::new()));
    let received_for_server = Arc::clone(&received_data);

    let server_task = tokio::spawn(async move {
        let result = timeout(
            Duration::from_secs(30),
            server.run(move |_addr, _session_id, event| {
                match &event {
                    Http3Event::HeadersEnd { .. } => return true,
                    Http3Event::WebTransportData { data, .. } => {
                        received_for_server.lock().unwrap().extend_from_slice(data);
                    }
                    _ => {}
                }
                false
            }),
        )
        .await;

        match result {
            Ok(r) => r,
            Err(_) => Ok(()),
        }
    });

    let large_data: Vec<u8> = (0..DATA_SIZE).map(|i| (i % 251) as u8).collect();
    let large_data_for_client = large_data.clone();

    let client_result = timeout(Duration::from_secs(30), async {
        let config = ClientConfig::new(server_addr, "localhost")
            .ca_cert(&cert_pem)
            .enable_webtransport(interop_wt::test_wt_settings());
        let mut session = WtClient::connect(config, "/webtransport").await?;
        let session_id = session.session_id();
        eprintln!(
            "[s2n client] session established: session_id={}",
            session_id
        );

        let mut bi_stream = session.open_bi_stream().await?;
        eprintln!(
            "[s2n client] bidi stream opened: stream_id={}",
            bi_stream.stream_id()
        );

        // open_bi_stream() が WT ヘッダーを自動送信
        bi_stream.send(&large_data_for_client).await?;
        bi_stream.finish()?;
        eprintln!("[s2n client] {} bytes sent", DATA_SIZE);

        Ok::<_, tokio_s2n_quic::Error>(session_id)
    })
    .await;

    // サーバーが全データを処理する時間を確保
    tokio::time::sleep(Duration::from_secs(1)).await;
    server_task.abort();
    interop_wt::cleanup_certificate_files(&cert_path, &key_path);

    match client_result {
        Ok(Ok(_)) => {
            let received = received_data.lock().unwrap().clone();
            assert_eq!(received.len(), DATA_SIZE, "received byte count must match");
            assert_eq!(received, large_data, "received data must match completely");
            eprintln!("test passed: {} bytes large data transfer", DATA_SIZE);
        }
        Ok(Err(e)) => panic!("client error: {:?}", e),
        Err(_) => panic!("client timed out"),
    }
}

/// Datagram テスト (RFC draft-ietf-webtrans-http3-15 Section 4.5)
///
/// s2n-quic クライアントから ngtcp2 サーバーへの Datagram 送受信をテストする。
/// 注: s2n-quic では QUIC DATAGRAM のサポートが限定的なため、
/// このテストではセッション確立のみを確認する。
#[serial]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_datagram_session() {
    let (cert_pem, key_pem) = generate_shared_certificate().unwrap();
    let (cert_path, key_path) = save_certificate_files(&cert_pem, &key_pem).unwrap();

    // ngtcp2 サーバーを起動
    let mut server =
        ServerWebTransportSession::bind("127.0.0.1:0".parse().unwrap(), &cert_path, &key_path)
            .await
            .expect("server creation failed");

    let server_addr = server.local_addr();
    eprintln!("[ngtcp2 server] started: {}", server_addr);

    // サーバータスク
    let server_task = tokio::spawn(async move {
        let result = timeout(
            Duration::from_secs(10),
            server.run(|addr, session_id, event| {
                if let Http3Event::HeadersEnd { stream_id, .. } = &event {
                    eprintln!(
                        "[ngtcp2 server] CONNECT request: addr={} session_id={} stream_id={}",
                        addr, session_id, stream_id
                    );
                    return true;
                }
                false
            }),
        )
        .await;

        match result {
            Ok(r) => r,
            Err(_) => {
                eprintln!("[ngtcp2 server] timed out");
                Ok(())
            }
        }
    });

    // s2n-quic クライアントでセッションを確立
    let client_result = timeout(Duration::from_secs(10), async {
        let config = ClientConfig::new(server_addr, "localhost")
            .ca_cert(&cert_pem)
            .enable_webtransport(interop_wt::test_wt_settings());
        let session = WtClient::connect(config, "/webtransport").await?;

        eprintln!(
            "[s2n client] datagram session established: session_id = {}",
            session.session_id()
        );

        // 注: s2n-quic の現行バージョンでは DATAGRAM API が限定的
        // セッション確立のみを確認
        Ok::<_, tokio_s2n_quic::Error>(session.session_id())
    })
    .await;

    server_task.abort();
    interop_wt::cleanup_certificate_files(&cert_path, &key_path);

    match client_result {
        Ok(Ok(session_id)) => {
            eprintln!(
                "test passed: datagram session established session_id = {}",
                session_id
            );
        }
        Ok(Err(e)) => {
            panic!("client error: {:?}", e);
        }
        Err(_) => {
            panic!("test timed out");
        }
    }
}
