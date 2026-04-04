//! HTTP/3 相互運用性テスト用共通ヘルパー
//!
//! s2n-quic, quiche, ngtcp2, quinn, tquic の相互運用性テストを支援するためのユーティリティ。

use std::error::Error;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::{Buf, Bytes};
use quiche::h3::NameValue;
use rcgen::generate_simple_self_signed;
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use tempfile::TempDir;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

pub const MAX_DATAGRAM_SIZE: usize = 1350;

/// 共有証明書を生成 (すべての実装で使用)
pub fn generate_shared_certificate() -> Result<(String, String), Box<dyn Error + Send + Sync>> {
    let subject_alt_names = vec!["localhost".to_string(), "127.0.0.1".to_string()];
    let certified_key = generate_simple_self_signed(subject_alt_names)?;
    let cert_pem = certified_key.cert.pem();
    let key_pem = certified_key.signing_key.serialize_pem();
    Ok((cert_pem, key_pem))
}

/// 証明書をファイルに保存 (quiche, ngtcp2 用)
///
/// 一時ディレクトリを作成して証明書を保存する。戻り値の `TempDir` を
/// テストのスコープ内で保持することで、テスト終了時に自動削除される。
/// 並行テスト実行時の証明書ファイルの競合を防ぐため、毎回異なる一時ディレクトリを使用する。
///
/// 一時ディレクトリを作成して証明書を保存する。戻り値の `TempDir` を
/// テストのスコープ内で保持することで、テスト終了時に自動削除される。
/// 並行テスト実行時の証明書ファイルの競合を防ぐため、毎回異なる一時ディレクトリを使用する。
pub fn save_certificate_files(
    cert_pem: &str,
    key_pem: &str,
) -> Result<(TempDir, PathBuf, PathBuf), Box<dyn Error + Send + Sync>> {
    let cert_dir = TempDir::new()?;

    let cert_path = cert_dir.path().join("cert.pem");
    let key_path = cert_dir.path().join("key.pem");

    std::fs::write(&cert_path, cert_pem)?;
    std::fs::write(&key_path, key_pem)?;

    Ok((cert_dir, cert_path, key_path))
}

