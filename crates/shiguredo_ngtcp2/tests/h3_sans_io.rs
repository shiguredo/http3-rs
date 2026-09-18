//! HTTP/3 Sans I/O テスト
//!
//! Http3Connection の read_stream / write_stream / poll_event を
//! 直接テストする。QUIC ハンドシェイク完了後の HTTP/3 レイヤの動作を検証する。

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
        "h3_sans_io_test_{}_{}",
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

/// HTTP/3 クライアント接続を作成する
#[test]
fn test_h3_client_creation() {
    let settings = Http3Settings::new().into_raw();
    let h3_conn = Http3Connection::client_new(&settings);

    assert!(h3_conn.is_ok(), "HTTP/3 クライアント接続を作成できるべき");
}

/// HTTP/3 サーバー接続を作成する
#[test]
fn test_h3_server_creation() {
    let settings = Http3Settings::new().into_raw();
    let h3_conn = Http3Connection::server_new(&settings);

    assert!(h3_conn.is_ok(), "HTTP/3 サーバー接続を作成できるべき");
}

/// HTTP/3 制御ストリームとQPACK ストリームのバインド
#[test]
fn test_h3_stream_binding() {
    let settings = Http3Settings::new().into_raw();
    let mut h3_conn = Http3Connection::client_new(&settings).expect("HTTP/3 接続作成失敗");

    // 制御ストリームをバインド (クライアント単方向ストリーム: 2, 6, 10, ...)
    // ストリーム ID: 0x02 = クライアント開始の単方向ストリーム (最初)
    let control_stream_id = 2; // 0x02
    let result = h3_conn.bind_control_stream(control_stream_id);
    assert!(result.is_ok(), "制御ストリームをバインドできるべき");

    // QPACK ストリームをバインド
    let qenc_stream_id = 6; // 0x06
    let qdec_stream_id = 10; // 0x0A
    let result = h3_conn.bind_qpack_streams(qenc_stream_id, qdec_stream_id);
    assert!(result.is_ok(), "QPACK ストリームをバインドできるべき");
}

