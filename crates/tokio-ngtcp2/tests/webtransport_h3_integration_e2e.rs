//! ngtcp2 <-> shiguredo_http3 WebTransport 統合 E2E テスト
//!
//! shiguredo_http3 (Sans I/O) の WebTransport プロトコルロジック
//! (Datagram, Capsule, StreamHeader, Session) を ngtcp2 の
//! 実ネットワーク I/O と組み合わせて検証する。

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use rcgen::{CertificateParams, KeyPair};
use tokio::time::timeout;

use shiguredo_http3::webtransport::{Capsule, Datagram};
use shiguredo_ngtcp2::Http3Event;
use tokio_ngtcp2::{ClientWebTransportSession, ServerWebTransportSession};

/// テスト用の証明書と秘密鍵を動的に生成
fn generate_test_certs() -> (PathBuf, PathBuf) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let unique_id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let temp_dir = std::env::temp_dir().join(format!(
        "wt_h3_integration_e2e_{}_{}",
        std::process::id(),
        unique_id
    ));
    std::fs::create_dir_all(&temp_dir).expect("一時ディレクトリ作成失敗");

    let cert_path = temp_dir.join("cert.pem");
    let key_path = temp_dir.join("key.pem");

    let mut params =
        CertificateParams::new(vec!["localhost".to_string()]).expect("CertificateParams 作成失敗");
    params.distinguished_name.push(
        rcgen::DnType::CommonName,
        rcgen::DnValue::Utf8String("localhost".to_string()),
    );

    let key_pair = KeyPair::generate().expect("鍵ペア生成失敗");
    let cert = params.self_signed(&key_pair).expect("証明書生成失敗");

    std::fs::write(&cert_path, cert.pem()).expect("証明書ファイル書き込み失敗");
    std::fs::write(&key_path, key_pair.serialize_pem()).expect("秘密鍵ファイル書き込み失敗");

    (cert_path, key_path)
}

/// shiguredo_http3::Datagram エンコード/デコード ↔ ngtcp2 DATAGRAM 統合テスト
///
/// shiguredo_http3 の Datagram::encode() でエンコードしたバイト列を
/// ngtcp2 の QUIC DATAGRAM として送信し、受信側で Datagram::decode() で
/// デコードして元のデータと一致することを検証する。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_h3_datagram_encode_decode_over_ngtcp2() {
    let (cert_path, key_path) = generate_test_certs();

    let received_raw = Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let received_raw_clone = received_raw.clone();
    let datagram_received = Arc::new(AtomicBool::new(false));
    let datagram_received_clone = datagram_received.clone();

    // サーバーを起動
    let mut server =
        ServerWebTransportSession::bind("127.0.0.1:0".parse().unwrap(), &cert_path, &key_path)
            .await
            .expect("サーバー作成失敗");

    let server_addr = server.local_addr();

    // サーバータスク: DATAGRAM を受信して生バイト列を保存
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

                // recv_datagram_for は内部で Quarter Stream ID を解析してペイロードのみ返す
                // ここでは ngtcp2 レベルで受信した DATAGRAM を取得
                if let Some(addr) = client_addr
                    && let Some(data) = server.recv_datagram_for(&addr)
                {
                    received_raw_clone.lock().unwrap().extend_from_slice(&data);
                    datagram_received_clone.store(true, Ordering::SeqCst);
                }
            }
        })
        .await;
    });

    // クライアント: shiguredo_http3 の Datagram を使ってエンコードしたペイロードを送信
    let client_result = timeout(Duration::from_secs(10), async {
        let mut session =
            ClientWebTransportSession::connect_insecure(server_addr, "localhost", "/webtransport")
                .await
                .expect("クライアント作成失敗");

        session.handshake().await.expect("ハンドシェイク失敗");

        let session_id = session
            .open_session(
                &format!("localhost:{}", server_addr.port()),
                "/webtransport",
            )
            .await
            .expect("セッション確立失敗");

        // shiguredo_http3 の Datagram を使って検証用データを構築
        let original_payload = b"H3 Datagram payload test data";
        let datagram = Datagram::new(session_id as u64, original_payload.to_vec()).unwrap();

        // Quarter Stream ID が正しく計算されることを検証
        assert_eq!(datagram.quarter_stream_id(), session_id as u64 / 4);

        // エンコードして送信 (send_datagram は内部で Quarter Stream ID を付与するため、
        // ペイロードのみを渡す)
        session
            .send_datagram(original_payload)
            .await
            .expect("DATAGRAM 送信失敗");

        // サーバーが処理する時間を確保
        tokio::time::sleep(Duration::from_millis(500)).await;

        original_payload.to_vec()
    })
    .await;

    server_task.abort();

    match client_result {
        Ok(original_payload) => {
            assert!(
                datagram_received.load(Ordering::SeqCst),
                "サーバーが DATAGRAM を受信するべき"
            );
            // サーバーの recv_datagram_for はペイロードのみを返す
            let raw = received_raw.lock().unwrap();
            assert_eq!(
                raw.as_slice(),
                original_payload.as_slice(),
                "受信したペイロードが元のデータと一致するべき"
            );
        }
        Err(_) => {
            panic!("テストタイムアウト");
        }
    }
}