/// quiche サーバーを起動
pub async fn start_quiche_server(
    cert_path: PathBuf,
    key_path: PathBuf,
    port_tx: std::sync::mpsc::Sender<u16>,
    mut shutdown_rx: mpsc::Receiver<()>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let socket = UdpSocket::bind("127.0.0.1:0").await?;
    let local_addr = socket.local_addr()?;
    eprintln!("[quiche server] サーバー起動: port = {}", local_addr.port());
    port_tx.send(local_addr.port()).unwrap();

    let mut config = quiche::Config::new(quiche::PROTOCOL_VERSION)?;
    config.load_cert_chain_from_pem_file(cert_path.to_str().unwrap())?;
    config.load_priv_key_from_pem_file(key_path.to_str().unwrap())?;
    config.set_application_protos(&[b"h3"])?;
    config.set_max_idle_timeout(10000);
    config.set_max_recv_udp_payload_size(MAX_DATAGRAM_SIZE);
    config.set_max_send_udp_payload_size(MAX_DATAGRAM_SIZE);
    config.set_initial_max_data(10_000_000);
    config.set_initial_max_stream_data_bidi_local(1_000_000);
    config.set_initial_max_stream_data_bidi_remote(1_000_000);
    config.set_initial_max_stream_data_uni(1_000_000);
    config.set_initial_max_streams_bidi(100);
    config.set_initial_max_streams_uni(100);
    config.set_disable_active_migration(true);

    let mut buf = [0u8; 65535];
    let mut out = [0u8; MAX_DATAGRAM_SIZE];

    let mut conn: Option<quiche::Connection> = None;
    let mut h3_conn: Option<quiche::h3::Connection> = None;

    let timeout = tokio::time::sleep(Duration::from_secs(10));
    tokio::pin!(timeout);

    loop {
        tokio::select! {
            biased;

            _ = shutdown_rx.recv() => {
                eprintln!("[quiche server] シャットダウン");
                break;
            }

            _ = &mut timeout => {
                eprintln!("[quiche server] タイムアウト");
                break;
            }

            result = socket.recv_from(&mut buf) => {
                let (len, from) = match result {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("[quiche server] recv エラー: {:?}", e);
                        continue;
                    }
                };

                let pkt_buf = &mut buf[..len];

                if conn.is_none() {
                    let hdr = match quiche::Header::from_slice(pkt_buf, quiche::MAX_CONN_ID_LEN) {
                        Ok(v) => v,
                        Err(e) => {
                            eprintln!("[quiche server] ヘッダーパースエラー: {:?}", e);
                            continue;
                        }
                    };

                    let scid: quiche::ConnectionId<'static> =
                        quiche::ConnectionId::from_ref(&hdr.dcid).into_owned();
                    let new_conn = quiche::accept(&scid, None, local_addr, from, &mut config)?;
                    conn = Some(new_conn);
                    eprintln!("[quiche server] 新しい接続: {:?}", from);
                }

                if let Some(ref mut c) = conn {
                    let recv_info = quiche::RecvInfo {
                        to: local_addr,
                        from,
                    };

                    if let Err(e) = c.recv(pkt_buf, recv_info) {
                        eprintln!("[quiche server] recv エラー: {:?}", e);
                        continue;
                    }

                    if c.is_established() && h3_conn.is_none() {
                        let mut h3_config = quiche::h3::Config::new()?;
                        h3_config.set_max_field_section_size(16384);
                        // QPACK 動的テーブルを無効化 (shiguredo_http3 は静的テーブルのみサポート)
                        h3_config.set_qpack_max_table_capacity(0);
                        h3_config.set_qpack_blocked_streams(0);
                        h3_conn = Some(quiche::h3::Connection::with_transport(c, &h3_config)?);
                        eprintln!("[quiche server] HTTP/3 接続確立");
                    }

                    if let Some(ref mut h3) = h3_conn {
                        loop {
                            match h3.poll(c) {
                                Ok((stream_id, quiche::h3::Event::Headers { list, .. })) => {
                                    eprintln!(
                                        "[quiche server] ヘッダー受信: stream_id = {}",
                                        stream_id
                                    );
                                    for h in &list {
                                        eprintln!(
                                            "  {}: {}",
                                            String::from_utf8_lossy(h.name()),
                                            String::from_utf8_lossy(h.value())
                                        );
                                    }

                                    let response_headers = vec![
                                        quiche::h3::Header::new(b":status", b"200"),
                                        quiche::h3::Header::new(
                                            b"content-type",
                                            b"text/plain; charset=utf-8",
                                        ),
                                    ];
                                    h3.send_response(c, stream_id, &response_headers, false)?;

                                    let body = b"Hello from quiche HTTP/3 server!";
                                    h3.send_body(c, stream_id, body, true)?;
                                    eprintln!("[quiche server] レスポンス送信完了");
                                }
                                Ok((stream_id, quiche::h3::Event::Data)) => {
                                    eprintln!(
                                        "[quiche server] データ受信: stream_id = {}",
                                        stream_id
                                    );
                                }
                                Ok((_, quiche::h3::Event::Finished)) => {
                                    eprintln!("[quiche server] ストリーム終了");
                                }
                                Ok((_, quiche::h3::Event::Reset { .. })) => {
                                    eprintln!("[quiche server] ストリームリセット");
                                }
                                Ok((_, quiche::h3::Event::PriorityUpdate)) => {}
                                Ok((_, quiche::h3::Event::GoAway)) => {
                                    eprintln!("[quiche server] GoAway 受信");
                                }
                                Err(quiche::h3::Error::Done) => break,
                                Err(e) => {
                                    eprintln!("[quiche server] HTTP/3 エラー: {:?}", e);
                                    break;
                                }
                            }
                        }
                    }

                    loop {
                        let (write, send_info) = match c.send(&mut out) {
                            Ok(v) => v,
                            Err(quiche::Error::Done) => break,
                            Err(e) => {
                                eprintln!("[quiche server] send エラー: {:?}", e);
                                break;
                            }
                        };

                        if let Err(e) = socket.send_to(&out[..write], send_info.to).await {
                            eprintln!("[quiche server] send_to エラー: {:?}", e);
                        }
                    }

                    if c.is_closed() {
                        eprintln!("[quiche server] 接続終了");
                        break;
                    }
                }
            }
        }
    }

    Ok(())
}