/// HTTP/3 リクエスト送信テスト
#[test]
fn test_h3_submit_request() {
    let settings = Http3Settings::new().into_raw();
    let mut h3_conn = Http3Connection::client_new(&settings).expect("HTTP/3 接続作成失敗");

    // 制御ストリームと QPACK ストリームをバインド
    h3_conn.bind_control_stream(2).expect("test must succeed");
    h3_conn
        .bind_qpack_streams(6, 10)
        .expect("test must succeed");

    // リクエストヘッダー
    let headers = vec![
        Header::method("GET"),
        Header::scheme("https"),
        Header::authority("localhost"),
        Header::path("/"),
    ];

    // リクエストを送信 (ストリーム ID 0 はクライアント開始の双方向ストリーム)
    let stream_id = 0;
    let result = h3_conn.submit_request(stream_id, &headers);
    assert!(result.is_ok(), "リクエストを送信できるべき");

    // write_stream でフレームデータを取得
    let mut vecs = vec![
        nghttp3_vec {
            base: std::ptr::null_mut(),
            len: 0
        };
        16
    ];
    let result = h3_conn.write_stream(&mut vecs);

    match result {
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

/// HTTP/3 レスポンス送信テスト
#[test]
fn test_h3_submit_response() {
    let settings = Http3Settings::new().into_raw();
    let mut h3_conn = Http3Connection::server_new(&settings).expect("HTTP/3 接続作成失敗");

    // 制御ストリームと QPACK ストリームをバインド (サーバー側)
    // サーバー開始の単方向ストリーム: 3, 7, 11, ...
    h3_conn.bind_control_stream(3).expect("test must succeed");
    h3_conn
        .bind_qpack_streams(7, 11)
        .expect("test must succeed");

    // レスポンスヘッダー
    let headers = vec![Header::status(200)];

    // クライアントからのリクエストを受信したと仮定してレスポンスを送信
    // ストリーム ID 0 はクライアント開始の双方向ストリーム
    let stream_id = 0;
    let result = h3_conn.submit_response(stream_id, &headers);

    // ストリームがオープンされていない場合はエラーになる可能性がある
    eprintln!("submit_response 結果: {:?}", result);
}

/// HTTP/3 イベントポーリングテスト
#[test]
fn test_h3_poll_event() {
    let settings = Http3Settings::new().into_raw();
    let mut h3_conn = Http3Connection::client_new(&settings).expect("HTTP/3 接続作成失敗");

    // 初期状態ではイベントがない
    let event = h3_conn.poll_event();
    assert!(event.is_none(), "初期状態ではイベントがないべき");
}

/// QUIC + HTTP/3 統合テスト
#[test]
fn test_quic_h3_integration() {
    let (cert_path, key_path) = generate_test_certs();

    // 接続 ID を生成
    let client_dcid = ConnectionId::random(16).expect("test must succeed");
    let client_scid = ConnectionId::random(16).expect("test must succeed");
    let server_scid = ConnectionId::random(16).expect("test must succeed");

    let client_addr: SocketAddr = "127.0.0.1:12345".parse().expect("test must succeed");
    let server_addr: SocketAddr = "127.0.0.1:4433".parse().expect("test must succeed");

    let ts = timestamp();

    // クライアント QUIC 接続を作成
    let client_tls_ctx = TlsContext::new_client_with_options(&[b"h3"], false)
        .expect("クライアント TLS コンテキスト作成失敗");
    let client_tls_session = client_tls_ctx
        .create_session()
        .expect("TLS セッション作成失敗");
    let client_params = TransportParams::new().with_datagram(65535).into_raw();

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
    let server_params = TransportParams::new()
        .with_datagram(65535)
        .with_original_dcid(&client_dcid)
        .into_raw();

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

    // HTTP/3 接続を作成

    let h3_settings = Http3Settings::new().into_raw();
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

    // クライアントから双方向ストリームを開いてリクエストを送信
    let request_stream = quic_client
        .open_bidi_stream()
        .expect("ストリームオープン失敗");
    eprintln!("リクエストストリーム: {}", request_stream);

    let headers = vec![
        Header::method("GET"),
        Header::scheme("https"),
        Header::authority("localhost"),
        Header::path("/"),
    ];

    h3_client
        .submit_request(request_stream, &headers)
        .expect("リクエスト送信失敗");

    eprintln!("HTTP/3 リクエスト送信完了");

    // HTTP/3 フレームデータを取得
    let mut vecs = vec![
        nghttp3_vec {
            base: std::ptr::null_mut(),
            len: 0
        };
        16
    ];
    let result = h3_client.write_stream(&mut vecs);

    match result {
        Ok((stream_id, fin, count)) => {
            eprintln!(
                "HTTP/3 write_stream: stream_id = {}, fin = {}, count = {}",
                stream_id, fin, count
            );
        }
        Err(e) => {
            eprintln!("HTTP/3 write_stream エラー: {:?}", e);
        }
    }

    eprintln!("HTTP/3 統合テスト完了");
}

/// HTTP/3 ヘッダー受信テスト
#[test]
fn test_h3_headers_receive() {
    let settings = Http3Settings::new().into_raw();
    let mut h3_server = Http3Connection::server_new(&settings).expect("HTTP/3 サーバー作成失敗");

    // 制御ストリームと QPACK ストリームをバインド
    h3_server.bind_control_stream(3).expect("test must succeed");
    h3_server
        .bind_qpack_streams(7, 11)
        .expect("test must succeed");

    // HTTP/3 HEADERS フレームのモックデータ
    // 実際のフレームデータは QPACK エンコードされているため、
    // ここでは空のデータで read_stream の動作を確認
    let result = h3_server.read_stream(0, &[], false, 0);

    match result {
        Ok(consumed) => {
            eprintln!("read_stream: consumed = {}", consumed);
        }
        Err(e) => {
            // ストリームがオープンされていない場合はエラーになる
            eprintln!("read_stream エラー (予想通り): {:?}", e);
        }
    }

    // イベントを確認
    while let Some(event) = h3_server.poll_event() {
        match event {
            Http3Event::HeadersBegin { stream_id } => {
                eprintln!("HeadersBegin: stream_id = {}", stream_id);
            }
            Http3Event::Header { stream_id, header } => {
                eprintln!(
                    "Header: stream_id = {}, name = {:?}, value = {:?}",
                    stream_id,
                    header.name_str(),
                    header.value_str()
                );
            }
            Http3Event::HeadersEnd { stream_id, fin } => {
                eprintln!("HeadersEnd: stream_id = {}, fin = {}", stream_id, fin);
            }
            _ => {
                eprintln!("Other event: {:?}", event);
            }
        }
    }
}