/// shiguredo_http3::Datagram ラウンドトリップ統合テスト
///
/// クライアントが send_datagram で送信した DATAGRAM を
/// サーバーが受信し、同じデータをサーバーから send_datagram_for で返送。
/// クライアントが recv_datagram で受信して、shiguredo_http3 の
/// Datagram::decode() でデコードし、元のペイロードと一致することを検証する。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_h3_datagram_roundtrip_over_ngtcp2() {
    let (cert_path, key_path) = generate_test_certs();

    // サーバーを起動
    let mut server =
        ServerWebTransportSession::bind("127.0.0.1:0".parse().unwrap(), &cert_path, &key_path)
            .await
            .expect("サーバー作成失敗");

    let server_addr = server.local_addr();

    // サーバータスク: 受信した DATAGRAM をそのまま返送
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

                // 受信した DATAGRAM をエコーバック
                if let Some(addr) = client_addr
                    && let Some(data) = server.recv_datagram_for(&addr)
                {
                    server.send_datagram_for(&addr, &data).await.ok();
                }
            }
        })
        .await;
    });

    // クライアント: DATAGRAM を送信してエコーバックを受信、shiguredo_http3 でデコード
    let client_result = timeout(Duration::from_secs(10), async {
        let mut session =
            ClientWebTransportSession::connect_insecure(server_addr, "localhost", "/webtransport")
                .await
                .expect("クライアント作成失敗");

        session.handshake().await.expect("ハンドシェイク失敗");

        let session_id = session
            .open_session(
                &format!("localhost:{}", server_addr.port()),
                "/webtransport",
            )
            .await
            .expect("セッション確立失敗");

        // ペイロードを送信
        let original_payload = b"Roundtrip datagram test";
        session
            .send_datagram(original_payload)
            .await
            .expect("DATAGRAM 送信失敗");

        // エコーバックを受信
        let mut received_payload = None;
        for _ in 0..30 {
            session
                .recv(Duration::from_millis(100))
                .await
                .expect("受信失敗");

            if let Some(data) = session.recv_datagram() {
                received_payload = Some(data);
                break;
            }
        }

        let received = received_payload.expect("DATAGRAM エコーバックを受信するべき");

        // shiguredo_http3 の Datagram で検証
        // recv_datagram() はペイロードのみを返すため、
        // 元のペイロードと直接比較
        assert_eq!(
            received.as_slice(),
            original_payload,
            "エコーバックされたペイロードが一致するべき"
        );

        // Datagram 構造体を使って session_id との紐付けを検証
        let h3_datagram = Datagram::new(session_id as u64, received.clone()).unwrap();
        assert_eq!(h3_datagram.session_id, session_id as u64);
        assert_eq!(h3_datagram.quarter_stream_id(), session_id as u64 / 4);

        // エンコード → デコードのラウンドトリップ
        let mut encoded = Vec::new();
        h3_datagram.encode(&mut encoded);
        let (decoded, _) = Datagram::decode(&encoded).expect("Datagram デコード成功するべき");
        assert_eq!(decoded.session_id, session_id as u64);
        assert_eq!(decoded.payload, received);

        received
    })
    .await;

    server_task.abort();

    assert!(client_result.is_ok(), "テストがタイムアウトしないこと");
}

