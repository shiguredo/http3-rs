use std::path::PathBuf;
use std::sync::Arc;

use base64ct::{Base64, Encoding};
use s2n_quic::provider::tls::rustls as s2n_rustls;

use crate::error::Error;

/// ALPN プロトコル識別子
const ALPN_H3: &[u8] = b"h3";

/// WebTransport の serverCertificateHashes で要求される最大有効期間 (14 日未満)
const CERT_VALIDITY_DAYS: i64 = 13;

/// キャッシュ済み証明書を再利用するために必要な最低残り有効期間
const MIN_REMAINING_HOURS: i64 = 1;

/// キャッシュファイルのパスを返す
fn cache_path() -> PathBuf {
    std::env::temp_dir().join("wt-server-cert.jsonc")
}

/// キャッシュ済み証明書と秘密鍵を読み込む
///
/// JSONC ファイルが存在し、残り有効期間が MIN_REMAINING_HOURS 以上であればそのまま返す。
/// 存在しないか期限切れの場合は None を返す。
fn load_cached_cert() -> Option<(rustls::pki_types::CertificateDer<'static>, Vec<u8>)> {
    let text = std::fs::read_to_string(cache_path()).ok()?;
    let (json, _) = nojson::RawJson::parse_jsonc(&text).ok()?;
    let root = json.value();

    let created_at: i64 = root
        .to_member("created_at")
        .ok()?
        .required()
        .ok()?
        .try_into()
        .ok()?;
    let cert_b64: String = root
        .to_member("cert")
        .ok()?
        .required()
        .ok()?
        .try_into()
        .ok()?;
    let key_b64: String = root
        .to_member("key")
        .ok()?
        .required()
        .ok()?
        .try_into()
        .ok()?;

    // 生成時刻 + 有効期間 = 失効時刻として、残り時間を計算する
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    let expires_at = created_at + CERT_VALIDITY_DAYS * 24 * 3600;
    let remaining_secs = expires_at - now;

    if remaining_secs < MIN_REMAINING_HOURS * 3600 {
        tracing::info!("Cached certificate expires soon, regenerating");
        return None;
    }

    let cert_bytes = Base64::decode_vec(&cert_b64).ok()?;
    let key_bytes = Base64::decode_vec(&key_b64).ok()?;

    let path = cache_path();
    tracing::info!(
        "Using cached certificate {:?} (expires in {:.1} hours)",
        path,
        remaining_secs as f64 / 3600.0
    );

    let cert_der = rustls::pki_types::CertificateDer::from(cert_bytes);
    Some((cert_der, key_bytes))
}

/// 新しい自己署名証明書を生成してキャッシュに保存する
fn generate_and_cache_cert() -> Result<(rustls::pki_types::CertificateDer<'static>, Vec<u8>), Error>
{
    // SAN に localhost / 127.0.0.1 / ::1 を設定する
    let mut params = rcgen::CertificateParams::new(vec!["localhost".to_string()])
        .map_err(|e| Error::Other(format!("certificate params failed: {e}")))?;
    params
        .subject_alt_names
        .push(rcgen::SanType::IpAddress(std::net::IpAddr::V4(
            std::net::Ipv4Addr::LOCALHOST,
        )));
    params
        .subject_alt_names
        .push(rcgen::SanType::IpAddress(std::net::IpAddr::V6(
            std::net::Ipv6Addr::LOCALHOST,
        )));

    // 有効期間を 14 日未満に設定する
    let now = time::OffsetDateTime::now_utc();
    params.not_before = now;
    params.not_after = now + time::Duration::days(CERT_VALIDITY_DAYS);

    // ECDSA P-256 鍵を生成する (Chrome の serverCertificateHashes は Ed25519 非対応)
    let signing_key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
        .map_err(|e| Error::Other(format!("key generation failed: {e}")))?;
    let cert = params
        .self_signed(&signing_key)
        .map_err(|e| Error::Other(format!("certificate generation failed: {e}")))?;

    let cert_der = cert.der().clone();
    let key_bytes = signing_key.serialize_der();

    // JSONC でキャッシュに保存する
    let cert_b64 = Base64::encode_string(cert_der.as_ref());
    let key_b64 = Base64::encode_string(&key_bytes);
    let created_at = now.unix_timestamp();

    let jsonc = format!(
        "// WebTransport server の自己署名証明書キャッシュ (自動生成)\n{}\n",
        nojson::json(|f| {
            f.set_indent_size(2);
            f.set_spacing(true);
            f.object(|f| {
                f.member("created_at", created_at)?;
                f.member("cert", &cert_b64)?;
                f.member("key", &key_b64)
            })
        })
    );

    let path = cache_path();
    std::fs::write(&path, &jsonc)
        .map_err(|e| Error::Other(format!("failed to write cert cache: {e}")))?;

    tracing::info!("Generated new certificate (cached to {:?})", path);

    Ok((cert_der, key_bytes))
}

/// 自己署名証明書を取得して rustls サーバー TLS を構築する
///
/// キャッシュ済み証明書が有効であればそれを使い、なければ新規生成する。
/// ALPN に h3 (WebTransport over HTTP/3) を設定する。
/// WebTransport の serverCertificateHashes に対応するため:
/// - ECDSA P-256 鍵を使用する (Chrome は Ed25519 非対応)
/// - 有効期間を 14 日未満に設定する
/// - SAN に localhost / 127.0.0.1 / ::1 を設定する
/// - 証明書の SHA-256 ハッシュを base64 でログに出力する
pub fn generate_tls_server() -> Result<s2n_rustls::Server, Error> {
    let (cert_der, key_bytes) = match load_cached_cert() {
        Some(cached) => cached,
        None => generate_and_cache_cert()?,
    };

    // 証明書の SHA-256 ハッシュを base64 で出力する (serverCertificateHashes 用)
    let hash = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, cert_der.as_ref());
    let hash_base64 = Base64::encode_string(hash.as_ref());
    tracing::info!("Certificate hash (SHA-256, base64): {hash_base64}");

    let key_der = rustls::pki_types::PrivateKeyDer::Pkcs8(
        rustls::pki_types::PrivatePkcs8KeyDer::from(key_bytes),
    );

    let provider = rustls::crypto::aws_lc_rs::default_provider();
    let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(provider))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| Error::Other(format!("TLS config failed: {e}")))?
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der)
        .map_err(|e| Error::Other(format!("TLS cert failed: {e}")))?;

    config.alpn_protocols = vec![ALPN_H3.to_vec()];

    Ok(s2n_rustls::Server::from(config))
}