/// quiche クライアントでリクエストを送信
pub async fn run_quiche_client(port: u16) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>> {
    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    let server_addr: SocketAddr = format!("127.0.0.1:{}", port).parse()?;

    let mut config = quiche::Config::new(quiche::PROTOCOL_VERSION)?;
    config.set_application_protos(&[b"h3"])?;
    config.set_max_idle_timeout(10000);
    config.set_max_recv_udp_payload_size(MAX_DATAGRAM_SIZE);
    config.set_max_send_udp_payload_size(MAX_DATAGRAM_SIZE);
    config.set_initial_max_data(10_000_000);
    config.set_initial_max_stream_data_bidi_local(1_000_000);
    config.set_initial_max_stream_data_bidi_remote(1_000_000);
    config.set_initial_max_stream_data_uni(1_000_000);
    config.set_initial_max_streams_bidi(100);
    config.set_initial_max_streams_uni(100);
    config.set_disable_active_migration(true);
    config.verify_peer(false);

    let scid = quiche::ConnectionId::from_ref(&[0xba; 16]);
    let local_addr = socket.local_addr()?;

    let mut conn = quiche::connect(
        Some("localhost"),
        &scid,
        local_addr,
        server_addr,
        &mut config,
    )?;

    let mut buf = [0u8; 65535];
    let mut out = [0u8; MAX_DATAGRAM_SIZE];

    let (write, send_info) = conn.send(&mut out)?;
    socket.send_to(&out[..write], send_info.to).await?;
    eprintln!("[quiche client] 初期パケット送信");

    let mut h3_conn: Option<quiche::h3::Connection> = None;
    let mut request_sent = false;
    let mut response_body = Vec::new();

    let timeout = tokio::time::sleep(Duration::from_secs(10));
    tokio::pin!(timeout);

    loop {
        tokio::select! {
            biased;

            _ = &mut timeout => {
                eprintln!("[quiche client] タイムアウト");
                break;
            }

            result = socket.recv_from(&mut buf) => {
                let (len, from) = match result {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("[quiche client] recv エラー: {:?}", e);
                        continue;
                    }
                };

                let recv_info = quiche::RecvInfo {
                    to: local_addr,
                    from,
                };

                if let Err(e) = conn.recv(&mut buf[..len], recv_info) {
                    eprintln!("[quiche client] recv エラー: {:?}", e);
                }

                if conn.is_established() && h3_conn.is_none() {
                    let h3_config = quiche::h3::Config::new()?;
                    h3_conn = Some(quiche::h3::Connection::with_transport(&mut conn, &h3_config)?);
                    eprintln!("[quiche client] HTTP/3 接続確立");
                }

                if let Some(ref mut h3) = h3_conn {
                    if !request_sent {
                        let request_headers = vec![
                            quiche::h3::Header::new(b":method", b"GET"),
                            quiche::h3::Header::new(b":path", b"/"),
                            quiche::h3::Header::new(b":scheme", b"https"),
                            quiche::h3::Header::new(b":authority", b"localhost"),
                        ];
                        let stream_id = h3.send_request(&mut conn, &request_headers, true)?;
                        eprintln!("[quiche client] リクエスト送信: stream_id = {}", stream_id);
                        request_sent = true;
                    }

                    loop {
                        match h3.poll(&mut conn) {
                            Ok((stream_id, quiche::h3::Event::Headers { list, .. })) => {
                                eprintln!(
                                    "[quiche client] レスポンスヘッダー: stream_id = {}",
                                    stream_id
                                );
                                for h in &list {
                                    eprintln!(
                                        "  {}: {}",
                                        String::from_utf8_lossy(h.name()),
                                        String::from_utf8_lossy(h.value())
                                    );
                                }
                            }
                            Ok((stream_id, quiche::h3::Event::Data)) => {
                                let mut body_buf = [0u8; 4096];
                                while let Ok(len) = h3.recv_body(&mut conn, stream_id, &mut body_buf)
                                {
                                    response_body.extend_from_slice(&body_buf[..len]);
                                    eprintln!("[quiche client] データ受信: {} bytes", len);
                                }
                            }
                            Ok((_, quiche::h3::Event::Finished)) => {
                                eprintln!("[quiche client] ストリーム終了");
                                return Ok(response_body);
                            }
                            Ok((_, quiche::h3::Event::Reset { .. })) => {
                                eprintln!("[quiche client] ストリームリセット");
                            }
                            Ok((_, quiche::h3::Event::PriorityUpdate)) => {}
                            Ok((_, quiche::h3::Event::GoAway)) => {
                                eprintln!("[quiche client] GoAway 受信");
                            }
                            Err(quiche::h3::Error::Done) => break,
                            Err(e) => {
                                eprintln!("[quiche client] HTTP/3 エラー: {:?}", e);
                                break;
                            }
                        }
                    }
                }

                loop {
                    let (write, send_info) = match conn.send(&mut out) {
                        Ok(v) => v,
                        Err(quiche::Error::Done) => break,
                        Err(e) => {
                            eprintln!("[quiche client] send エラー: {:?}", e);
                            break;
                        }
                    };

                    if let Err(e) = socket.send_to(&out[..write], send_info.to).await {
                        eprintln!("[quiche client] send_to エラー: {:?}", e);
                    }
                }

                if conn.is_closed() {
                    eprintln!("[quiche client] 接続終了");
                    break;
                }
            }
        }
    }

    Ok(response_body)
}