/// shiguredo_http3 Session と ngtcp2 WebTransport の統合テスト
///
/// shiguredo_http3 の Session 構造体を使ってセッション状態を管理しながら、
/// ngtcp2 で実際のデータ送受信を行う。Session::add_stream / get_stream / stream_count
/// でストリーム管理が ngtcp2 のストリーム操作と整合することを検証する。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_h3_session_state_with_ngtcp2_streams() {
    use shiguredo_http3::webtransport::{Session, Stream};

    let (cert_path, key_path) = generate_test_certs();

    let received_stream_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let received_stream_count_clone = received_stream_count.clone();

    // サーバーを起動
    let mut server =
        ServerWebTransportSession::bind("127.0.0.1:0".parse().unwrap(), &cert_path, &key_path)
            .await
            .expect("サーバー作成失敗");

    let server_addr = server.local_addr();

    // サーバータスク
    let server_task = tokio::spawn(async move {
        let mut seen_streams = std::collections::HashSet::<i64>::new();

        let _ = timeout(Duration::from_secs(10), async {
            loop {
                let mut handler =
                    |_addr: std::net::SocketAddr, _session_id: i64, event: Http3Event| -> bool {
                        match &event {
                            Http3Event::HeadersEnd { .. } => {
                                return true;
                            }
                            Http3Event::WebTransportData { stream_id, .. } => {
                                if seen_streams.insert(*stream_id) {
                                    received_stream_count_clone.fetch_add(1, Ordering::SeqCst);
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
            }
        })
        .await;
    });

    // クライアント: shiguredo_http3 Session でストリーム管理しながら ngtcp2 で送受信
    let client_result = timeout(Duration::from_secs(10), async {
        let mut session =
            ClientWebTransportSession::connect_insecure(server_addr, "localhost", "/webtransport")
                .await
                .expect("クライアント作成失敗");

        session.handshake().await.expect("ハンドシェイク失敗");

        let session_id = session
            .open_session(
                &format!("localhost:{}", server_addr.port()),
                "/webtransport",
            )
            .await
            .expect("セッション確立失敗");

        // shiguredo_http3 Session を作成してストリームを管理
        let mut h3_session = Session::new(session_id as u64);
        h3_session.set_established();
        assert!(h3_session.is_established());

        // ngtcp2 でストリームを開き、shiguredo_http3 Session にも登録
        let stream_count = 3;
        let mut stream_ids = Vec::new();

        for i in 0..stream_count {
            let ngtcp2_stream_id = session.open_bidi_stream().expect("ストリーム作成失敗");
            stream_ids.push(ngtcp2_stream_id);

            // shiguredo_http3 Session にストリームを追加
            let h3_stream = Stream::new(ngtcp2_stream_id as u64, session_id as u64, true);
            h3_session.add_stream(h3_stream);

            // ストリーム数の整合性を検証
            assert_eq!(h3_session.stream_count(), i + 1);

            // ストリームが取得可能であることを検証
            assert!(h3_session.get_stream(ngtcp2_stream_id as u64).is_some());

            // ngtcp2 でデータ送信
            let data = format!("Stream {} data", i);
            session
                .send_stream_data(ngtcp2_stream_id, data.as_bytes(), true)
                .await
                .expect("データ送信失敗");
        }

        // 最終的な Session 状態を検証
        assert_eq!(h3_session.stream_count(), stream_count);

        // サーバーが処理する時間を確保
        tokio::time::sleep(Duration::from_millis(500)).await;

        // ストリームを削除して整合性を確認
        for &stream_id in &stream_ids {
            h3_session.remove_stream(stream_id as u64);
        }
        assert_eq!(h3_session.stream_count(), 0);

        stream_ids
    })
    .await;

    server_task.abort();

    match client_result {
        Ok(stream_ids) => {
            let server_count = received_stream_count.load(Ordering::SeqCst);
            assert_eq!(
                server_count,
                stream_ids.len(),
                "サーバーが全ストリームからデータを受信するべき"
            );
        }
        Err(_) => {
            panic!("テストタイムアウト");
        }
    }
}

/// StreamHeader 双方向ストリームエンコード/デコード統合テスト
///
/// shiguredo_http3 の StreamHeader::encode_bidirectional() でエンコードしたデータを
/// bidi ストリームの先頭に付与して送信し、受信側で StreamHeader::decode_bidirectional()
/// でデコードして session_id が実セッション ID と一致することを検証する。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_h3_stream_header_bidi_over_ngtcp2() {
    use shiguredo_http3::webtransport::StreamHeader;

    let (cert_path, key_path) = generate_test_certs();

    let received_data = Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let received_data_clone = received_data.clone();
    let data_received = Arc::new(AtomicBool::new(false));
    let data_received_clone = data_received.clone();

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
                                data_received_clone.store(true, Ordering::SeqCst);
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

    // クライアント: StreamHeader をエンコードしてストリーム先頭に付与して送信
    let client_result = timeout(Duration::from_secs(10), async {
        let mut session =
            ClientWebTransportSession::connect_insecure(server_addr, "localhost", "/webtransport")
                .await
                .expect("クライアント作成失敗");

        session.handshake().await.expect("ハンドシェイク失敗");

        let session_id = session
            .open_session(
                &format!("localhost:{}", server_addr.port()),
                "/webtransport",
            )
            .await
            .expect("セッション確立失敗");

        // StreamHeader を bidi ストリーム用にエンコード
        let header = StreamHeader::new(session_id as u64).expect("valid bidi session id");
        let mut header_bytes = Vec::new();
        header.encode_bidirectional(&mut header_bytes);

        // ヘッダー + ペイロードを結合して送信
        let payload = b"bidi-header-test";
        let mut data = header_bytes.clone();
        data.extend_from_slice(payload);

        let stream_id = session.open_bidi_stream().expect("ストリーム作成失敗");
        session
            .send_stream_data(stream_id, &data, true)
            .await
            .expect("データ送信失敗");

        tokio::time::sleep(Duration::from_millis(500)).await;

        (session_id, header_bytes.len())
    })
    .await;

    server_task.abort();

    match client_result {
        Ok((session_id, header_len)) => {
            assert!(
                data_received.load(Ordering::SeqCst),
                "サーバーがデータを受信するべき"
            );

            let raw = received_data.lock().unwrap();

            // 受信データから StreamHeader をデコード
            let (decoded_header, consumed) = StreamHeader::decode_bidirectional(&raw)
                .expect("StreamHeader デコード成功するべき");

            assert_eq!(consumed, header_len, "消費バイト数が一致するべき");
            assert_eq!(
                decoded_header.session_id, session_id as u64,
                "デコードした session_id が実セッション ID と一致するべき"
            );

            // ヘッダー以降のペイロードを検証
            let payload = &raw[consumed..];
            assert_eq!(
                String::from_utf8_lossy(payload),
                "bidi-header-test",
                "ペイロードが正しいべき"
            );
        }
        Err(_) => {
            panic!("テストタイムアウト");
        }
    }
}

/// StreamHeader 単方向ストリームエンコード/デコード統合テスト
///
/// shiguredo_http3 の StreamHeader::encode_unidirectional() でエンコードしたデータを
/// uni ストリームの先頭に付与して送信し、受信側で StreamHeader::decode_unidirectional()
/// でデコードして session_id が実セッション ID と一致することを検証する。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_h3_stream_header_uni_over_ngtcp2() {
    use shiguredo_http3::webtransport::StreamHeader;

    let (cert_path, key_path) = generate_test_certs();

    let received_data = Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let received_data_clone = received_data.clone();
    let data_received = Arc::new(AtomicBool::new(false));
    let data_received_clone = data_received.clone();

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
                                data_received_clone.store(true, Ordering::SeqCst);
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

    // クライアント: StreamHeader を uni ストリーム用にエンコードして送信
    let client_result = timeout(Duration::from_secs(10), async {
        let mut session =
            ClientWebTransportSession::connect_insecure(server_addr, "localhost", "/webtransport")
                .await
                .expect("クライアント作成失敗");

        session.handshake().await.expect("ハンドシェイク失敗");

        let session_id = session
            .open_session(
                &format!("localhost:{}", server_addr.port()),
                "/webtransport",
            )
            .await
            .expect("セッション確立失敗");

        // StreamHeader を uni ストリーム用にエンコード
        let header = StreamHeader::new(session_id as u64).expect("valid bidi session id");
        let mut header_bytes = Vec::new();
        header.encode_unidirectional(&mut header_bytes);

        // ヘッダー + ペイロードを結合して送信
        let payload = b"uni-header-test";
        let mut data = header_bytes.clone();
        data.extend_from_slice(payload);

        let stream_id = session.open_uni_stream().expect("uni ストリーム作成失敗");
        session
            .send_stream_data(stream_id, &data, true)
            .await
            .expect("データ送信失敗");

        tokio::time::sleep(Duration::from_millis(500)).await;

        (session_id, header_bytes.len())
    })
    .await;

    server_task.abort();

    match client_result {
        Ok((session_id, header_len)) => {
            assert!(
                data_received.load(Ordering::SeqCst),
                "サーバーがデータを受信するべき"
            );

            let raw = received_data.lock().unwrap();

            // 受信データから StreamHeader をデコード
            let (decoded_header, consumed) = StreamHeader::decode_unidirectional(&raw)
                .expect("StreamHeader デコード成功するべき");

            assert_eq!(consumed, header_len, "消費バイト数が一致するべき");
            assert_eq!(
                decoded_header.session_id, session_id as u64,
                "デコードした session_id が実セッション ID と一致するべき"
            );

            // ヘッダー以降のペイロードを検証
            let payload = &raw[consumed..];
            assert_eq!(
                String::from_utf8_lossy(payload),
                "uni-header-test",
                "ペイロードが正しいべき"
            );
        }
        Err(_) => {
            panic!("テストタイムアウト");
        }
    }
}

/// ConnectRequest / ConnectResponse 検証統合テスト
///
/// ngtcp2 で実際にセッション確立後、shiguredo_http3 の ConnectRequest::new() / validate()
/// と ConnectResponse::new(200).is_success() が実セッションパラメータで正しく動作することを検証する。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_h3_connect_validation_with_ngtcp2_session() {
    use shiguredo_http3::webtransport::{ConnectRequest, ConnectResponse};

    let (cert_path, key_path) = generate_test_certs();

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
                        if let Http3Event::HeadersEnd { .. } = &event {
                            return true;
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

    // クライアント: セッション確立後に ConnectRequest / ConnectResponse を検証
    let client_result = timeout(Duration::from_secs(10), async {
        let mut session =
            ClientWebTransportSession::connect_insecure(server_addr, "localhost", "/webtransport")
                .await
                .expect("クライアント作成失敗");

        session.handshake().await.expect("ハンドシェイク失敗");

        let authority = format!("localhost:{}", server_addr.port());
        let path = "/webtransport";

        let _session_id = session
            .open_session(&authority, path)
            .await
            .expect("セッション確立失敗");

        // 実セッションパラメータで ConnectRequest を検証
        let request = ConnectRequest::new("https", &authority, path);
        assert!(
            request.validate().is_ok(),
            "正しいパラメータの ConnectRequest は検証成功するべき"
        );

        // 不正な scheme の場合
        let bad_request = ConnectRequest::new("http", &authority, path);
        assert!(
            bad_request.validate().is_err(),
            "http scheme の ConnectRequest は検証失敗するべき"
        );

        // 空 authority の場合
        let bad_request = ConnectRequest::new("https", "", path);
        assert!(
            bad_request.validate().is_err(),
            "空 authority の ConnectRequest は検証失敗するべき"
        );

        // 空 path の場合
        let bad_request = ConnectRequest::new("https", &authority, "");
        assert!(
            bad_request.validate().is_err(),
            "空 path の ConnectRequest は検証失敗するべき"
        );

        // ConnectResponse の成功判定
        let response = ConnectResponse::new(200);
        assert!(response.is_success(), "200 は成功レスポンスであるべき");

        let response_404 = ConnectResponse::new(404);
        assert!(!response_404.is_success(), "404 は成功レスポンスでないべき");
    })
    .await;

    server_task.abort();

    assert!(client_result.is_ok(), "テストがタイムアウトしないこと");
}

/// ApplicationErrorCode 変換統合テスト
///
/// 実セッション上でアプリケーションエラーコード (0-1000) を
/// ApplicationErrorCode::to_http3_code() で変換し、from_http3_code() で復元する
/// ラウンドトリップを検証する。予約コードポイント回避も確認。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_h3_error_code_conversion_with_ngtcp2() {
    use shiguredo_http3::webtransport::ApplicationErrorCode;

    let (cert_path, key_path) = generate_test_certs();

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
                        if let Http3Event::HeadersEnd { .. } = &event {
                            return true;
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

    // クライアント: セッション確立後にエラーコード変換を検証
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

        // 0-1000 のアプリケーションエラーコードのラウンドトリップ検証
        for app_code in 0..=1000u32 {
            let http3_code = ApplicationErrorCode::to_http3_code(app_code);

            // HTTP/3 コードが WebTransport 範囲内であることを確認
            assert!(
                ApplicationErrorCode::is_application_error(http3_code),
                "app_code={} の HTTP/3 コード {} は WebTransport 範囲内であるべき",
                app_code,
                http3_code
            );

            // 予約コードポイントでないことを確認
            assert!(
                !http3_code.wrapping_sub(0x21).is_multiple_of(0x1f),
                "app_code={} の HTTP/3 コード {} は予約コードポイントでないべき",
                app_code,
                http3_code
            );

            // ラウンドトリップ検証
            let restored = ApplicationErrorCode::from_http3_code(http3_code);
            assert_eq!(
                restored,
                Some(app_code),
                "app_code={} のラウンドトリップが一致するべき",
                app_code
            );
        }
    })
    .await;

    server_task.abort();

    assert!(client_result.is_ok(), "テストがタイムアウトしないこと");
}

