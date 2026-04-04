//! WebTransport エコーサーバーサンプル

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use rcgen::generate_simple_self_signed;
use shiguredo_http3::webtransport;
use tokio_s2n_quic::{ServerConfig, WtServer};

fn generate_certificate() -> Result<(String, String), Box<dyn std::error::Error + Send + Sync>> {
    let subject_alt_names = vec!["localhost".to_string(), "127.0.0.1".to_string()];
    let certified_key = generate_simple_self_signed(subject_alt_names)?;
    let cert_pem = certified_key.cert.pem();
    let key_pem = certified_key.signing_key.serialize_pem();
    Ok((cert_pem, key_pem))
}

#[tokio::main]
async fn main() -> tokio_s2n_quic::Result<()> {
    let (cert_pem, key_pem) = generate_certificate()
        .map_err(|e| tokio_s2n_quic::Error::Internal(format!("証明書生成エラー: {e}")))?;

    let listen_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 4433);
    let wt = webtransport::Settings::new()
        .wt_enabled(1)
        .enable_webtransport_draft02(true)
        .webtransport_max_sessions_draft07(1)
        .wt_initial_max_streams_bidi(100)
        .wt_initial_max_streams_uni(100)
        .wt_initial_max_data(1048576);
    let config = ServerConfig::new(listen_addr, &cert_pem, &key_pem).enable_webtransport(wt);

    let mut server = WtServer::bind(config)?;
    eprintln!("WebTransport エコーサーバーを起動しました: https://127.0.0.1:4433");

    loop {
        let session_request = server.accept().await?;
        eprintln!(
            "セッションリクエスト: path={}, authority={}",
            String::from_utf8_lossy(session_request.path()),
            String::from_utf8_lossy(session_request.authority())
        );

        let mut session = session_request.accept().await?;
        eprintln!("セッション確立: id={}", session.session_id());

        tokio::spawn(async move {
            while let Ok(mut bi_stream) = session.accept_bi_stream().await {
                tokio::spawn(async move {
                    while let Ok(data) = bi_stream.recv().await {
                        eprintln!("受信: {}", String::from_utf8_lossy(&data));
                        if let Err(e) = bi_stream.send(&data).await {
                            eprintln!("送信エラー: {e}");
                            break;
                        }
                    }
                });
            }
        });
    }
}