/// 自己署名証明書テスト用の証明書検証無効化
#[derive(Debug)]
struct InsecureCertVerifier;

impl rustls::client::danger::ServerCertVerifier for InsecureCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::aws_lc_rs::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// quinn (h3) サーバーを起動
pub async fn start_quinn_server(
    cert_pem: String,
    key_pem: String,
    port_tx: std::sync::mpsc::Sender<u16>,
    mut shutdown_rx: mpsc::Receiver<()>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    // PEM をパース
    let certs: Vec<CertificateDer<'static>> =
        CertificateDer::pem_slice_iter(cert_pem.as_bytes())
            .collect::<Result<Vec<_>, _>>()?;
    let key: PrivateKeyDer<'static> = PrivateKeyDer::from_pem_slice(key_pem.as_bytes())?;

    // rustls サーバー設定
    let mut tls_config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()?
    .with_no_client_auth()
    .with_single_cert(certs, key)?;
    tls_config.alpn_protocols = vec![b"h3".to_vec()];

    // quinn サーバー設定
    let server_config = quinn::ServerConfig::with_crypto(Arc::new(
        quinn::crypto::rustls::QuicServerConfig::try_from(tls_config)?,
    ));

    let endpoint = quinn::Endpoint::server(server_config, "127.0.0.1:0".parse()?)?;
    let local_addr = endpoint.local_addr()?;
    eprintln!("[quinn server] サーバー起動: port = {}", local_addr.port());
    port_tx.send(local_addr.port()).unwrap();

    let timeout = tokio::time::sleep(Duration::from_secs(10));
    tokio::pin!(timeout);

    tokio::select! {
        biased;

        _ = shutdown_rx.recv() => {
            eprintln!("[quinn server] シャットダウン");
        }

        _ = &mut timeout => {
            eprintln!("[quinn server] タイムアウト");
        }

        incoming = endpoint.accept() => {
            let Some(incoming) = incoming else {
                eprintln!("[quinn server] endpoint closed");
                endpoint.close(0u32.into(), b"done");
                return Ok(());
            };

            let quinn_conn = incoming.await?;
            eprintln!("[quinn server] 接続確立: {:?}", quinn_conn.remote_address());

            let mut h3_conn: h3::server::Connection<h3_quinn::Connection, Bytes> =
                h3::server::builder()
                    .build(h3_quinn::Connection::new(quinn_conn))
                    .await?;

            // リクエスト処理
            match h3_conn.accept().await {
                Ok(Some(resolver)) => {
                    let (req, mut stream) = resolver.resolve_request().await?;
                    eprintln!(
                        "[quinn server] リクエスト受信: {} {}",
                        req.method(),
                        req.uri()
                    );

                    let response = http::Response::builder()
                        .status(200)
                        .header("content-type", "text/plain; charset=utf-8")
                        .body(())
                        .unwrap();

                    stream.send_response(response).await?;
                    stream
                        .send_data(Bytes::from("Hello from quinn HTTP/3 server!"))
                        .await?;
                    stream.finish().await?;
                    eprintln!("[quinn server] レスポンス送信完了");

                    // クライアントがレスポンスを受信するまで待機
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
                Ok(None) => {
                    eprintln!("[quinn server] 接続終了");
                }
                Err(e) => {
                    eprintln!("[quinn server] accept エラー: {:?}", e);
                }
            }
        }
    }

    endpoint.close(0u32.into(), b"done");
    Ok(())
}

/// quinn (h3) クライアントでリクエストを送信
pub async fn run_quinn_client(port: u16) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>> {
    // rustls クライアント設定 (証明書検証無効)
    let mut tls_config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()?
    .dangerous()
    .with_custom_certificate_verifier(Arc::new(InsecureCertVerifier))
    .with_no_client_auth();
    tls_config.alpn_protocols = vec![b"h3".to_vec()];

    // quinn クライアント設定
    let client_config = quinn::ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(tls_config)?,
    ));

    let mut endpoint = quinn::Endpoint::client("0.0.0.0:0".parse()?)?;
    endpoint.set_default_client_config(client_config);

    let server_addr: SocketAddr = format!("127.0.0.1:{}", port).parse()?;
    let quinn_conn = endpoint.connect(server_addr, "localhost")?.await?;
    eprintln!("[quinn client] 接続確立: {:?}", quinn_conn.remote_address());

    let (mut driver, mut send_request) =
        h3::client::new(h3_quinn::Connection::new(quinn_conn)).await?;

    // h3 接続を駆動するタスク
    let driver_task = tokio::spawn(async move {
        let e = std::future::poll_fn(|cx| driver.poll_close(cx)).await;
        eprintln!("[quinn client] driver 終了: {:?}", e);
    });

    // GET リクエスト送信
    let req = http::Request::builder()
        .method("GET")
        .uri(format!("https://localhost:{}/", port))
        .body(())
        .unwrap();

    let mut stream = send_request.send_request(req).await?;
    stream.finish().await?;
    eprintln!("[quinn client] リクエスト送信完了");

    // レスポンス受信
    let response = stream.recv_response().await?;
    eprintln!("[quinn client] レスポンス: status={}", response.status());

    let mut body = Vec::new();
    while let Some(chunk) = stream.recv_data().await? {
        body.extend_from_slice(chunk.chunk());
    }
    eprintln!(
        "[quinn client] ボディ受信: {}",
        String::from_utf8_lossy(&body)
    );

    driver_task.abort();
    endpoint.close(0u32.into(), b"done");
    Ok(body)
}

