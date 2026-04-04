//! WebTransport エコークライアントサンプル

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use shiguredo_http3::webtransport;
use tokio_s2n_quic::{ClientConfig, WtClient};

#[tokio::main]
async fn main() -> tokio_s2n_quic::Result<()> {
    let remote_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 4433);

    let wt = webtransport::Settings::new()
        .wt_enabled(1)
        .enable_webtransport_draft02(true)
        .webtransport_max_sessions_draft07(1)
        .wt_initial_max_streams_bidi(100)
        .wt_initial_max_streams_uni(100)
        .wt_initial_max_data(1048576);
    let config = ClientConfig::new(remote_addr, "localhost")
        .insecure()
        .enable_webtransport(wt);

    let mut session = WtClient::connect(config, "/echo").await?;
    eprintln!("WebTransport セッション確立: id={}", session.session_id());

    // エコーテスト: 双方向ストリームでデータを送受信する
    let mut bi_stream = session.open_bi_stream().await?;
    eprintln!("双方向ストリーム確立: id={}", bi_stream.stream_id());

    let messages = ["hello", "world", "webtransport"];
    for msg in &messages {
        let data = msg.as_bytes().to_vec();
        eprintln!("送信: {msg}");
        bi_stream.send(&data).await?;

        let recv_data = bi_stream.recv().await?;
        let echo = String::from_utf8_lossy(&recv_data);
        eprintln!("受信: {echo}");
        assert_eq!(msg.as_bytes(), recv_data.as_slice(), "エコーが一致しない");
    }
    eprintln!("エコーテスト完了");

    session.close(0, "done").await?;
    eprintln!("クライアント終了");

    Ok(())
}
