//! WebTransport エコーサーバーサンプル

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use rcgen::generate_simple_self_signed;
use shiguredo_http3::{VarInt, webtransport};
use tokio_s2n_quic::{ServerConfig, WtServer};

/// 自己署名証明書を生成する
///
/// `WT_CERT_PEM` / `WT_KEY_PEM` が設定されていればそれを読み込む (ブラウザ検証で
/// ECDSA P-256 証明書を使うため)。未設定なら Ed25519 の自己署名証明書を生成する。
///
/// ブラウザの `serverCertificateHashes` は ECDSA P-256 / RSA を要求するため、
/// ブラウザから接続する場合は環境変数で P-256 証明書を渡すこと。
fn generate_certificate() -> Result<(String, String), Box<dyn std::error::Error + Send + Sync>> {
    if let (Ok(cert), Ok(key)) = (std::env::var("WT_CERT_PEM"), std::env::var("WT_KEY_PEM")) {
        return Ok((cert, key));
    }
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

    // 待ち受けポートは `WT_LISTEN_PORT` で変更できる
    let port: u16 = match std::env::var("WT_LISTEN_PORT") {
        Ok(value) => value.parse().map_err(|e| {
            tokio_s2n_quic::Error::Internal(format!("WT_LISTEN_PORT の解析に失敗: {e}"))
        })?,
        Err(_) => 4433,
    };
    let listen_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), port);
    run(listen_addr, cert_pem, key_pem).await
}

/// サーバー本体を起動する
async fn run(
    listen_addr: SocketAddr,
    cert_pem: String,
    key_pem: String,
) -> tokio_s2n_quic::Result<()> {
    let v = |value: u64| VarInt::new(value).expect("WT settings value must fit VarInt");
    let wt = webtransport::Settings::new()
        .wt_enabled(VarInt::from_static(1))
        .enable_webtransport_draft02(true)
        .webtransport_max_sessions_draft07(VarInt::from_static(1))
        .wt_initial_max_streams_bidi(v(100))
        .wt_initial_max_streams_uni(v(100))
        .wt_initial_max_data(v(1_048_576));
    let config = ServerConfig::new(listen_addr, &cert_pem, &key_pem).enable_webtransport(wt);

    let mut server = WtServer::bind(config)?;
    eprintln!("WebTransport エコーサーバーを起動しました: https://{listen_addr}");

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
