//! HTTP/3 クライアントサンプル

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use tokio_s2n_quic::{ClientConfig, H3Client, H3ClientRequest};

#[tokio::main]
async fn main() -> tokio_s2n_quic::Result<()> {
    let remote_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 4433);

    let config = ClientConfig::new(remote_addr, "localhost").insecure();

    let mut client = H3Client::connect(config).await?;
    eprintln!("サーバーに接続しました");

    let request = H3ClientRequest::get(b"/").authority(b"localhost");

    let response = client.send_request(request).await?;

    eprintln!("ステータス: {}", response.status());
    for (name, value) in response.headers() {
        eprintln!(
            "  {}: {}",
            String::from_utf8_lossy(name),
            String::from_utf8_lossy(value)
        );
    }
    eprintln!("ボディ: {}", String::from_utf8_lossy(response.body()));

    Ok(())
}