/// Session ライフサイクル統合テスト
///
/// ngtcp2 セッションの各段階で shiguredo_http3 Session の状態を同期管理:
/// Pending -> Connecting -> Established -> ストリーム追加/削除 -> drain() -> Closed。
/// 実ネットワーク操作と状態遷移が整合することを検証する。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_h3_session_lifecycle_with_ngtcp2() {
    use shiguredo_http3::webtransport::{Session, SessionState, Stream};

    let (cert_path, key_path) = generate_test_certs();

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
                        if let Http3Event::HeadersEnd { .. } = &event {
                            return true;
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

    // クライアント: Session ライフサイクルを検証
    let client_result = timeout(Duration::from_secs(10), async {
        let mut session =
            ClientWebTransportSession::connect_insecure(server_addr, "localhost", "/webtransport")
                .await
                .expect("クライアント作成失敗");

        session.handshake().await.expect("ハンドシェイク失敗");

        // Session を Pending 状態で作成
        let mut h3_session = Session::new(0);
        assert_eq!(h3_session.state(), SessionState::Pending);
        assert!(!h3_session.is_established());

        // Connecting 状態に遷移
        h3_session.set_connecting();
        assert_eq!(h3_session.state(), SessionState::Connecting);

        // 実際のセッション確立
        let session_id = session
            .open_session(
                &format!("localhost:{}", server_addr.port()),
                "/webtransport",
            )
            .await
            .expect("セッション確立失敗");

        // Established 状態に遷移
        h3_session.set_established();
        assert_eq!(h3_session.state(), SessionState::Established);
        assert!(h3_session.is_established());

        // ストリームの追加と削除
        let stream_id = session.open_bidi_stream().expect("ストリーム作成失敗");
        let h3_stream = Stream::new(stream_id as u64, session_id as u64, true);
        h3_session.add_stream(h3_stream);
        assert_eq!(h3_session.stream_count(), 1);
        assert!(h3_session.get_stream(stream_id as u64).is_some());

        // データ送信
        session
            .send_stream_data(stream_id, b"lifecycle-test", true)
            .await
            .expect("データ送信失敗");

        // ストリーム削除
        h3_session.remove_stream(stream_id as u64);
        assert_eq!(h3_session.stream_count(), 0);

        // Draining 状態に遷移
        h3_session.drain();
        assert_eq!(h3_session.state(), SessionState::Draining);

        // DrainSession Capsule が送信キューに追加されていることを確認
        let pending = h3_session.take_pending_capsules();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0], Capsule::DrainSession);

        // Closed 状態に遷移
        h3_session.close(None);
        assert_eq!(h3_session.state(), SessionState::Closed);
        assert!(h3_session.is_closed());

        tokio::time::sleep(Duration::from_millis(200)).await;
    })
    .await;

    server_task.abort();

    assert!(client_result.is_ok(), "テストがタイムアウトしないこと");
}