#[cfg(feature = "tquic-impl")]
mod tquic_impl {

    use super::*;

    // --- tquic ヘルパー ---

    /// tquic 用の UDP パケット送信ハンドラ
    struct TquicPacketSender {
        socket: RefCell<std::net::UdpSocket>,
    }

    impl tquic::PacketSendHandler for TquicPacketSender {
        fn on_packets_send(&self, pkts: &[(Vec<u8>, tquic::PacketInfo)]) -> tquic::Result<usize> {
            let socket = self.socket.borrow();
            let mut sent = 0;
            for (pkt, info) in pkts {
                match socket.send_to(pkt, info.dst) {
                    Ok(_) => sent += 1,
                    Err(_) => break,
                }
            }
            Ok(sent)
        }
    }

    /// tquic サーバー用の TransportHandler
    struct TquicServerHandler {
        h3_conn: Option<tquic::h3::connection::Http3Connection>,
        response_sent: bool,
    }

    impl TquicServerHandler {
        fn new() -> Self {
            Self {
                h3_conn: None,
                response_sent: false,
            }
        }

        fn process_h3_events(&mut self, conn: &mut tquic::Connection) {
            let h3 = match self.h3_conn.as_mut() {
                Some(h3) => h3,
                None => return,
            };

            let mut buf = [0u8; 4096];
            loop {
                match h3.poll(conn) {
                    Ok((stream_id, tquic::h3::Http3Event::Headers { .. })) => {
                        eprintln!("[tquic server] headers received: stream_id = {}", stream_id);
                    }
                    Ok((stream_id, tquic::h3::Http3Event::Data)) => {
                        while h3.recv_body(conn, stream_id, &mut buf).is_ok() {}
                    }
                    Ok((stream_id, tquic::h3::Http3Event::Finished)) => {
                        eprintln!("[tquic server] stream finished: stream_id = {}", stream_id);

                        let response_headers = vec![
                            tquic::h3::Header::new(b":status", b"200"),
                            tquic::h3::Header::new(b"content-type", b"text/plain; charset=utf-8"),
                        ];
                        if let Err(e) = h3.send_headers(conn, stream_id, &response_headers, false) {
                            eprintln!("[tquic server] send_headers error: {:?}", e);
                            return;
                        }

                        let body = b"Hello from tquic HTTP/3 server!";
                        match h3.send_body(conn, stream_id, bytes::Bytes::from_static(body), true) {
                            Ok(_) => {
                                eprintln!("[tquic server] response sent");
                                self.response_sent = true;
                            }
                            Err(e) => {
                                eprintln!("[tquic server] send_body error: {:?}", e);
                            }
                        }
                    }
                    Ok((_, tquic::h3::Http3Event::Reset(_))) => {}
                    Ok((_, tquic::h3::Http3Event::GoAway)) => {}
                    Ok((_, tquic::h3::Http3Event::PriorityUpdate)) => {}
                    Err(tquic::h3::Http3Error::Done) => break,
                    Err(e) => {
                        eprintln!("[tquic server] h3 poll error: {:?}", e);
                        break;
                    }
                }
            }
        }
    }

