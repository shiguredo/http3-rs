//! QUIC Sans I/O テスト
//!
//! Connection の read_pkt / write_pkt を直接テストする。
//! ネットワーク I/O を介さずにパケット交換をシミュレートし、
//! ハンドシェイクの動作を検証する。

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use ngtcp2_sys::ngtcp2_transport_params;
use rcgen::{CertificateParams, KeyPair};
use shiguredo_ngtcp2::{Connection, ConnectionId, PacketInfo, TlsContext, TransportParamsExt};

/// テスト用の証明書と秘密鍵を動的に生成
fn generate_test_certs() -> (PathBuf, PathBuf) {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let unique_id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let temp_dir = std::env::temp_dir().join(format!(
        "quic_sans_io_test_{}_{}",
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

/// テスト用のタイムスタンプを取得 (ナノ秒)
fn timestamp() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64
}

/// クライアント接続を作成し、Initial パケットを生成する
#[test]
fn test_client_initial_packet_generation() {
    let dcid = ConnectionId::random(16).unwrap();
    let scid = ConnectionId::random(16).unwrap();
    let local_addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
    let remote_addr: SocketAddr = "127.0.0.1:4433".parse().unwrap();

    // TLS コンテキストを作成 (証明書検証なし)
    let tls_ctx =
        TlsContext::new_client_with_options(&[b"h3"], false).expect("TLS コンテキスト作成失敗");
    let tls_session = tls_ctx.create_session().expect("TLS セッション作成失敗");

    // トランスポートパラメータを設定
    let params = ngtcp2_transport_params::default_params().with_datagram(65535);

    let ts = timestamp();

    // クライアント接続を作成
    let mut client = Connection::client_new(
        &dcid,
        &scid,
        local_addr,
        remote_addr,
        "localhost",
        tls_session,
        &params,
        ts,
    )
    .expect("クライアント接続作成失敗");

    // Initial パケットを生成
    let mut buf = vec![0u8; 1350];
    let (written, _pkt_info) = client.write_pkt(&mut buf, ts).expect("パケット生成失敗");

    // パケットが生成されたことを確認
    assert!(written > 0, "Initial パケットが生成されるべき");
    eprintln!("Initial パケットサイズ: {} bytes", written);

    // QUIC パケットのヘッダーを確認
    // Long Header Format: 最初のビットが 1
    assert!(buf[0] & 0x80 != 0, "Long Header であるべき");

    // ハンドシェイクはまだ完了していない
    assert!(
        !client.is_handshake_completed(),
        "ハンドシェイクは未完了であるべき"
    );
}

/// サーバー接続を作成する
#[test]
fn test_server_connection_creation() {
    let (cert_path, key_path) = generate_test_certs();

    // クライアントの DCID をサーバーの SCID として使用
    let client_dcid = ConnectionId::random(16).unwrap();
    let server_scid = ConnectionId::random(16).unwrap();
    let local_addr: SocketAddr = "127.0.0.1:4433".parse().unwrap();
    let remote_addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();

    // TLS コンテキストを作成
    let tls_ctx =
        TlsContext::new_server(&cert_path, &key_path, &[b"h3"]).expect("TLS コンテキスト作成失敗");
    let tls_session = tls_ctx.create_session().expect("TLS セッション作成失敗");

    // トランスポートパラメータを設定
    let params = ngtcp2_transport_params::default_params()
        .with_datagram(65535)
        .with_original_dcid(&client_dcid);

    let ts = timestamp();

    // サーバー接続を作成
    let server = Connection::server_new(
        &client_dcid,
        &server_scid,
        local_addr,
        remote_addr,
        tls_session,
        &params,
        ts,
    )
    .expect("サーバー接続作成失敗");

    // ハンドシェイクはまだ完了していない
    assert!(
        !server.is_handshake_completed(),
        "ハンドシェイクは未完了であるべき"
    );
}

/// クライアント/サーバー間でパケットを交換してハンドシェイクを完了する
#[test]
fn test_quic_handshake() {
    let (cert_path, key_path) = generate_test_certs();

    // 接続 ID を生成
    let client_dcid = ConnectionId::random(16).unwrap();
    let client_scid = ConnectionId::random(16).unwrap();
    let server_scid = ConnectionId::random(16).unwrap();

    let client_addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
    let server_addr: SocketAddr = "127.0.0.1:4433".parse().unwrap();

    let ts = timestamp();

    // クライアント接続を作成
    let client_tls_ctx = TlsContext::new_client_with_options(&[b"h3"], false)
        .expect("クライアント TLS コンテキスト作成失敗");
    let client_tls_session = client_tls_ctx
        .create_session()
        .expect("クライアント TLS セッション作成失敗");

    let client_params = ngtcp2_transport_params::default_params().with_datagram(65535);

    let mut client = Connection::client_new(
        &client_dcid,
        &client_scid,
        client_addr,
        server_addr,
        "localhost",
        client_tls_session,
        &client_params,
        ts,
    )
    .expect("クライアント接続作成失敗");

    // サーバー TLS コンテキストを作成
    let server_tls_ctx = TlsContext::new_server(&cert_path, &key_path, &[b"h3"])
        .expect("サーバー TLS コンテキスト作成失敗");

    let pkt_info = PacketInfo::default();
    let mut buf = vec![0u8; 1350];
    let mut round = 0;
    const MAX_ROUNDS: usize = 10;

    // サーバー接続はまだ作成されていない (Initial パケット受信後に作成)
    let mut server: Option<Connection> = None;

    while round < MAX_ROUNDS {
        round += 1;
        let current_ts = timestamp();
        eprintln!("--- Round {} ---", round);

        // クライアントからパケットを生成
        let (client_written, _) = client
            .write_pkt(&mut buf, current_ts)
            .unwrap_or((0, pkt_info));
        if client_written > 0 {
            eprintln!("クライアント -> サーバー: {} bytes", client_written);

            // サーバーがまだ作成されていない場合は作成
            if server.is_none() {
                let server_tls_session = server_tls_ctx
                    .create_session()
                    .expect("サーバー TLS セッション作成失敗");

                let server_params = ngtcp2_transport_params::default_params()
                    .with_datagram(65535)
                    .with_original_dcid(&client_dcid);

                server = Some(
                    Connection::server_new(
                        &client_scid,
                        &server_scid,
                        server_addr,
                        client_addr,
                        server_tls_session,
                        &server_params,
                        current_ts,
                    )
                    .expect("サーバー接続作成失敗"),
                );
            }

            // サーバーがパケットを処理
            if let Some(ref mut s) = server {
                let result = s.read_pkt(
                    &server_addr,
                    &client_addr,
                    &pkt_info,
                    &buf[..client_written],
                    current_ts,
                );
                match result {
                    Ok(()) => eprintln!("サーバー: パケット処理成功"),
                    Err(e) => {
                        eprintln!("サーバー: パケット処理エラー: {:?}", e);
                        // エラーコードを確認
                        if format!("{:?}", e).contains("-225") {
                            eprintln!("ERR_TRANSPORT_PARAM (-225) が発生");
                            eprintln!("トランスポートパラメータの不一致の可能性");
                        }
                    }
                }
            }
        }

        // サーバーからパケットを生成
        if let Some(ref mut s) = server {
            let (server_written, _) = s.write_pkt(&mut buf, current_ts).unwrap_or((0, pkt_info));
            if server_written > 0 {
                eprintln!("サーバー -> クライアント: {} bytes", server_written);

                // クライアントがパケットを処理
                let result = client.read_pkt(
                    &client_addr,
                    &server_addr,
                    &pkt_info,
                    &buf[..server_written],
                    current_ts,
                );
                match result {
                    Ok(()) => eprintln!("クライアント: パケット処理成功"),
                    Err(e) => eprintln!("クライアント: パケット処理エラー: {:?}", e),
                }
            }
        }

        // ハンドシェイク完了を確認
        let client_done = client.is_handshake_completed();
        let server_done = server.as_ref().is_some_and(|s| s.is_handshake_completed());

        eprintln!(
            "ハンドシェイク状態: クライアント={}, サーバー={}",
            client_done, server_done
        );

        if client_done && server_done {
            eprintln!("ハンドシェイク完了!");
            break;
        }
    }

    // ハンドシェイクが完了したことを確認
    assert!(
        client.is_handshake_completed(),
        "クライアントのハンドシェイクが完了するべき"
    );
    assert!(
        server.as_ref().is_some_and(|s| s.is_handshake_completed()),
        "サーバーのハンドシェイクが完了するべき"
    );
}

/// ストリームオープンと接続状態の確認テスト
#[test]
fn test_stream_open_after_handshake() {
    let (cert_path, key_path) = generate_test_certs();

    // 接続 ID を生成
    let client_dcid = ConnectionId::random(16).unwrap();
    let client_scid = ConnectionId::random(16).unwrap();
    let server_scid = ConnectionId::random(16).unwrap();

    let client_addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
    let server_addr: SocketAddr = "127.0.0.1:4433".parse().unwrap();

    let ts = timestamp();

    // クライアント接続を作成
    let client_tls_ctx = TlsContext::new_client_with_options(&[b"h3"], false)
        .expect("クライアント TLS コンテキスト作成失敗");
    let client_tls_session = client_tls_ctx
        .create_session()
        .expect("クライアント TLS セッション作成失敗");

    let client_params = ngtcp2_transport_params::default_params().with_datagram(65535);

    let mut client = Connection::client_new(
        &client_dcid,
        &client_scid,
        client_addr,
        server_addr,
        "localhost",
        client_tls_session,
        &client_params,
        ts,
    )
    .expect("クライアント接続作成失敗");

    // サーバー TLS コンテキストを作成
    let server_tls_ctx = TlsContext::new_server(&cert_path, &key_path, &[b"h3"])
        .expect("サーバー TLS コンテキスト作成失敗");

    let pkt_info = PacketInfo::default();
    let mut buf = vec![0u8; 1350];
    let mut server: Option<Connection> = None;

    // まずハンドシェイクを完了させる
    for _round in 0..10 {
        let current_ts = timestamp();

        // クライアントからパケットを生成
        let (client_written, _) = client
            .write_pkt(&mut buf, current_ts)
            .unwrap_or((0, pkt_info));
        if client_written > 0 {
            if server.is_none() {
                let server_tls_session = server_tls_ctx
                    .create_session()
                    .expect("サーバー TLS セッション作成失敗");

                let server_params = ngtcp2_transport_params::default_params()
                    .with_datagram(65535)
                    .with_original_dcid(&client_dcid);

                server = Some(
                    Connection::server_new(
                        &client_scid,
                        &server_scid,
                        server_addr,
                        client_addr,
                        server_tls_session,
                        &server_params,
                        current_ts,
                    )
                    .expect("サーバー接続作成失敗"),
                );
            }

            if let Some(ref mut s) = server {
                let _ = s.read_pkt(
                    &server_addr,
                    &client_addr,
                    &pkt_info,
                    &buf[..client_written],
                    current_ts,
                );
            }
        }

        if let Some(ref mut s) = server {
            let (server_written, _) = s.write_pkt(&mut buf, current_ts).unwrap_or((0, pkt_info));
            if server_written > 0 {
                let _ = client.read_pkt(
                    &client_addr,
                    &server_addr,
                    &pkt_info,
                    &buf[..server_written],
                    current_ts,
                );
            }
        }

        if client.is_handshake_completed()
            && server.as_ref().is_some_and(|s| s.is_handshake_completed())
        {
            break;
        }
    }

    // ハンドシェイクが完了していることを確認
    assert!(
        client.is_handshake_completed(),
        "クライアントのハンドシェイクが完了するべき"
    );
    assert!(
        server.as_ref().is_some_and(|s| s.is_handshake_completed()),
        "サーバーのハンドシェイクが完了するべき"
    );

    eprintln!("ハンドシェイク完了");

    // クライアントから双方向ストリームを開く
    let stream_id = client.open_bidi_stream().expect("ストリームオープン失敗");
    eprintln!("双方向ストリーム ID: {}", stream_id);
    assert_eq!(stream_id, 0, "最初の双方向ストリームは ID 0");

    // 単方向ストリームを開く
    let uni_stream_id = client
        .open_uni_stream()
        .expect("単方向ストリームオープン失敗");
    eprintln!("単方向ストリーム ID: {}", uni_stream_id);
    assert_eq!(uni_stream_id, 2, "最初の単方向ストリームは ID 2");

    // 残りのストリーム数を確認
    let bidi_left = client.get_streams_bidi_left();
    let uni_left = client.get_streams_uni_left();
    eprintln!(
        "残り双方向ストリーム: {}, 残り単方向ストリーム: {}",
        bidi_left, uni_left
    );
    assert!(bidi_left > 0, "双方向ストリームがまだ開ける");
    assert!(uni_left > 0, "単方向ストリームがまだ開ける");
}

/// 接続状態の確認テスト
#[test]
fn test_connection_state() {
    let (cert_path, key_path) = generate_test_certs();

    // 接続 ID を生成
    let client_dcid = ConnectionId::random(16).unwrap();
    let client_scid = ConnectionId::random(16).unwrap();
    let server_scid = ConnectionId::random(16).unwrap();

    let client_addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
    let server_addr: SocketAddr = "127.0.0.1:4433".parse().unwrap();

    let ts = timestamp();

    // クライアント接続を作成
    let client_tls_ctx = TlsContext::new_client_with_options(&[b"h3"], false)
        .expect("クライアント TLS コンテキスト作成失敗");
    let client_tls_session = client_tls_ctx
        .create_session()
        .expect("クライアント TLS セッション作成失敗");

    let client_params = ngtcp2_transport_params::default_params().with_datagram(65535);

    let mut client = Connection::client_new(
        &client_dcid,
        &client_scid,
        client_addr,
        server_addr,
        "localhost",
        client_tls_session,
        &client_params,
        ts,
    )
    .expect("クライアント接続作成失敗");

    // サーバー TLS コンテキストを作成
    let server_tls_ctx = TlsContext::new_server(&cert_path, &key_path, &[b"h3"])
        .expect("サーバー TLS コンテキスト作成失敗");

    let pkt_info = PacketInfo::default();
    let mut buf = vec![0u8; 1350];
    let mut server: Option<Connection> = None;

    // まずハンドシェイクを完了させる
    for _round in 0..10 {
        let current_ts = timestamp();

        let (client_written, _) = client
            .write_pkt(&mut buf, current_ts)
            .unwrap_or((0, pkt_info));
        if client_written > 0 {
            if server.is_none() {
                let server_tls_session = server_tls_ctx
                    .create_session()
                    .expect("サーバー TLS セッション作成失敗");

                let server_params = ngtcp2_transport_params::default_params()
                    .with_datagram(65535)
                    .with_original_dcid(&client_dcid);

                server = Some(
                    Connection::server_new(
                        &client_scid,
                        &server_scid,
                        server_addr,
                        client_addr,
                        server_tls_session,
                        &server_params,
                        current_ts,
                    )
                    .expect("サーバー接続作成失敗"),
                );
            }

            if let Some(ref mut s) = server {
                let _ = s.read_pkt(
                    &server_addr,
                    &client_addr,
                    &pkt_info,
                    &buf[..client_written],
                    current_ts,
                );
            }
        }

        if let Some(ref mut s) = server {
            let (server_written, _) = s.write_pkt(&mut buf, current_ts).unwrap_or((0, pkt_info));
            if server_written > 0 {
                let _ = client.read_pkt(
                    &client_addr,
                    &server_addr,
                    &pkt_info,
                    &buf[..server_written],
                    current_ts,
                );
            }
        }

        if client.is_handshake_completed()
            && server.as_ref().is_some_and(|s| s.is_handshake_completed())
        {
            break;
        }
    }

    // ハンドシェイクが完了していることを確認
    assert!(
        client.is_handshake_completed(),
        "クライアントのハンドシェイクが完了するべき"
    );

    // 接続状態を確認
    assert!(!client.is_in_closing_period(), "クローズ中ではないべき");
    assert!(!client.is_in_draining_period(), "ドレイン中ではないべき");

    // 接続のリソース確認
    let max_data_left = client.get_max_data_left();
    eprintln!("max_data_left: {}", max_data_left);
    assert!(max_data_left > 0, "データ送信可能量があるべき");

    // DATAGRAM キューの確認
    assert!(!client.has_datagram(), "初期状態ではDATAGRAMがないべき");
    assert!(client.poll_datagram().is_none(), "DATAGRAMがないべき");

    // ストリームデータキューの確認
    assert!(
        !client.has_stream_data(),
        "初期状態ではストリームデータがないべき"
    );
    assert!(
        client.poll_stream_data().is_none(),
        "ストリームデータがないべき"
    );

    eprintln!("接続状態テスト完了");
}
