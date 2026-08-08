//! 証明書検証テスト用の CA / 証明書生成ヘルパー

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair};

/// テスト用のルート CA
pub struct TestCa {
    cert_pem: String,
    key: KeyPair,
    params: CertificateParams,
}

impl TestCa {
    /// ルート CA を生成する
    ///
    /// BasicConstraints の CA 制約を Unconstrained (pathLen 制約なし) に設定する。
    pub fn new() -> Self {
        let mut params =
            CertificateParams::new(vec!["test-ca".to_string()]).expect("test must succeed");
        // subject を明示的に設定する。デフォルトでは subject が空になり、
        // サーバー証明書側も空 subject だと subject == issuer で
        // 自己署名と誤判定されるため
        params
            .distinguished_name
            .push(DnType::CommonName, "test-ca");
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let key = KeyPair::generate().expect("test must succeed");
        let cert = params.self_signed(&key).expect("test must succeed");
        Self {
            cert_pem: cert.pem(),
            key,
            params,
        }
    }

    /// CA 証明書の PEM 文字列を取得する
    pub fn cert_pem(&self) -> &str {
        &self.cert_pem
    }

    /// 指定ホスト名のサーバー証明書を CA で署名して生成する
    ///
    /// 戻り値は (証明書 PEM, 秘密鍵 PEM)。
    pub fn issue_server_cert(&self, hostnames: &[String]) -> (String, String) {
        let mut params = CertificateParams::new(hostnames.to_vec()).expect("test must succeed");
        // subject を明示的に設定する (issuer 名と異なる名前にする)
        params.distinguished_name.push(
            DnType::CommonName,
            hostnames.first().map_or("server", |s| s.as_str()),
        );
        let key = KeyPair::generate().expect("test must succeed");
        // issuer は CA のパラメータから作る (サーバー証明書のパラメータから
        // 作ると issuer 名が CA の subject と一致せずチェーン検証に失敗する)
        let issuer = rcgen::Issuer::from_params(&self.params, &self.key);
        let cert = params.signed_by(&key, &issuer).expect("test must succeed");
        (cert.pem(), key.serialize_pem())
    }
}

/// 自己署名のサーバー証明書を生成する
///
/// 戻り値は (証明書 PEM, 秘密鍵 PEM)。
pub fn generate_self_signed(hostnames: &[String]) -> (String, String) {
    let params = CertificateParams::new(hostnames.to_vec()).expect("test must succeed");
    let key = KeyPair::generate().expect("test must succeed");
    let cert = params.self_signed(&key).expect("test must succeed");
    (cert.pem(), key.serialize_pem())
}

/// PEM 文字列を一時ファイルに書き込む
///
/// テストごとにユニークなディレクトリを作成する。戻り値は (証明書パス, 鍵パス)。
pub fn write_temp_pem(cert_pem: &str, key_pem: &str) -> (PathBuf, PathBuf) {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let unique_id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let cert_dir = std::env::temp_dir().join(format!(
        "tokio_ngtcp2_cert_test_{}_{}",
        std::process::id(),
        unique_id
    ));
    std::fs::create_dir_all(&cert_dir).expect("test must succeed");
    let cert_path = cert_dir.join("cert.pem");
    let key_path = cert_dir.join("key.pem");
    std::fs::write(&cert_path, cert_pem).expect("test must succeed");
    std::fs::write(&key_path, key_pem).expect("test must succeed");
    (cert_path, key_path)
}
