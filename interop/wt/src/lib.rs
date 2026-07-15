//! WebTransport 相互運用性テスト用共通ヘルパー
//!
//! tokio-ngtcp2, tokio-s2n-quic の WebTransport 相互運用性テストを支援するためのユーティリティ。

use std::error::Error;
use std::path::PathBuf;
use std::time::Duration;

use rcgen::generate_simple_self_signed;

/// テスト用の WebTransport Settings を作成
pub fn test_wt_settings() -> shiguredo_http3::webtransport::Settings {
    use shiguredo_http3::VarInt;
    let v = |value: u64| VarInt::new(value).expect("WT settings value must fit VarInt");
    shiguredo_http3::webtransport::Settings::new()
        .wt_enabled(VarInt::from_static(1))
        .enable_webtransport_draft02(true)
        .webtransport_max_sessions_draft07(VarInt::from_static(1))
        .wt_initial_max_streams_bidi(v(100))
        .wt_initial_max_streams_uni(v(100))
        .wt_initial_max_data(v(1_048_576))
}

/// 共有証明書を生成 (すべての実装で使用)
pub fn generate_shared_certificate() -> Result<(String, String), Box<dyn Error + Send + Sync>> {
    let subject_alt_names = vec!["localhost".to_string(), "127.0.0.1".to_string()];
    let certified_key = generate_simple_self_signed(subject_alt_names)?;
    let cert_pem = certified_key.cert.pem();
    let key_pem = certified_key.signing_key.serialize_pem();
    Ok((cert_pem, key_pem))
}

/// 証明書をファイルに保存 (tokio-ngtcp2 用)
pub fn save_certificate_files(
    cert_pem: &str,
    key_pem: &str,
) -> Result<(PathBuf, PathBuf), Box<dyn Error + Send + Sync>> {
    // テストの並列実行で競合しないよう、一意のディレクトリ名を生成
    let unique_id = format!(
        "interop_webtransport_test_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    );
    let cert_dir = std::env::temp_dir().join(unique_id);
    std::fs::create_dir_all(&cert_dir)?;

    let cert_path = cert_dir.join("cert.pem");
    let key_path = cert_dir.join("key.pem");

    std::fs::write(&cert_path, cert_pem)?;
    std::fs::write(&key_path, key_pem)?;

    Ok((cert_path, key_path))
}

/// 証明書ファイルをクリーンアップ
pub fn cleanup_certificate_files(cert_path: &PathBuf, key_path: &PathBuf) {
    if let Some(parent) = cert_path.parent() {
        let _ = std::fs::remove_dir_all(parent);
    } else {
        let _ = std::fs::remove_file(cert_path);
        let _ = std::fs::remove_file(key_path);
    }
}

/// QUIC 可変長整数をエンコードする (RFC 9000 Section 16)
///
/// `value` が VarInt 範囲 (`0..=2^62 - 1`) を超える場合は panic する。
pub fn encode_varint(value: u64) -> Vec<u8> {
    let mut buf = Vec::new();
    let v = shiguredo_http3::VarInt::new(value).expect("interop varint value fits in VarInt");
    shiguredo_http3::varint::encode_into_vec(&mut buf, v);
    buf
}

/// QUIC 可変長整数をデコードする (RFC 9000 Section 16)
pub fn decode_varint(data: &[u8]) -> Option<(u64, usize)> {
    shiguredo_http3::varint::decode(data)
        .ok()
        .map(|(v, n)| (v.get(), n))
}

/// WebTransport 双方向ストリームヘッダーをエンコードする
///
/// RFC draft-ietf-webtrans-http3 Section 4.3
/// Signal Value (0x41) + Session ID
pub fn encode_wt_bidi_header(session_id: u64) -> Vec<u8> {
    let mut buf = Vec::new();
    shiguredo_http3::webtransport::StreamHeader::new(session_id)
        .expect("session_id must be a client-initiated bidi stream id")
        .encode_bidirectional(&mut buf);
    buf
}

/// WebTransport 双方向ストリームヘッダーをデコードする
///
/// 成功時は (session_id, consumed_bytes) を返す
pub fn decode_wt_bidi_header(data: &[u8]) -> Option<(u64, usize)> {
    let (header, consumed) =
        shiguredo_http3::webtransport::StreamHeader::decode_bidirectional(data)?;
    Some((header.session_id, consumed))
}

/// WebTransport 単方向ストリームヘッダーをエンコードする
///
/// RFC draft-ietf-webtrans-http3 Section 4.2
/// Stream Type (0x54) + Session ID
pub fn encode_wt_uni_header(session_id: u64) -> Vec<u8> {
    let mut buf = Vec::new();
    shiguredo_http3::webtransport::StreamHeader::new(session_id)
        .expect("session_id must be a client-initiated bidi stream id")
        .encode_unidirectional(&mut buf);
    buf
}

/// WebTransport 単方向ストリームヘッダーをデコードする
///
/// 成功時は (session_id, consumed_bytes) を返す
pub fn decode_wt_uni_header(data: &[u8]) -> Option<(u64, usize)> {
    let (header, consumed) =
        shiguredo_http3::webtransport::StreamHeader::decode_unidirectional(data)?;
    Some((header.session_id, consumed))
}

// --- quinn + h3-webtransport ヘルパー ---

use std::sync::Arc;

use bytes::Bytes;
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};

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

