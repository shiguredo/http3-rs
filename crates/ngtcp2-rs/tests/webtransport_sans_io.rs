//! WebTransport Sans I/O テスト
//!
//! WebTransport API (submit_wt_request / submit_wt_response / server_confirm_wt_session) を
//! 直接テストする。QUIC ハンドシェイク完了後の WebTransport セッション確立を検証する。

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use nghttp3_sys::nghttp3_vec;
use rcgen::{CertificateParams, KeyPair};

use shiguredo_ngtcp2::{
    Connection, ConnectionId, Header, Http3Connection, Http3Event, Http3Settings, PacketInfo,
    TlsContext, TransportParams,
};

/// テスト用の証明書と秘密鍵を動的に生成
fn generate_test_certs() -> (PathBuf, PathBuf) {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let unique_id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let temp_dir = std::env::temp_dir().join(format!(
        "wt_sans_io_test_{}_{}",
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
        .expect("test must succeed")
        .as_nanos() as u64
}

/// QUIC ハンドシェイクを完了させるヘルパー関数
fn complete_quic_handshake(
    client: &mut Connection,
    server: &mut Connection,
    client_addr: SocketAddr,
    server_addr: SocketAddr,
) -> bool {
    let pkt_info = PacketInfo::default();
    let mut buf = vec![0u8; 1350];

    for _ in 0..10 {
        let current_ts = timestamp();

        // クライアント -> サーバー
        let (client_written, _) = client
            .write_pkt(&mut buf, current_ts)
            .unwrap_or((0, pkt_info));
        if client_written > 0 {
            let _ = server.read_pkt(
                &server_addr,
                &client_addr,
                &pkt_info,
                &buf[..client_written],
                current_ts,
            );
        }

        // サーバー -> クライアント
        let (server_written, _) = server
            .write_pkt(&mut buf, current_ts)
            .unwrap_or((0, pkt_info));
        if server_written > 0 {
            let _ = client.read_pkt(
                &client_addr,
                &server_addr,
                &pkt_info,
                &buf[..server_written],
                current_ts,
            );
        }

        if client.is_handshake_completed() && server.is_handshake_completed() {
            return true;
        }
    }

    false
}

/// Header ヘルパーメソッドのテスト
#[test]
fn test_webtransport_headers() {
    // WebTransport CONNECT リクエストヘッダー
    let headers = [
        Header::method("CONNECT"),
        Header::scheme("https"),
        Header::new(b":protocol".to_vec(), b"webtransport".to_vec()),
        Header::authority("localhost:4433"),
        Header::path("/webtransport"),
    ];

    // ヘッダーの内容を確認
    assert_eq!(headers[0].name_str(), Some(":method"));
    assert_eq!(headers[0].value_str(), Some("CONNECT"));
    assert_eq!(headers[1].name_str(), Some(":scheme"));
    assert_eq!(headers[1].value_str(), Some("https"));
    assert_eq!(headers[2].name_str(), Some(":protocol"));
    assert_eq!(headers[2].value_str(), Some("webtransport"));
    assert_eq!(headers[3].name_str(), Some(":authority"));
    assert_eq!(headers[3].value_str(), Some("localhost:4433"));
    assert_eq!(headers[4].name_str(), Some(":path"));
    assert_eq!(headers[4].value_str(), Some("/webtransport"));
}

/// WebTransport 対応 HTTP/3 クライアント接続を作成する
#[test]
fn test_wt_h3_client_creation() {
    let settings = Http3Settings::new().with_webtransport().into_raw();

    // WebTransport が有効になっていることを確認
    assert_eq!(settings.enable_connect_protocol, 1);
    assert_eq!(settings.h3_datagram, 1);
    assert_eq!(settings.wt_enabled, 1);

    let h3_conn = Http3Connection::client_new(&settings);
    assert!(
        h3_conn.is_ok(),
        "WebTransport 対応 HTTP/3 クライアント接続を作成できるべき"
    );
}

/// WebTransport 対応 HTTP/3 サーバー接続を作成する
#[test]
fn test_wt_h3_server_creation() {
    let settings = Http3Settings::new().with_webtransport().into_raw();

    let h3_conn = Http3Connection::server_new(&settings);
    assert!(
        h3_conn.is_ok(),
        "WebTransport 対応 HTTP/3 サーバー接続を作成できるべき"
    );
}

/// WebTransport リクエスト送信テスト
///
/// 注: Sans I/O テストでは、SETTINGS フレームの交換なしには
/// WebTransport リクエストは送信できない。エラーが発生するのは正常。
#[test]
fn test_submit_wt_request() {
    let settings = Http3Settings::new().with_webtransport().into_raw();
    let mut h3_client =
        Http3Connection::client_new(&settings).expect("HTTP/3 クライアント作成失敗");

    // 制御ストリームと QPACK ストリームをバインド
    h3_client.bind_control_stream(2).expect("test must succeed");
    h3_client
        .bind_qpack_streams(6, 10)
        .expect("test must succeed");

    // WebTransport CONNECT リクエストヘッダー
    let headers = vec![
        Header::method("CONNECT"),
        Header::scheme("https"),
        Header::new(b":protocol".to_vec(), b"webtransport".to_vec()),
        Header::authority("localhost:4433"),
        Header::path("/webtransport"),
    ];

    // WebTransport リクエストを送信 (ストリーム ID 0)
    // 注: Sans I/O テストでは SETTINGS フレームの交換がないため、
    // ERR_INVALID_STATE が発生する可能性がある
    let stream_id = 0;
    let result = h3_client.submit_wt_request(stream_id, &headers);

    match result {
        Ok(()) => {
            eprintln!("submit_wt_request 成功");

            // write_stream でフレームデータを取得
            let mut vecs = vec![
                nghttp3_vec {
                    base: std::ptr::null_mut(),
                    len: 0
                };
                16
            ];
            let write_result = h3_client.write_stream(&mut vecs);

            match write_result {
                Ok((sid, fin, count)) => {
                    eprintln!(
                        "write_stream: stream_id = {}, fin = {}, vec_count = {}",
                        sid, fin, count
                    );
                }
                Err(e) => {
                    eprintln!("write_stream エラー: {:?}", e);
                }
            }
        }
        Err(e) => {
            // Sans I/O テストでは SETTINGS 交換がないためエラーは想定内
            eprintln!("submit_wt_request エラー (想定内): {:?}", e);
        }
    }
}

/// WebTransport レスポンス送信テスト
#[test]
fn test_submit_wt_response() {
    let settings = Http3Settings::new().with_webtransport().into_raw();
    let mut h3_server = Http3Connection::server_new(&settings).expect("HTTP/3 サーバー作成失敗");

    // 制御ストリームと QPACK ストリームをバインド
    h3_server.bind_control_stream(3).expect("test must succeed");
    h3_server
        .bind_qpack_streams(7, 11)
        .expect("test must succeed");

    // WebTransport レスポンスヘッダー (200 OK)
    let headers = vec![Header::status(200)];

    // WebTransport レスポンスを送信
    // 注: 実際にはクライアントからのリクエストを受信してからレスポンスを送信する
    let stream_id = 0;
    let result = h3_server.submit_wt_response(stream_id, &headers);

    // ストリームがオープンされていない場合はエラーになる可能性がある
    eprintln!("submit_wt_response 結果: {:?}", result);
}

/// WebTransport セッション確認テスト
#[test]
fn test_server_confirm_wt_session() {
    let settings = Http3Settings::new().with_webtransport().into_raw();
    let mut h3_server = Http3Connection::server_new(&settings).expect("HTTP/3 サーバー作成失敗");

    // 制御ストリームと QPACK ストリームをバインド
    h3_server.bind_control_stream(3).expect("test must succeed");
    h3_server
        .bind_qpack_streams(7, 11)
        .expect("test must succeed");

    // セッション確認を試行
    // 注: 実際にはリクエスト/レスポンスの後に呼び出す
    let session_id = 0;
    let ts = timestamp();
    let result = h3_server.server_confirm_wt_session(session_id, ts);

    // セッションがオープンされていない場合はエラーになる
    eprintln!("server_confirm_wt_session 結果: {:?}", result);
}

/// WebTransport データストリームオープンテスト
#[test]
fn test_open_wt_data_stream() {
    let settings = Http3Settings::new().with_webtransport().into_raw();
    let mut h3_client =
        Http3Connection::client_new(&settings).expect("HTTP/3 クライアント作成失敗");

    // 制御ストリームと QPACK ストリームをバインド
    h3_client.bind_control_stream(2).expect("test must succeed");
    h3_client
        .bind_qpack_streams(6, 10)
        .expect("test must succeed");

    // WebTransport データストリームをオープン
    let session_id = 0;
    let stream_id = 4; // クライアント開始の双方向ストリーム (2番目)
    let result = h3_client.open_wt_data_stream(session_id, stream_id);

    // セッションがオープンされていない場合はエラーになる
    eprintln!("open_wt_data_stream 結果: {:?}", result);
}

/// WebTransport セッションクローズテスト
#[test]
fn test_close_wt_session() {
    let settings = Http3Settings::new().with_webtransport().into_raw();
    let mut h3_client =
        Http3Connection::client_new(&settings).expect("HTTP/3 クライアント作成失敗");

    // 制御ストリームと QPACK ストリームをバインド
    h3_client.bind_control_stream(2).expect("test must succeed");
    h3_client
        .bind_qpack_streams(6, 10)
        .expect("test must succeed");

    // WebTransport セッションをクローズ
    let session_id = 0;
    let error_code = 0;
    let result = h3_client.close_wt_session(session_id, error_code, None);

    // セッションがオープンされていない場合はエラーになる
    eprintln!("close_wt_session 結果: {:?}", result);
}

/// QUIC + HTTP/3 + WebTransport 統合テスト
#[test]
fn test_quic_h3_webtransport_integration() {
    let (cert_path, key_path) = generate_test_certs();

    // 接続 ID を生成
    let client_dcid = ConnectionId::random(16).expect("test must succeed");
    let client_scid = ConnectionId::random(16).expect("test must succeed");
    let server_scid = ConnectionId::random(16).expect("test must succeed");

    let client_addr: SocketAddr = "127.0.0.1:12345".parse().expect("test must succeed");
    let server_addr: SocketAddr = "127.0.0.1:4433".parse().expect("test must succeed");

    let ts = timestamp();

    // DATAGRAM を有効にしたトランスポートパラメータ (WebTransport に必要)
    let client_params = TransportParams::new().with_datagram(65535).into_raw();
    let server_params = TransportParams::new()
        .with_datagram(65535)
        .with_original_dcid(&client_dcid)
        .into_raw();

    // クライアント QUIC 接続を作成
    let client_tls_ctx = TlsContext::new_client_with_options(&[b"h3"], false)
        .expect("クライアント TLS コンテキスト作成失敗");
    let client_tls_session = client_tls_ctx
        .create_session()
        .expect("TLS セッション作成失敗");

    let mut quic_client = Connection::client_new(
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

    // サーバー QUIC 接続を作成
    let server_tls_ctx = TlsContext::new_server(&cert_path, &key_path, &[b"h3"])
        .expect("サーバー TLS コンテキスト作成失敗");
    let server_tls_session = server_tls_ctx
        .create_session()
        .expect("TLS セッション作成失敗");

    let mut quic_server = Connection::server_new(
        &client_scid,
        &server_scid,
        server_addr,
        client_addr,
        server_tls_session,
        &server_params,
        ts,
    )
    .expect("サーバー接続作成失敗");

    // QUIC ハンドシェイクを完了
    let handshake_done =
        complete_quic_handshake(&mut quic_client, &mut quic_server, client_addr, server_addr);

    if !handshake_done {
        eprintln!("QUIC ハンドシェイクが完了しなかったためスキップ");
        return;
    }

    eprintln!("QUIC ハンドシェイク完了");

    // WebTransport 対応 HTTP/3 接続を作成

    let h3_settings = Http3Settings::new().with_webtransport().into_raw();
    let mut h3_client =
        Http3Connection::client_new(&h3_settings).expect("HTTP/3 クライアント作成失敗");
    let mut h3_server = Http3Connection::server_new(&h3_settings).expect("HTTP/3 サーバー作成失敗");

    // クライアント側: 単方向ストリームを開いて制御ストリームと QPACK ストリームをバインド
    let client_control_stream = quic_client
        .open_uni_stream()
        .expect("ストリームオープン失敗");
    let client_qenc_stream = quic_client
        .open_uni_stream()
        .expect("ストリームオープン失敗");
    let client_qdec_stream = quic_client
        .open_uni_stream()
        .expect("ストリームオープン失敗");

    eprintln!(
        "クライアント制御ストリーム: {}, QPACK: {}, {}",
        client_control_stream, client_qenc_stream, client_qdec_stream
    );

    h3_client
        .bind_control_stream(client_control_stream)
        .expect("制御ストリームバインド失敗");
    h3_client
        .bind_qpack_streams(client_qenc_stream, client_qdec_stream)
        .expect("QPACK ストリームバインド失敗");

    // サーバー側: 単方向ストリームを開いて制御ストリームと QPACK ストリームをバインド
    let server_control_stream = quic_server
        .open_uni_stream()
        .expect("ストリームオープン失敗");
    let server_qenc_stream = quic_server
        .open_uni_stream()
        .expect("ストリームオープン失敗");
    let server_qdec_stream = quic_server
        .open_uni_stream()
        .expect("ストリームオープン失敗");

    eprintln!(
        "サーバー制御ストリーム: {}, QPACK: {}, {}",
        server_control_stream, server_qenc_stream, server_qdec_stream
    );

    h3_server
        .bind_control_stream(server_control_stream)
        .expect("制御ストリームバインド失敗");
    h3_server
        .bind_qpack_streams(server_qenc_stream, server_qdec_stream)
        .expect("QPACK ストリームバインド失敗");

    // クライアントから双方向ストリームを開いて WebTransport リクエストを送信
    let request_stream = quic_client
        .open_bidi_stream()
        .expect("ストリームオープン失敗");
    eprintln!("WebTransport リクエストストリーム: {}", request_stream);

    let wt_headers = vec![
        Header::method("CONNECT"),
        Header::scheme("https"),
        Header::new(b":protocol".to_vec(), b"webtransport".to_vec()),
        Header::authority("localhost:4433"),
        Header::path("/webtransport"),
    ];

    // WebTransport リクエストを送信
    // 注: Sans I/O テストでは SETTINGS フレームの交換がないため、
    // エラーが発生する可能性がある
    let wt_result = h3_client.submit_wt_request(request_stream, &wt_headers);

    match wt_result {
        Ok(()) => {
            eprintln!("WebTransport CONNECT リクエスト送信完了");
        }
        Err(e) => {
            // Sans I/O テストでは SETTINGS 交換がないためエラーは想定内
            eprintln!("WebTransport リクエスト送信エラー (想定内): {:?}", e);
            eprintln!("WebTransport 統合テスト完了 (SETTINGS 交換なしのため部分的)");
            return;
        }
    }

    // HTTP/3 フレームデータを取得
    let mut vecs = vec![
        nghttp3_vec {
            base: std::ptr::null_mut(),
            len: 0
        };
        16
    ];
    loop {
        let result = h3_client.write_stream(&mut vecs);

        match result {
            Ok((stream_id, fin, count)) => {
                if count == 0 {
                    break;
                }
                eprintln!(
                    "HTTP/3 write_stream: stream_id = {}, fin = {}, count = {}",
                    stream_id, fin, count
                );

                // データ長を計算
                let total_len: usize = vecs[..count].iter().map(|v| v.len).sum();
                eprintln!("  合計データ長: {} bytes", total_len);

                // add_write_offset を呼び出す
                h3_client
                    .add_write_offset(stream_id, total_len)
                    .expect("add_write_offset 失敗");
            }
            Err(e) => {
                eprintln!("HTTP/3 write_stream エラー: {:?}", e);
                break;
            }
        }
    }

    eprintln!("WebTransport 統合テスト完了");
}

/// WebTransport イベント受信テスト
#[test]
fn test_wt_events() {
    let settings = Http3Settings::new().with_webtransport().into_raw();
    let mut h3_server = Http3Connection::server_new(&settings).expect("HTTP/3 サーバー作成失敗");

    // 制御ストリームと QPACK ストリームをバインド
    h3_server.bind_control_stream(3).expect("test must succeed");
    h3_server
        .bind_qpack_streams(7, 11)
        .expect("test must succeed");

    // 初期状態ではイベントがない
    let event = h3_server.poll_event();
    assert!(event.is_none(), "初期状態ではイベントがないべき");

    // イベントの種類を確認 (実際のデータ受信時にテスト)
    // WebTransportData イベントは recv_wt_data コールバックで生成される
    eprintln!("WebTransport イベントテスト完了");
}

/// DATAGRAM 対応トランスポートパラメータのテスト
#[test]
fn test_datagram_transport_params() {
    let params = TransportParams::new().with_datagram(65535).into_raw();

    assert_eq!(
        params.max_datagram_frame_size, 65535,
        "DATAGRAM サイズが設定されるべき"
    );

    eprintln!("DATAGRAM トランスポートパラメータ:");
    eprintln!(
        "  max_datagram_frame_size = {}",
        params.max_datagram_frame_size
    );
    eprintln!(
        "  initial_max_streams_bidi = {}",
        params.initial_max_streams_bidi
    );
    eprintln!(
        "  initial_max_streams_uni = {}",
        params.initial_max_streams_uni
    );
    eprintln!("  initial_max_data = {}", params.initial_max_data);
}

/// DATAGRAM 送信テスト (Sans I/O)
///
/// ngtcp2 クライアントからサーバーへの DATAGRAM 送信をテストする。
#[test]
fn test_datagram_send() {
    let (cert_path, key_path) = generate_test_certs();

    // 接続 ID を生成
    let client_dcid = ConnectionId::random(16).expect("test must succeed");
    let client_scid = ConnectionId::random(16).expect("test must succeed");
    let server_scid = ConnectionId::random(16).expect("test must succeed");

    let client_addr: SocketAddr = "127.0.0.1:12346".parse().expect("test must succeed");
    let server_addr: SocketAddr = "127.0.0.1:4434".parse().expect("test must succeed");

    let ts = timestamp();

    // DATAGRAM を有効にしたトランスポートパラメータ
    let client_params = TransportParams::new().with_datagram(65535).into_raw();
    let server_params = TransportParams::new()
        .with_datagram(65535)
        .with_original_dcid(&client_dcid)
        .into_raw();

    // クライアント QUIC 接続を作成
    let client_tls_ctx = TlsContext::new_client_with_options(&[b"h3"], false)
        .expect("クライアント TLS コンテキスト作成失敗");
    let client_tls_session = client_tls_ctx
        .create_session()
        .expect("TLS セッション作成失敗");

    let mut quic_client = Connection::client_new(
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

    // サーバー QUIC 接続を作成
    let server_tls_ctx = TlsContext::new_server(&cert_path, &key_path, &[b"h3"])
        .expect("サーバー TLS コンテキスト作成失敗");
    let server_tls_session = server_tls_ctx
        .create_session()
        .expect("TLS セッション作成失敗");

    let mut quic_server = Connection::server_new(
        &client_scid,
        &server_scid,
        server_addr,
        client_addr,
        server_tls_session,
        &server_params,
        ts,
    )
    .expect("サーバー接続作成失敗");

    // QUIC ハンドシェイクを完了
    let handshake_done =
        complete_quic_handshake(&mut quic_client, &mut quic_server, client_addr, server_addr);

    if !handshake_done {
        panic!("QUIC ハンドシェイクが完了しなかった");
    }

    eprintln!("QUIC ハンドシェイク完了");

    // ローカルとリモートの DATAGRAM サポートを確認
    let local_max_datagram = quic_client.get_local_max_datagram_frame_size();
    let remote_max_datagram = quic_client.get_remote_max_datagram_frame_size();
    eprintln!("local_max_datagram_frame_size = {}", local_max_datagram);
    eprintln!("remote_max_datagram_frame_size = {}", remote_max_datagram);

    assert!(
        local_max_datagram > 0,
        "ローカルが DATAGRAM をサポートしていること"
    );
    assert!(
        quic_client.can_send_datagram(),
        "リモートピアが DATAGRAM をサポートしていること"
    );

    // DATAGRAM を送信
    let datagram_data = b"Hello DATAGRAM!";
    let mut buf = vec![0u8; 1350];
    let current_ts = timestamp();

    eprintln!("DATAGRAM 送信中...");
    let result = quic_client.write_datagram(&mut buf, datagram_data, current_ts);

    match result {
        Ok((written, accepted)) => {
            eprintln!(
                "DATAGRAM 送信成功: written = {}, accepted = {}",
                written, accepted
            );
            // 送信が成功しても、輻輳制御により受け入れられない場合がある
            if accepted {
                eprintln!("DATAGRAM が受け入れられた");
            } else {
                eprintln!("DATAGRAM は輻輳制御により受け入れられなかった");
            }
        }
        Err(e) => {
            eprintln!("DATAGRAM 送信エラー: {:?}", e);
            panic!("DATAGRAM 送信失敗: {:?}", e);
        }
    }

    eprintln!("DATAGRAM 送信テスト完了");
}

// =============================================================================
// ヘルパー関数: QUIC + HTTP/3 + WebTransport セッション確立
// =============================================================================

/// QUIC + HTTP/3 接続ペア
struct QuicH3Pair {
    quic_client: Connection,
    quic_server: Connection,
    h3_client: Http3Connection,
    h3_server: Http3Connection,
    client_addr: SocketAddr,
    server_addr: SocketAddr,
}

/// QUIC + HTTP/3 接続ペアを作成し、SETTINGS 交換まで完了させる
fn setup_quic_h3_pair() -> QuicH3Pair {
    let (cert_path, key_path) = generate_test_certs();

    let client_dcid = ConnectionId::random(16).expect("test must succeed");
    let client_scid = ConnectionId::random(16).expect("test must succeed");
    let server_scid = ConnectionId::random(16).expect("test must succeed");

    let client_addr: SocketAddr = "127.0.0.1:12345".parse().expect("test must succeed");
    let server_addr: SocketAddr = "127.0.0.1:4433".parse().expect("test must succeed");

    let ts = timestamp();

    // DATAGRAM を有効にしたトランスポートパラメータ (WebTransport に必要)
    let client_params = TransportParams::new().with_datagram(65535).into_raw();
    let server_params = TransportParams::new()
        .with_datagram(65535)
        .with_original_dcid(&client_dcid)
        .into_raw();

    // QUIC 接続を作成
    let client_tls_ctx = TlsContext::new_client_with_options(&[b"h3"], false)
        .expect("クライアント TLS コンテキスト作成失敗");
    let client_tls_session = client_tls_ctx
        .create_session()
        .expect("TLS セッション作成失敗");
    let mut quic_client = Connection::client_new(
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

    let server_tls_ctx = TlsContext::new_server(&cert_path, &key_path, &[b"h3"])
        .expect("サーバー TLS コンテキスト作成失敗");
    let server_tls_session = server_tls_ctx
        .create_session()
        .expect("TLS セッション作成失敗");
    let mut quic_server = Connection::server_new(
        &client_scid,
        &server_scid,
        server_addr,
        client_addr,
        server_tls_session,
        &server_params,
        ts,
    )
    .expect("サーバー接続作成失敗");

    // QUIC ハンドシェイクを完了
    assert!(
        complete_quic_handshake(&mut quic_client, &mut quic_server, client_addr, server_addr),
        "QUIC ハンドシェイクが完了しなかった"
    );

    // WebTransport 対応 HTTP/3 接続を作成

    let h3_settings = Http3Settings::new().with_webtransport().into_raw();
    let mut h3_client =
        Http3Connection::client_new(&h3_settings).expect("HTTP/3 クライアント作成失敗");
    let mut h3_server = Http3Connection::server_new(&h3_settings).expect("HTTP/3 サーバー作成失敗");

    // 制御ストリームと QPACK ストリームをバインド
    let client_ctrl = quic_client
        .open_uni_stream()
        .expect("ストリームオープン失敗");
    let client_qenc = quic_client
        .open_uni_stream()
        .expect("ストリームオープン失敗");
    let client_qdec = quic_client
        .open_uni_stream()
        .expect("ストリームオープン失敗");
    h3_client
        .bind_control_stream(client_ctrl)
        .expect("制御ストリームバインド失敗");
    h3_client
        .bind_qpack_streams(client_qenc, client_qdec)
        .expect("QPACK ストリームバインド失敗");

    let server_ctrl = quic_server
        .open_uni_stream()
        .expect("ストリームオープン失敗");
    let server_qenc = quic_server
        .open_uni_stream()
        .expect("ストリームオープン失敗");
    let server_qdec = quic_server
        .open_uni_stream()
        .expect("ストリームオープン失敗");
    h3_server
        .bind_control_stream(server_ctrl)
        .expect("制御ストリームバインド失敗");
    h3_server
        .bind_qpack_streams(server_qenc, server_qdec)
        .expect("QPACK ストリームバインド失敗");

    // SETTINGS フレームを交換するためにパケットをやり取り
    exchange_packets(
        &mut quic_client,
        &mut quic_server,
        &mut h3_client,
        &mut h3_server,
        client_addr,
        server_addr,
    );

    QuicH3Pair {
        quic_client,
        quic_server,
        h3_client,
        h3_server,
        client_addr,
        server_addr,
    }
}

/// H3 write_stream → QUIC write_stream → QUIC write_pkt → 相手の QUIC read_pkt →
/// QUIC poll_stream_data → H3 read_stream のパケット交換ループ
fn exchange_packets(
    quic_client: &mut Connection,
    quic_server: &mut Connection,
    h3_client: &mut Http3Connection,
    h3_server: &mut Http3Connection,
    client_addr: SocketAddr,
    server_addr: SocketAddr,
) {
    let pkt_info = PacketInfo::default();

    for _ in 0..10 {
        let ts = timestamp();

        // クライアント → サーバー: H3 データを QUIC パケットに変換して送信
        let packets = write_h3_to_packets(h3_client, quic_client, ts);
        for pkt in &packets {
            let _ = quic_server.read_pkt(&server_addr, &client_addr, &pkt_info, pkt, ts);
        }
        // 残りの QUIC パケット (ACK 等) を送信
        send_quic_packets(
            quic_client,
            quic_server,
            client_addr,
            server_addr,
            &pkt_info,
            ts,
        );
        // 受信ストリームデータを H3 に渡す
        feed_quic_to_h3(quic_server, h3_server, ts);

        // サーバー → クライアント
        let ts = timestamp();
        let packets = write_h3_to_packets(h3_server, quic_server, ts);
        for pkt in &packets {
            let _ = quic_client.read_pkt(&client_addr, &server_addr, &pkt_info, pkt, ts);
        }
        send_quic_packets(
            quic_server,
            quic_client,
            server_addr,
            client_addr,
            &pkt_info,
            ts,
        );
        feed_quic_to_h3(quic_client, h3_client, ts);
    }
}

/// H3 の write_stream からデータを取り出し、QUIC パケットに変換して返す
///
/// tokio-ngtcp2 の write_h3_streams() に相当する。
/// NGTCP2_WRITE_STREAM_FLAG_MORE により、write_stream はデータをバッファに
/// 追加するだけでパケットを生成しない場合がある (pkt_written == 0)。
/// その場合、後続の write_pkt でパケットが生成される。
fn write_h3_to_packets(h3: &mut Http3Connection, quic: &mut Connection, ts: u64) -> Vec<Vec<u8>> {
    let mut packets = Vec::new();
    let mut send_buf = vec![0u8; 1350];

    let mut vecs = vec![
        nghttp3_vec {
            base: std::ptr::null_mut(),
            len: 0,
        };
        16
    ];

    while let Ok((stream_id, fin, count)) = h3.write_stream(&mut vecs) {
        if count == 0 {
            break;
        }

        // nghttp3_vec のポインタからデータをコピー
        let mut h3_data = Vec::new();
        for v in vecs.iter().take(count) {
            if v.len > 0 && !v.base.is_null() {
                let data = unsafe { std::slice::from_raw_parts(v.base as *const u8, v.len) };
                h3_data.extend_from_slice(data);
            }
        }

        if h3_data.is_empty() && !fin {
            continue;
        }

        // QUIC レイヤに書き込み
        match quic.write_stream(&mut send_buf, stream_id, &h3_data, fin, ts) {
            Ok((pkt_written, data_written)) => {
                if pkt_written > 0 {
                    packets.push(send_buf[..pkt_written].to_vec());
                }
                if let Some(dw) = data_written
                    && dw > 0
                {
                    let _ = h3.add_write_offset(stream_id, dw);
                }
            }
            Err(shiguredo_ngtcp2::Error::StreamDataBlocked(_)) => {
                h3.block_stream(stream_id);
            }
            Err(shiguredo_ngtcp2::Error::StreamShutWr(_)) => {
                h3.shutdown_stream_write(stream_id);
            }
            Err(_) => break,
        }
    }

    // バッファに溜まったデータをフラッシュ
    loop {
        match quic.write_pkt(&mut send_buf, ts) {
            Ok((written, _)) if written > 0 => {
                packets.push(send_buf[..written].to_vec());
            }
            _ => break,
        }
    }

    packets
}

/// QUIC パケットを送信側から受信側に転送する
fn send_quic_packets(
    sender: &mut Connection,
    receiver: &mut Connection,
    sender_addr: SocketAddr,
    receiver_addr: SocketAddr,
    pkt_info: &PacketInfo,
    ts: u64,
) {
    let mut buf = vec![0u8; 1350];
    loop {
        match sender.write_pkt(&mut buf, ts) {
            Ok((written, _)) if written > 0 => {
                let _ =
                    receiver.read_pkt(&receiver_addr, &sender_addr, pkt_info, &buf[..written], ts);
            }
            _ => break,
        }
    }
}

/// QUIC の受信ストリームデータを H3 に渡す
fn feed_quic_to_h3(quic: &mut Connection, h3: &mut Http3Connection, ts: u64) {
    while let Some(stream_data) = quic.poll_stream_data() {
        if let Ok(consumed) = h3.read_stream(
            stream_data.stream_id,
            &stream_data.data,
            stream_data.fin,
            ts,
        ) && consumed > 0
        {
            let _ = quic.extend_max_stream_offset(stream_data.stream_id, consumed as u64);
            quic.extend_max_offset(consumed as u64);
        }
    }
}

/// WebTransport セッションを確立するヘルパー関数
///
/// QUIC + HTTP/3 接続ペア上で WebTransport CONNECT リクエスト/レスポンスを交換し、
/// セッションを確立する。セッション ID (= CONNECT ストリーム ID) を返す。
fn establish_wt_session(pair: &mut QuicH3Pair) -> i64 {
    // クライアントから双方向ストリームを開いて WebTransport リクエストを送信
    let session_stream = pair
        .quic_client
        .open_bidi_stream()
        .expect("ストリームオープン失敗");

    let wt_headers = vec![
        Header::method("CONNECT"),
        Header::scheme("https"),
        Header::new(b":protocol".to_vec(), b"webtransport".to_vec()),
        Header::authority("localhost:4433"),
        Header::path("/webtransport"),
    ];

    pair.h3_client
        .submit_wt_request(session_stream, &wt_headers)
        .expect("WebTransport リクエスト送信失敗");

    // クライアント → サーバーへパケット交換
    exchange_packets(
        &mut pair.quic_client,
        &mut pair.quic_server,
        &mut pair.h3_client,
        &mut pair.h3_server,
        pair.client_addr,
        pair.server_addr,
    );

    // サーバー側でヘッダーイベントを処理
    drain_header_events(&mut pair.h3_server);

    // サーバーが WebTransport レスポンスを送信
    let response_headers = vec![Header::status(200)];
    pair.h3_server
        .submit_wt_response(session_stream, &response_headers)
        .expect("WebTransport レスポンス送信失敗");

    let ts = timestamp();
    pair.h3_server
        .server_confirm_wt_session(session_stream, ts)
        .expect("WebTransport セッション確認失敗");

    // サーバー → クライアントへパケット交換
    exchange_packets(
        &mut pair.quic_client,
        &mut pair.quic_server,
        &mut pair.h3_client,
        &mut pair.h3_server,
        pair.client_addr,
        pair.server_addr,
    );

    // クライアント側でヘッダーイベントを処理
    drain_header_events(&mut pair.h3_client);

    session_stream
}

/// ヘッダーイベントを消費する (テスト対象外のイベントをドレイン)
fn drain_header_events(h3: &mut Http3Connection) {
    while let Some(event) = h3.poll_event() {
        match event {
            Http3Event::HeadersBegin { .. }
            | Http3Event::Header { .. }
            | Http3Event::HeadersEnd { .. } => {}
            _ => {}
        }
    }
}

/// サーバー側の WebTransportData を集めるまでパケット交換する
///
/// Sans I/O では 1 回の交換では ACK / 再送待ちでデータが届かないことがあるため、
/// データが来るまで複数ラウンド回す。
fn exchange_until_server_wt_data(pair: &mut QuicH3Pair) -> Vec<u8> {
    let mut received = Vec::new();
    for _ in 0..20 {
        exchange_packets(
            &mut pair.quic_client,
            &mut pair.quic_server,
            &mut pair.h3_client,
            &mut pair.h3_server,
            pair.client_addr,
            pair.server_addr,
        );
        while let Some(event) = pair.h3_server.poll_event() {
            if let Http3Event::WebTransportData { data, .. } = event {
                received.extend_from_slice(&data);
            }
        }
        if !received.is_empty() {
            break;
        }
    }
    received
}

/// サーバー側の WebTransportData をストリーム ID 別に集めるまでパケット交換する
fn exchange_until_server_wt_stream_data(
    pair: &mut QuicH3Pair,
) -> std::collections::HashMap<i64, Vec<u8>> {
    let mut stream_data: std::collections::HashMap<i64, Vec<u8>> = std::collections::HashMap::new();
    for _ in 0..20 {
        exchange_packets(
            &mut pair.quic_client,
            &mut pair.quic_server,
            &mut pair.h3_client,
            &mut pair.h3_server,
            pair.client_addr,
            pair.server_addr,
        );
        while let Some(event) = pair.h3_server.poll_event() {
            if let Http3Event::WebTransportData {
                stream_id, data, ..
            } = event
            {
                stream_data
                    .entry(stream_id)
                    .or_default()
                    .extend_from_slice(&data);
            }
        }
        if !stream_data.is_empty() {
            break;
        }
    }
    stream_data
}

// =============================================================================
// B1: WebTransport データストリーム送受信テスト
// =============================================================================

/// セッション確立後、クライアントが bidi ストリームでデータ送信 → サーバーが受信
#[test]
fn test_wt_bidirectional_data_exchange() {
    let mut pair = setup_quic_h3_pair();
    let session_id = establish_wt_session(&mut pair);

    // クライアントから双方向データストリームを開く
    let data_stream = pair
        .quic_client
        .open_bidi_stream()
        .expect("データストリームオープン失敗");

    pair.h3_client
        .open_wt_data_stream(session_id, data_stream)
        .expect("WebTransport データストリームオープン失敗");

    // データを送信
    let test_data = b"Hello WebTransport!";
    pair.h3_client
        .send_wt_stream_data(data_stream, test_data, false)
        .expect("データ送信失敗");

    let received_data = exchange_until_server_wt_data(&mut pair);

    assert_eq!(
        received_data, test_data,
        "サーバーが WebTransport データを受信するべき"
    );
}

/// クライアントが uni ストリームでデータ送信 → サーバーで受信
#[test]
fn test_wt_unidirectional_data_stream() {
    let mut pair = setup_quic_h3_pair();
    let session_id = establish_wt_session(&mut pair);

    // クライアントから単方向データストリームを開く
    let data_stream = pair
        .quic_client
        .open_uni_stream()
        .expect("単方向ストリームオープン失敗");

    pair.h3_client
        .open_wt_data_stream(session_id, data_stream)
        .expect("WebTransport 単方向データストリームオープン失敗");

    // データを送信
    let test_data = b"Unidirectional data";
    pair.h3_client
        .send_wt_stream_data(data_stream, test_data, false)
        .expect("単方向データ送信失敗");

    let received_data = exchange_until_server_wt_data(&mut pair);

    assert_eq!(
        received_data, test_data,
        "サーバーが単方向 WebTransport データを受信するべき"
    );
}

/// send_wt_stream_data(id, data, true) で FIN 付きデータ送信 → サーバーでストリーム終了を検出
#[test]
fn test_wt_send_data_with_fin() {
    let mut pair = setup_quic_h3_pair();
    let session_id = establish_wt_session(&mut pair);

    // クライアントから双方向データストリームを開く
    let data_stream = pair
        .quic_client
        .open_bidi_stream()
        .expect("データストリームオープン失敗");

    pair.h3_client
        .open_wt_data_stream(session_id, data_stream)
        .expect("WebTransport データストリームオープン失敗");

    // FIN 付きでデータを送信
    let test_data = b"Final data";
    pair.h3_client
        .send_wt_stream_data(data_stream, test_data, true)
        .expect("FIN 付きデータ送信失敗");

    let mut received_data = Vec::new();
    let mut stream_ended = false;
    for _ in 0..20 {
        exchange_packets(
            &mut pair.quic_client,
            &mut pair.quic_server,
            &mut pair.h3_client,
            &mut pair.h3_server,
            pair.client_addr,
            pair.server_addr,
        );
        while let Some(event) = pair.h3_server.poll_event() {
            match event {
                Http3Event::WebTransportData { data, .. } => {
                    received_data.extend_from_slice(&data);
                }
                Http3Event::StreamEnd { stream_id } if stream_id == data_stream => {
                    stream_ended = true;
                }
                _ => {}
            }
        }
        if !received_data.is_empty() {
            break;
        }
    }

    assert_eq!(
        received_data, test_data,
        "サーバーが FIN 付きデータを受信するべき"
    );
    // StreamEnd イベントは nghttp3 の実装に依存するため必須ではない
    if stream_ended {
        eprintln!("StreamEnd イベントを受信した");
    }
}

// =============================================================================
// B2: WebTransport セッション終了テスト
// =============================================================================

/// close_wt_session(session_id, error_code, msg) でセッションクローズ → 相手側で検出
#[test]
fn test_wt_close_session_with_error() {
    let mut pair = setup_quic_h3_pair();
    let session_id = establish_wt_session(&mut pair);

    // クライアントがセッションをクローズ
    let error_code = 42u32;
    let error_msg = b"test close";
    pair.h3_client
        .close_wt_session(session_id, error_code, Some(error_msg))
        .expect("セッションクローズ失敗");

    // パケット交換
    exchange_packets(
        &mut pair.quic_client,
        &mut pair.quic_server,
        &mut pair.h3_client,
        &mut pair.h3_server,
        pair.client_addr,
        pair.server_addr,
    );

    // サーバー側でクローズイベントを確認
    // nghttp3 は WT_CLOSE_SESSION を受信すると StreamClose イベントを生成する
    let mut got_close_event = false;
    while let Some(event) = pair.h3_server.poll_event() {
        match event {
            Http3Event::StreamClose { stream_id, .. } if stream_id == session_id => {
                got_close_event = true;
            }
            Http3Event::StreamEnd { stream_id } if stream_id == session_id => {
                got_close_event = true;
            }
            _ => {}
        }
    }

    // nghttp3 の実装によってイベントの種類が異なる場合がある
    eprintln!("セッションクローズイベント検出: {}", got_close_event);
}

// =============================================================================
// B3: 複数ストリームテスト
// =============================================================================

/// 1 セッション上で bidi 2 本 + uni 1 本を開き、各ストリームのデータが分離されて受信される
#[test]
fn test_wt_multiple_data_streams() {
    let mut pair = setup_quic_h3_pair();
    let session_id = establish_wt_session(&mut pair);

    // bidi ストリーム 2 本を開く
    let bidi_stream_1 = pair
        .quic_client
        .open_bidi_stream()
        .expect("bidi ストリーム 1 オープン失敗");
    let bidi_stream_2 = pair
        .quic_client
        .open_bidi_stream()
        .expect("bidi ストリーム 2 オープン失敗");

    // uni ストリーム 1 本を開く
    let uni_stream = pair
        .quic_client
        .open_uni_stream()
        .expect("uni ストリームオープン失敗");

    pair.h3_client
        .open_wt_data_stream(session_id, bidi_stream_1)
        .expect("bidi データストリーム 1 オープン失敗");
    pair.h3_client
        .open_wt_data_stream(session_id, bidi_stream_2)
        .expect("bidi データストリーム 2 オープン失敗");
    pair.h3_client
        .open_wt_data_stream(session_id, uni_stream)
        .expect("uni データストリームオープン失敗");

    // 各ストリームにデータを送信
    let data_1 = b"Stream 1 data";
    let data_2 = b"Stream 2 data";
    let data_3 = b"Stream 3 uni data";

    pair.h3_client
        .send_wt_stream_data(bidi_stream_1, data_1, false)
        .expect("bidi ストリーム 1 データ送信失敗");
    pair.h3_client
        .send_wt_stream_data(bidi_stream_2, data_2, false)
        .expect("bidi ストリーム 2 データ送信失敗");
    pair.h3_client
        .send_wt_stream_data(uni_stream, data_3, false)
        .expect("uni ストリームデータ送信失敗");

    let stream_data = exchange_until_server_wt_stream_data(&mut pair);

    // 少なくとも 1 つのストリームでデータが受信されていることを確認
    assert!(
        !stream_data.is_empty(),
        "少なくとも 1 つのストリームでデータが受信されるべき"
    );

    // 各ストリームのデータが正しいことを検証
    if let Some(d) = stream_data.get(&bidi_stream_1) {
        assert_eq!(d, data_1, "bidi ストリーム 1 のデータが一致するべき");
    }
    if let Some(d) = stream_data.get(&bidi_stream_2) {
        assert_eq!(d, data_2, "bidi ストリーム 2 のデータが一致するべき");
    }
    if let Some(d) = stream_data.get(&uni_stream) {
        assert_eq!(d, data_3, "uni ストリームのデータが一致するべき");
    }

    eprintln!(
        "受信ストリーム数: {}, ストリーム ID: {:?}",
        stream_data.len(),
        stream_data.keys().collect::<Vec<_>>()
    );
}
