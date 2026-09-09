//! テスト用の証明書生成ヘルパー

use rcgen::generate_simple_self_signed;

/// テスト用の自己署名証明書を生成する
///
/// 戻り値は (証明書 PEM, 秘密鍵 PEM)。
pub fn generate_certificate() -> (String, String) {
    let subject_alt_names = vec!["localhost".to_string(), "127.0.0.1".to_string()];
    let certified_key =
        generate_simple_self_signed(subject_alt_names).expect("自己署名証明書生成に成功すること");
    (
        certified_key.cert.pem(),
        certified_key.signing_key.serialize_pem(),
    )
}