/// quinn (h3-webtransport) WebTransport サーバーを起動
///
/// CONNECT リクエストを受け付けてセッション確立し、双方向ストリームでエコーする。
pub async fn start_quinn_wt_server(
    cert_pem: String,
    key_pem: String,
    port_tx: std::sync::mpsc::Sender<u16>,
    mut shutdown_rx: tokio::sync::mpsc::Receiver<()>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // PEM をパース
    let certs: Vec<CertificateDer<'static>> =
        CertificateDer::pem_slice_iter(cert_pem.as_bytes()).collect::<Result<Vec<_>, _>>()?;
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
    eprintln!("[quinn wt server] started: port = {}", local_addr.port());
    port_tx
        .send(local_addr.port())
        .expect("port channel receiver is alive");

    let timeout = tokio::time::sleep(Duration::from_secs(10));
    tokio::pin!(timeout);

    tokio::select! {
        biased;

        _ = shutdown_rx.recv() => {
            eprintln!("[quinn wt server] shutdown");
        }

        _ = &mut timeout => {
            eprintln!("[quinn wt server] timeout");
        }

        incoming = endpoint.accept() => {
            let Some(incoming) = incoming else {
                endpoint.close(0u32.into(), b"done");
                return Ok(());
            };

            let quinn_conn = incoming.await?;
            eprintln!("[quinn wt server] connection established: {:?}", quinn_conn.remote_address());

            // h3 サーバー接続 (WebTransport 有効)
            let mut h3_conn: h3::server::Connection<_, Bytes> = h3::server::builder()
                .enable_webtransport(true)
                .enable_extended_connect(true)
                .enable_datagram(true)
                .max_webtransport_sessions(1)
                .build(h3_quinn::Connection::new(quinn_conn))
                .await?;

            // CONNECT リクエストを待つ
            match h3_conn.accept().await? {
                Some(resolver) => {
                    let (req, stream) = resolver.resolve_request().await?;
                    eprintln!(
                        "[quinn wt server] request: {} {} {:?}",
                        req.method(),
                        req.uri(),
                        req.extensions().get::<h3::ext::Protocol>()
                    );

                    // WebTransport セッション受付
                    let wt_session =
                        h3_webtransport::server::WebTransportSession::accept(req, stream, h3_conn)
                            .await?;
                    eprintln!(
                        "[quinn wt server] WT session established: session_id = {:?}",
                        wt_session.session_id()
                    );

                    // 双方向ストリームを受け付けてエコー
                    match wt_session.accept_bi().await? {
                        Some(h3_webtransport::server::AcceptedBi::BidiStream(
                            _session_id,
                            mut bidi_stream,
                        )) => {
                            use tokio::io::{AsyncReadExt, AsyncWriteExt};
                            eprintln!("[quinn wt server] bidi stream accepted");
                            let mut data = Vec::new();
                            let mut buf = [0u8; 4096];
                            loop {
                                match bidi_stream.read(&mut buf).await {
                                    Ok(0) => break,
                                    Ok(n) => data.extend_from_slice(&buf[..n]),
                                    Err(_) => break,
                                }
                            }
                            eprintln!(
                                "[quinn wt server] received: {} bytes",
                                data.len()
                            );

                            // エコー
                            bidi_stream.write_all(&data).await?;
                            bidi_stream.shutdown().await?;
                            eprintln!("[quinn wt server] echo sent");
                        }
                        _ => {
                            eprintln!("[quinn wt server] unexpected stream type");
                        }
                    }

                    // クライアントがレスポンスを受信するまで待機
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
                None => {
                    eprintln!("[quinn wt server] no request");
                }
            }
        }
    }

    endpoint.close(0u32.into(), b"done");
    Ok(())
}

/// quinn (h3) クライアントで WebTransport セッション確立を確認する
///
/// CONNECT リクエストを送信し、200 レスポンスが返ることを確認する。
pub async fn run_quinn_wt_client(
    port: u16,
    path: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // rustls クライアント設定 (証明書検証無効)
    let mut tls_config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()?
    .dangerous()
    .with_custom_certificate_verifier(Arc::new(InsecureCertVerifier))
    .with_no_client_auth();
    tls_config.alpn_protocols = vec![b"h3".to_vec()];

    let client_config = quinn::ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(tls_config)?,
    ));

    let mut endpoint = quinn::Endpoint::client("0.0.0.0:0".parse()?)?;
    endpoint.set_default_client_config(client_config);

    let server_addr: std::net::SocketAddr = format!("127.0.0.1:{}", port).parse()?;
    let quinn_conn = endpoint.connect(server_addr, "localhost")?.await?;
    eprintln!(
        "[quinn wt client] connection established: {:?}",
        quinn_conn.remote_address()
    );

    // h3 クライアント接続 (WebTransport 有効)
    let mut builder = h3::client::builder();
    builder.enable_datagram(true).enable_extended_connect(true);

    let (mut driver, mut send_request) = builder
        .build::<_, _, Bytes>(h3_quinn::Connection::new(quinn_conn))
        .await?;

    // h3 接続を駆動するタスク
    let driver_task = tokio::spawn(async move {
        let e = std::future::poll_fn(|cx| driver.poll_close(cx)).await;
        eprintln!("[quinn wt client] driver closed: {:?}", e);
    });

    // CONNECT リクエスト送信 (extended CONNECT with :protocol=webtransport)
    let req = {
        let mut req = http::Request::builder()
            .method("CONNECT")
            .uri(format!("https://localhost:{}{}", port, path))
            .body(())
            .expect("interop setup must succeed");
        req.extensions_mut()
            .insert(h3::ext::Protocol::WEB_TRANSPORT);
        req
    };

    let mut stream = send_request.send_request(req).await?;
    let response = stream.recv_response().await?;
    eprintln!("[quinn wt client] response: status={}", response.status());

    if response.status() != http::StatusCode::OK {
        driver_task.abort();
        endpoint.close(0u32.into(), b"done");
        return Err(format!("unexpected status: {}", response.status()).into());
    }

    eprintln!("[quinn wt client] WT session established");

    driver_task.abort();
    endpoint.close(0u32.into(), b"done");
    Ok(())
}