    impl tquic::TransportHandler for TquicServerHandler {
        fn on_conn_created(&mut self, _conn: &mut tquic::Connection) {
            eprintln!("[tquic server] connection created");
        }

        fn on_conn_established(&mut self, conn: &mut tquic::Connection) {
            eprintln!("[tquic server] connection established");
            let h3_config = tquic::h3::Http3Config::new().unwrap();
            self.h3_conn = Some(
                tquic::h3::connection::Http3Connection::new_with_quic_conn(conn, &h3_config)
                    .unwrap(),
            );
        }

        fn on_conn_closed(&mut self, _conn: &mut tquic::Connection) {
            eprintln!("[tquic server] connection closed");
        }

        fn on_stream_created(&mut self, _conn: &mut tquic::Connection, stream_id: u64) {
            eprintln!("[tquic server] stream created: {}", stream_id);
        }

        fn on_stream_readable(&mut self, conn: &mut tquic::Connection, _stream_id: u64) {
            self.process_h3_events(conn);
        }

        fn on_stream_writable(&mut self, conn: &mut tquic::Connection, stream_id: u64) {
            let _ = conn.stream_want_write(stream_id, false);
        }

        fn on_stream_closed(&mut self, _conn: &mut tquic::Connection, stream_id: u64) {
            eprintln!("[tquic server] stream closed: {}", stream_id);
        }

        fn on_new_token(&mut self, _conn: &mut tquic::Connection, _token: Vec<u8>) {}
    }