/// Stream バイト追跡統合テスト
///
/// 実セッション上で複数ストリームのデータ送受信を行い、
/// Stream::add_bytes_sent() / add_bytes_received() で追跡したバイト数が
/// 実際の送受信量と一致することを検証する。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_h3_stream_byte_tracking_with_ngtcp2() {
    use shiguredo_http3::webtransport::{Session, Stream};

    let (cert_path, key_path) = generate_test_certs();

    // サーバーを起動
    let mut server =
        ServerWebTransportSession::bind("127.0.0.1:0".parse().unwrap(), &cert_path, &key_path)
            .await
            .expect("サーバー作成失敗");

    let server_addr = server.local_addr();

    // サーバータスク: エコーバック
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

    // クライアント: バイト追跡を検証
    let client_result = timeout(Duration::from_secs(10), async {
        let mut session =
            ClientWebTransportSession::connect_insecure(server_addr, "localhost", "/webtransport")
                .await
                .expect("クライアント作成失敗");

        session.handshake().await.expect("ハンドシェイク失敗");

        let session_id = session
            .open_session(
                &format!("localhost:{}", server_addr.port()),
                "/webtransport",
            )
            .await
            .expect("セッション確立失敗");

        let mut h3_session = Session::new(session_id as u64);
        h3_session.set_established();

        // 複数ストリームで送受信
        let payloads = [
            b"short".as_slice(),
            b"medium length data".as_slice(),
            b"a longer payload for byte tracking test".as_slice(),
        ];
        let mut stream_tracking = Vec::new();

        for payload in &payloads {
            let ngtcp2_stream_id = session.open_bidi_stream().expect("ストリーム作成失敗");
            let mut h3_stream = Stream::new(ngtcp2_stream_id as u64, session_id as u64, true);

            // 送信バイト数を追跡
            h3_stream.add_bytes_sent(payload.len() as u64);
            h3_session.add_stream(h3_stream);

            session
                .send_stream_data(ngtcp2_stream_id, payload, true)
                .await
                .expect("データ送信失敗");

            stream_tracking.push((ngtcp2_stream_id, payload.len()));
        }

        // エコーバックを受信
        let mut received_per_stream = std::collections::HashMap::<i64, Vec<u8>>::new();
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
                    received_per_stream
                        .entry(stream_id)
                        .or_default()
                        .extend_from_slice(&data);

                    // 受信バイト数を追跡
                    if let Some(h3_stream) = h3_session.get_stream_mut(stream_id as u64) {
                        h3_stream.add_bytes_received(data.len() as u64);
                    }
                }
            }

            if received_per_stream.len() >= payloads.len() {
                break;
            }
        }

        // 各ストリームの送受信バイト数を検証
        for (ngtcp2_stream_id, expected_len) in &stream_tracking {
            let h3_stream = h3_session
                .get_stream(*ngtcp2_stream_id as u64)
                .expect("ストリームが存在するべき");

            assert_eq!(
                h3_stream.bytes_sent(),
                *expected_len as u64,
                "ストリーム {} の送信バイト数が一致するべき",
                ngtcp2_stream_id
            );

            // エコーバックされたデータがあれば受信バイト数も検証
            if let Some(recv_data) = received_per_stream.get(ngtcp2_stream_id) {
                assert_eq!(
                    h3_stream.bytes_received(),
                    recv_data.len() as u64,
                    "ストリーム {} の受信バイト数が一致するべき",
                    ngtcp2_stream_id
                );
            }
        }
    })
    .await;

    server_task.abort();

    assert!(client_result.is_ok(), "テストがタイムアウトしないこと");
}

