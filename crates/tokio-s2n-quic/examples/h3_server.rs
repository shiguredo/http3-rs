//! HTTP/3 サーバーサンプル

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use rcgen::generate_simple_self_signed;
use tokio_s2n_quic::{H3Response, H3Server, ServerConfig};

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
    let config = ServerConfig::new(listen_addr, &cert_pem, &key_pem);

    let mut server = H3Server::bind(config)?;
    eprintln!("HTTP/3 サーバーを起動しました: https://127.0.0.1:4433");

    loop {
        let mut conn = server.accept().await?;
        tokio::spawn(async move {
            while let Ok(request) = conn.accept_request().await {
                eprintln!(
                    "リクエスト: {} {}",
                    String::from_utf8_lossy(request.method()),
                    String::from_utf8_lossy(request.path())
                );

                let response = H3Response::new(200)
                    .header(b"content-type", b"text/plain; charset=utf-8")
                    .body(b"Hello from HTTP/3 server!");

                if let Err(e) = request.send_response(response).await {
                    eprintln!("レスポンス送信エラー: {e}");
                }
            }
        });
    }
}