    /// tquic サーバーを起動 (ブロッキングスレッドで実行)
    ///
    /// tquic の Endpoint は !Send のため std::thread で実行する。
    pub fn start_tquic_server(
        cert_path: PathBuf,
        key_path: PathBuf,
        port_tx: std::sync::mpsc::Sender<u16>,
        shutdown_rx: std::sync::mpsc::Receiver<()>,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let socket = std::net::UdpSocket::bind("127.0.0.1:0")?;
        let local_addr = socket.local_addr()?;
        eprintln!(
            "[tquic server] server started: port = {}",
            local_addr.port()
        );
        port_tx.send(local_addr.port()).unwrap();
        socket.set_nonblocking(true)?;

        let mut config = tquic::Config::new()?;
        config.set_max_idle_timeout(10000);
        config.set_initial_max_data(10_000_000);
        config.set_initial_max_stream_data_bidi_local(1_000_000);
        config.set_initial_max_stream_data_bidi_remote(1_000_000);
        config.set_initial_max_stream_data_uni(1_000_000);
        config.set_initial_max_streams_bidi(100);
        config.set_initial_max_streams_uni(100);

        let tls_config = tquic::TlsConfig::new_server_config(
            cert_path.to_str().unwrap(),
            key_path.to_str().unwrap(),
            vec![b"h3".to_vec()],
            false,
        )?;
        config.set_tls_config(tls_config);

        let sender = Rc::new(TquicPacketSender {
            socket: RefCell::new(socket.try_clone()?),
        });

        let handler = TquicServerHandler::new();
        let mut endpoint = tquic::Endpoint::new(Box::new(config), true, Box::new(handler), sender);

        let mut recv_buf = vec![0u8; 65535];
        let deadline = Instant::now() + Duration::from_secs(10);

        loop {
            if shutdown_rx.try_recv().is_ok() {
                eprintln!("[tquic server] shutdown");
                break;
            }

            if Instant::now() >= deadline {
                eprintln!("[tquic server] timeout");
                break;
            }

            // パケット受信
            match socket.recv_from(&mut recv_buf) {
                Ok((len, from)) => {
                    let pkt_info = tquic::PacketInfo {
                        src: from,
                        dst: local_addr,
                        time: Instant::now(),
                    };
                    let _ = endpoint.recv(&mut recv_buf[..len], &pkt_info);
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => {
                    eprintln!("[tquic server] recv error: {:?}", e);
                }
            }

            // タイムアウト処理
            endpoint.on_timeout(Instant::now());

            // 接続処理
            let _ = endpoint.process_connections();

            // ポーリング間隔
            let wait = endpoint.timeout().unwrap_or(Duration::from_millis(10));
            std::thread::sleep(wait.min(Duration::from_millis(10)));
        }

        endpoint.close(true);
        Ok(())
    }

    /// tquic クライアントの共有状態
    struct TquicClientState {
        h3_conn: Option<tquic::h3::connection::Http3Connection>,
        request_sent: bool,
        response_body: Vec<u8>,
        finished: bool,
    }

    /// tquic クライアント用の TransportHandler
    struct TquicClientHandler {
        state: Rc<RefCell<TquicClientState>>,
    }

    impl TquicClientHandler {
        fn new(state: Rc<RefCell<TquicClientState>>) -> Self {
            Self { state }
        }
    }

    fn tquic_client_process_h3(state: &mut TquicClientState, conn: &mut tquic::Connection) {
        let h3 = match state.h3_conn.as_mut() {
            Some(h3) => h3,
            None => return,
        };

        // リクエスト送信
        if !state.request_sent {
            match h3.stream_new(conn) {
                Ok(stream_id) => {
                    let request_headers = vec![
                        tquic::h3::Header::new(b":method", b"GET"),
                        tquic::h3::Header::new(b":path", b"/"),
                        tquic::h3::Header::new(b":scheme", b"https"),
                        tquic::h3::Header::new(b":authority", b"localhost"),
                    ];
                    if let Err(e) = h3.send_headers(conn, stream_id, &request_headers, true) {
                        eprintln!("[tquic client] send_headers error: {:?}", e);
                        return;
                    }
                    eprintln!("[tquic client] request sent: stream_id = {}", stream_id);
                    state.request_sent = true;
                }
                Err(e) => {
                    eprintln!("[tquic client] stream_new error: {:?}", e);
                    return;
                }
            }
        }

        // レスポンス受信
        let mut buf = [0u8; 4096];
        loop {
            match h3.poll(conn) {
                Ok((stream_id, tquic::h3::Http3Event::Headers { headers, .. })) => {
                    eprintln!("[tquic client] response headers: stream_id = {}", stream_id);
                    for h in &headers {
                        eprintln!(
                            "  {}: {}",
                            String::from_utf8_lossy(tquic::h3::NameValue::name(h)),
                            String::from_utf8_lossy(tquic::h3::NameValue::value(h))
                        );
                    }
                }
                Ok((stream_id, tquic::h3::Http3Event::Data)) => {
                    while let Ok(len) = h3.recv_body(conn, stream_id, &mut buf) {
                        state.response_body.extend_from_slice(&buf[..len]);
                        eprintln!("[tquic client] data received: {} bytes", len);
                    }
                }
                Ok((_, tquic::h3::Http3Event::Finished)) => {
                    eprintln!("[tquic client] stream finished");
                    state.finished = true;
                    return;
                }
                Ok((_, tquic::h3::Http3Event::Reset(_))) => {
                    state.finished = true;
                    return;
                }
                Ok((_, tquic::h3::Http3Event::GoAway)) => {}
                Ok((_, tquic::h3::Http3Event::PriorityUpdate)) => {}
                Err(tquic::h3::Http3Error::Done) => break,
                Err(e) => {
                    eprintln!("[tquic client] h3 poll error: {:?}", e);
                    break;
                }
            }
        }
    }

    impl tquic::TransportHandler for TquicClientHandler {
        fn on_conn_created(&mut self, _conn: &mut tquic::Connection) {
            eprintln!("[tquic client] connection created");
        }

        fn on_conn_established(&mut self, conn: &mut tquic::Connection) {
            eprintln!("[tquic client] connection established");
            let h3_config = tquic::h3::Http3Config::new().unwrap();
            let mut state = self.state.borrow_mut();
            state.h3_conn = Some(
                tquic::h3::connection::Http3Connection::new_with_quic_conn(conn, &h3_config)
                    .unwrap(),
            );
            tquic_client_process_h3(&mut state, conn);
        }

        fn on_conn_closed(&mut self, _conn: &mut tquic::Connection) {
            eprintln!("[tquic client] connection closed");
        }

        fn on_stream_created(&mut self, _conn: &mut tquic::Connection, _stream_id: u64) {}

        fn on_stream_readable(&mut self, conn: &mut tquic::Connection, _stream_id: u64) {
            tquic_client_process_h3(&mut self.state.borrow_mut(), conn);
        }

        fn on_stream_writable(&mut self, conn: &mut tquic::Connection, stream_id: u64) {
            let _ = conn.stream_want_write(stream_id, false);
            tquic_client_process_h3(&mut self.state.borrow_mut(), conn);
        }

        fn on_stream_closed(&mut self, _conn: &mut tquic::Connection, _stream_id: u64) {}

        fn on_new_token(&mut self, _conn: &mut tquic::Connection, _token: Vec<u8>) {}
    }

    /// tquic クライアントでリクエストを送信 (ブロッキングスレッドで実行)
    ///
    /// tquic の Endpoint は !Send のため std::thread で実行する。
    /// 結果は channel 経由で返す。
    pub fn run_tquic_client(port: u16) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>> {
        let (result_tx, result_rx) = std::sync::mpsc::channel();

        std::thread::spawn(move || {
            let result = run_tquic_client_inner(port);
            let _ = result_tx.send(result);
        });

        result_rx
            .recv_timeout(Duration::from_secs(10))
            .map_err(|_| "tquic client timeout")?
    }

    fn run_tquic_client_inner(port: u16) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>> {
        let socket = std::net::UdpSocket::bind("0.0.0.0:0")?;
        let local_addr = socket.local_addr()?;
        let server_addr: SocketAddr = format!("127.0.0.1:{}", port).parse()?;
        socket.set_nonblocking(true)?;

        let mut config = tquic::Config::new()?;
        config.set_max_idle_timeout(10000);
        config.set_initial_max_data(10_000_000);
        config.set_initial_max_stream_data_bidi_local(1_000_000);
        config.set_initial_max_stream_data_bidi_remote(1_000_000);
        config.set_initial_max_stream_data_uni(1_000_000);
        config.set_initial_max_streams_bidi(100);
        config.set_initial_max_streams_uni(100);

        let mut tls_config = tquic::TlsConfig::new_client_config(vec![b"h3".to_vec()], false)?;
        tls_config.set_verify(false);
        config.set_tls_config(tls_config);

        let sender = Rc::new(TquicPacketSender {
            socket: RefCell::new(socket.try_clone()?),
        });

        let state = Rc::new(RefCell::new(TquicClientState {
            h3_conn: None,
            request_sent: false,
            response_body: Vec::new(),
            finished: false,
        }));

        let handler = TquicClientHandler::new(Rc::clone(&state));
        let mut endpoint = tquic::Endpoint::new(Box::new(config), false, Box::new(handler), sender);

        let _ = endpoint.connect(local_addr, server_addr, Some("localhost"), None, None, None)?;
        let _ = endpoint.process_connections();

        let mut recv_buf = vec![0u8; 65535];
        let deadline = Instant::now() + Duration::from_secs(10);

        loop {
            if Instant::now() >= deadline {
                return Err("tquic client timeout".into());
            }

            // 完了チェック
            if state.borrow().finished {
                let body = state.borrow().response_body.clone();
                endpoint.close(true);
                return Ok(body);
            }

            // パケット受信
            match socket.recv_from(&mut recv_buf) {
                Ok((len, from)) => {
                    let pkt_info = tquic::PacketInfo {
                        src: from,
                        dst: local_addr,
                        time: Instant::now(),
                    };
                    let _ = endpoint.recv(&mut recv_buf[..len], &pkt_info);
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => {
                    eprintln!("[tquic client] recv error: {:?}", e);
                }
            }

            // タイムアウト処理
            endpoint.on_timeout(Instant::now());

            // 接続処理
            let _ = endpoint.process_connections();

            // ポーリング間隔
            let wait = endpoint.timeout().unwrap_or(Duration::from_millis(10));
            std::thread::sleep(wait.min(Duration::from_millis(10)));
        }
    }
} // mod tquic_impl

#[cfg(feature = "tquic-impl")]
pub use tquic_impl::*;