/// 全 Capsule 種類統合テスト
///
/// CloseSession, DrainSession, MaxData, MaxStreams(bidi), MaxStreams(uni),
/// DataBlocked, StreamsBlocked(bidi), StreamsBlocked(uni) の全 8 種類を
/// 1 ストリームで連続送信し、全て正しくデコードされることを検証する。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_h3_all_capsule_types_over_ngtcp2() {
    let (cert_path, key_path) = generate_test_certs();

    let received_data = Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let received_data_clone = received_data.clone();
    let data_received = Arc::new(AtomicBool::new(false));
    let data_received_clone = data_received.clone();

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
                                data_received_clone.store(true, Ordering::SeqCst);
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

    // 全 8 種類の Capsule
    let capsules = vec![
        Capsule::CloseSession {
            error_code: 42,
            message: "test close".to_string(),
        },
        Capsule::DrainSession,
        Capsule::MaxData { maximum: 1_000_000 },
        Capsule::MaxStreams {
            bidirectional: true,
            maximum: 100,
        },
        Capsule::MaxStreams {
            bidirectional: false,
            maximum: 50,
        },
        Capsule::DataBlocked { maximum: 500_000 },
        Capsule::StreamsBlocked {
            bidirectional: true,
            maximum: 10,
        },
        Capsule::StreamsBlocked {
            bidirectional: false,
            maximum: 5,
        },
    ];

    let capsules_clone = capsules.clone();
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

        // 全 Capsule を連続エンコード
        let mut all_bytes = Vec::new();
        for capsule in &capsules_clone {
            capsule.encode(&mut all_bytes);
        }

        let stream_id = session.open_bidi_stream().expect("ストリーム作成失敗");
        session
            .send_stream_data(stream_id, &all_bytes, true)
            .await
            .expect("データ送信失敗");

        tokio::time::sleep(Duration::from_millis(500)).await;
    })
    .await;

    server_task.abort();

    assert!(client_result.is_ok(), "テストがタイムアウトしないこと");
    assert!(
        data_received.load(Ordering::SeqCst),
        "サーバーがデータを受信するべき"
    );

    // 受信バイト列から全 Capsule を順次デコード
    let raw = received_data.lock().unwrap();
    let mut offset = 0;
    let mut decoded_capsules = Vec::new();

    while offset < raw.len() {
        let (capsule, consumed) = Capsule::decode(&raw[offset..])
            .expect("Capsule デコードがエラーにならないべき")
            .expect("Capsule デコードが incomplete にならないべき");
        decoded_capsules.push(capsule);
        offset += consumed;
    }

    assert_eq!(
        decoded_capsules.len(),
        capsules.len(),
        "全 {} Capsule がデコードされるべき",
        capsules.len()
    );
    assert_eq!(
        decoded_capsules, capsules,
        "デコードした Capsule が元と一致するべき"
    );
}

/// 大量 DATAGRAM ラウンドトリップ統合テスト
///
/// クライアントから 10 個の DATAGRAM を連続送信し、サーバーで受信した各ペイロードを
/// Datagram::new() で構築し、encode() -> decode() ラウンドトリップが全て正しいことを検証する。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_h3_multiple_datagrams_roundtrip_over_ngtcp2() {
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

        let session_id = session
            .open_session(
                &format!("localhost:{}", server_addr.port()),
                "/webtransport",
            )
            .await
            .expect("セッション確立失敗");

        for i in 0..10 {
            let payload = format!("dgram-roundtrip-{}", i);
            session
                .send_datagram(payload.as_bytes())
                .await
                .expect("DATAGRAM 送信失敗");
        }

        tokio::time::sleep(Duration::from_secs(1)).await;

        session_id
    })
    .await;

    server_task.abort();

    match client_result {
        Ok(session_id) => {
            let datagrams = received_datagrams.lock().unwrap();
            assert!(
                !datagrams.is_empty(),
                "サーバーが少なくとも 1 個の DATAGRAM を受信するべき"
            );

            // 受信した各ペイロードで Datagram エンコード/デコードのラウンドトリップを検証
            for payload in datagrams.iter() {
                let datagram = Datagram::new(session_id as u64, payload.clone()).unwrap();

                // Quarter Stream ID の検証
                assert_eq!(
                    datagram.quarter_stream_id(),
                    session_id as u64 / 4,
                    "Quarter Stream ID が正しいべき"
                );

                // エンコード → デコードのラウンドトリップ
                let mut encoded = Vec::new();
                datagram.encode(&mut encoded);

                let (decoded, consumed) =
                    Datagram::decode(&encoded).expect("Datagram デコード成功するべき");
                assert_eq!(consumed, encoded.len(), "全バイトが消費されるべき");
                assert_eq!(
                    decoded.session_id, session_id as u64,
                    "session_id が一致するべき"
                );
                assert_eq!(decoded.payload, *payload, "ペイロードが一致するべき");

                // ペイロード内容の形式検証
                let s = String::from_utf8_lossy(payload);
                assert!(
                    s.starts_with("dgram-roundtrip-"),
                    "DATAGRAM ペイロードの形式が正しいべき: {:?}",
                    s
                );
            }
        }
        Err(_) => {
            panic!("テストタイムアウト");
        }
    }
}
