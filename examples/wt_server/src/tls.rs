//! WebTransport サーバー用の TLS 証明書生成とキャッシュ
//!
//! 自己署名証明書と秘密鍵を一時ディレクトリ配下にキャッシュする。キャッシュには
//! 秘密鍵が含まれるため、専用ディレクトリを 0700、キャッシュファイルを 0600 で
//! 作成する。また、共有の一時ディレクトリに固定名でファイルを作る symlink 攻撃
//! (TOCTOU を含む) を避けるため、書き込みはシンボリックリンクを辿らない手順で行う。
//!
//! 旧バージョンは一時ディレクトリ直下に 0644 相当で秘密鍵入りキャッシュを書いて
//! いたため、初回アクセス時に旧キャッシュを best-effort で削除する。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Once};

use base64ct::{Base64, Encoding};
use s2n_quic::provider::tls::rustls as s2n_rustls;

use crate::error::Error;

/// ALPN プロトコル識別子
const ALPN_H3: &[u8] = b"h3";

/// WebTransport の serverCertificateHashes で要求される最大有効期間 (14 日未満)
const CERT_VALIDITY_DAYS: i64 = 13;

/// キャッシュ済み証明書を再利用するために必要な最低残り有効期間
const MIN_REMAINING_HOURS: i64 = 1;

/// キャッシュディレクトリ名
///
/// キャッシュには秘密鍵が含まれるため、一時ディレクトリ直下にファイルを置かず、
/// 専用ディレクトリにまとめてディレクトリ単位でパーミッションを管理する。
const CACHE_DIR_NAME: &str = "wt-server";

/// キャッシュファイル名
const CACHE_FILE_NAME: &str = "wt-server-cert.jsonc";

/// 旧バージョンのキャッシュファイル名
///
/// 旧バージョンは一時ディレクトリ直下にこの名前で秘密鍵入りキャッシュを書いていた
/// (パーミッションは 0644 相当)。現在は参照しないが、秘密鍵が残り続けないよう
/// 初回アクセス時に削除する。
const LEGACY_CACHE_FILE_NAME: &str = "wt-server-cert.jsonc";

/// 旧バージョンのキャッシュファイルのパスを返す
///
/// 一時ディレクトリの場所は環境変数 `TMPDIR` 等で決まるため、`temp_dir` を引数で
/// 受け取る (単体テストで差し替えられる)。
fn legacy_cache_path(temp_dir: &Path) -> PathBuf {
    temp_dir.join(LEGACY_CACHE_FILE_NAME)
}

/// 旧バージョンのキャッシュファイルを best-effort で削除する
///
/// シンボリックリンクの場合はリンク自体を削除し、リンク先には触らない。
/// 削除できなかった場合は警告ログのみを出して処理を継続する (新しいキャッシュは
/// 別ディレクトリに作られるため、起動を妨げる必要はない)。
fn remove_legacy_cache_in(temp_dir: &Path) {
    let path = legacy_cache_path(temp_dir);
    match std::fs::symlink_metadata(&path) {
        Ok(_) => match std::fs::remove_file(&path) {
            Ok(()) => tracing::info!("Removed legacy certificate cache {:?}", path),
            Err(e) => tracing::warn!("Failed to remove legacy certificate cache {:?}: {e}", path),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::debug!("Legacy certificate cache not found at {:?}", path);
        }
        Err(e) => {
            tracing::warn!("Failed to inspect legacy certificate cache {:?}: {e}", path);
        }
    }
}

/// 旧バージョンのキャッシュファイルの削除をプロセス内で 1 回だけ行う
///
/// キャッシュの読み書きは起動時に 1 回だけ行われるが、将来呼び出し経路が増えても
/// 削除が繰り返されないよう `Once` で保護する。
fn remove_legacy_cache_once() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| remove_legacy_cache_in(&std::env::temp_dir()));
}

/// キャッシュディレクトリとキャッシュファイルのパスを返す
///
/// 一時ディレクトリの場所は環境変数 `TMPDIR` 等で決まるため、固定パスを
/// ハードコードせず `temp_dir` を引数で受け取る (単体テストで差し替えられる)。
fn cache_paths(temp_dir: &Path) -> (PathBuf, PathBuf) {
    let dir = temp_dir.join(CACHE_DIR_NAME);
    let file = dir.join(CACHE_FILE_NAME);
    (dir, file)
}

/// キャッシュディレクトリとして安全なメタデータかどうか
///
/// シンボリックリンク・通常ファイルを拒否する。unix ではパーミッションが
/// 0700 であることも要求する。unix 以外には所有者・パーミッションの概念が
/// 無いため種別のみを検査する。
fn is_secure_cache_dir(metadata: &std::fs::Metadata) -> bool {
    let file_type = metadata.file_type();
    if file_type.is_symlink() || !file_type.is_dir() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // ファイル種別ビットを除いたパーミッション部分だけを比較する
        if metadata.permissions().mode() & 0o777 != 0o700 {
            return false;
        }
    }
    true
}

/// キャッシュファイルとして安全なメタデータかどうか
///
/// シンボリックリンク・ディレクトリを拒否する。unix ではパーミッションが
/// 0600 であることも要求する。
fn is_secure_cache_file(metadata: &std::fs::Metadata) -> bool {
    let file_type = metadata.file_type();
    if file_type.is_symlink() || !file_type.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o777 != 0o600 {
            return false;
        }
    }
    true
}

/// キャッシュディレクトリのパーミッションを 0700 にする
#[cfg(unix)]
fn set_cache_dir_mode(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
}

/// キャッシュディレクトリのパーミッションを 0700 にする
///
/// unix 以外には POSIX パーミッションが無いため何もしない。
#[cfg(not(unix))]
fn set_cache_dir_mode(_dir: &Path) -> std::io::Result<()> {
    Ok(())
}

/// キャッシュディレクトリを 0700 で作成する
fn create_cache_dir(dir: &Path) -> Result<(), Error> {
    #[cfg(unix)]
    let builder = {
        use std::os::unix::fs::DirBuilderExt;

        // 作成の瞬間から 0700 にして、生成直後の短い期間にも他ユーザーから
        // 読み書きできないようにする
        let mut builder = std::fs::DirBuilder::new();
        builder.mode(0o700);
        builder
    };
    #[cfg(not(unix))]
    let builder = std::fs::DirBuilder::new();

    builder
        .create(dir)
        .map_err(|e| Error::Other(format!("failed to create cert cache directory: {e}")))?;
    // umask によっては 0700 より狭いパーミッションになるため、明示的に設定する
    set_cache_dir_mode(dir)
        .map_err(|e| Error::Other(format!("failed to set cert cache directory mode: {e}")))
}

/// キャッシュディレクトリを安全な状態に整える
///
/// パスがシンボリックリンクや通常ファイルの場合は削除して作り直す。既存の
/// ディレクトリのパーミッションが緩い場合は 0700 に締め直す。締め直せない場合
/// (他ユーザー所有のディレクトリ等) は、安全に書き込めないためエラーにする。
fn ensure_cache_dir(dir: &Path) -> Result<(), Error> {
    match std::fs::symlink_metadata(dir) {
        Ok(metadata) => {
            if is_secure_cache_dir(&metadata) {
                return Ok(());
            }
            if metadata.file_type().is_dir() {
                // 既存ディレクトリは所有者・パーミッションだけを締め直す
                set_cache_dir_mode(dir).map_err(|e| {
                    Error::Other(format!("failed to secure cert cache directory: {e}"))
                })?;
                // 締め直した結果を再確認する (他ユーザー所有だと失敗するか 0700 にならない)
                let metadata = std::fs::symlink_metadata(dir).map_err(|e| {
                    Error::Other(format!("failed to inspect cert cache directory: {e}"))
                })?;
                if is_secure_cache_dir(&metadata) {
                    Ok(())
                } else {
                    Err(Error::Other(
                        "cert cache directory permissions are not 0700".to_string(),
                    ))
                }
            } else {
                // シンボリックリンク・通常ファイルはパス自体を削除して作り直す。
                // remove_file はシンボリックリンクを辿らずリンクだけを削除する。
                std::fs::remove_file(dir).map_err(|e| {
                    Error::Other(format!("failed to remove stale cert cache path: {e}"))
                })?;
                create_cache_dir(dir)
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => create_cache_dir(dir),
        Err(e) => Err(Error::Other(format!(
            "failed to inspect cert cache directory: {e}"
        ))),
    }
}

/// キャッシュファイルを 0600 で書き込む
///
/// 既存ファイル (シンボリックリンクを含む) を削除してから `create_new` で
/// 作り直す。`create_new` は O_EXCL 付きで開くため、削除直後に別プロセスが
/// 同名のファイルやシンボリックリンクを作っても、それを辿って書き込むことはない
/// (作成に失敗してエラーになる)。
fn write_cache_file(dir: &Path, contents: &str) -> Result<(), Error> {
    let path = dir.join(CACHE_FILE_NAME);
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) => {
            let file_type = metadata.file_type();
            if !file_type.is_file() && !file_type.is_symlink() {
                return Err(Error::Other(format!(
                    "unexpected cert cache file type: {path:?}"
                )));
            }
            std::fs::remove_file(&path).map_err(|e| {
                Error::Other(format!("failed to remove stale cert cache file: {e}"))
            })?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(Error::Other(format!(
                "failed to inspect cert cache file: {e}"
            )));
        }
    }

    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        // 作成の瞬間から 0600 にする
        options.mode(0o600);
    }
    let mut file = options
        .open(&path)
        .map_err(|e| Error::Other(format!("failed to create cert cache file: {e}")))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        // umask によっては 0600 より狭いパーミッションになるため、明示的に設定する
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|e| Error::Other(format!("failed to set cert cache file mode: {e}")))?;
    }
    file.write_all(contents.as_bytes())
        .map_err(|e| Error::Other(format!("failed to write cert cache file: {e}")))?;
    Ok(())
}

/// キャッシュファイルを読み込む
///
/// ディレクトリとファイルが安全な種別・パーミッションでない場合は `None` を
/// 返す。呼び出し側は証明書を再生成する (再生成時に安全な状態へ作り直される)。
fn read_cache_file(temp_dir: &Path) -> Option<(PathBuf, String)> {
    let (dir, path) = cache_paths(temp_dir);
    let dir_metadata = std::fs::symlink_metadata(&dir).ok()?;
    if !is_secure_cache_dir(&dir_metadata) {
        return None;
    }
    let metadata = std::fs::symlink_metadata(&path).ok()?;
    if !is_secure_cache_file(&metadata) {
        return None;
    }
    let text = std::fs::read_to_string(&path).ok()?;
    Some((path, text))
}

/// キャッシュ済み証明書と秘密鍵を読み込む
///
/// JSONC ファイルが存在し、残り有効期間が MIN_REMAINING_HOURS 以上であればそのまま返す。
/// 存在しないか期限切れの場合は None を返す。
fn load_cached_cert() -> Option<(rustls::pki_types::CertificateDer<'static>, Vec<u8>)> {
    load_cached_cert_from(&std::env::temp_dir())
}

/// `temp_dir` 配下のキャッシュから証明書と秘密鍵を読み込む
fn load_cached_cert_from(
    temp_dir: &Path,
) -> Option<(rustls::pki_types::CertificateDer<'static>, Vec<u8>)> {
    let (path, text) = read_cache_file(temp_dir)?;
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
    generate_and_cache_cert_in(&std::env::temp_dir())
}

/// `temp_dir` 配下のキャッシュに新しい自己署名証明書を保存する
fn generate_and_cache_cert_in(
    temp_dir: &Path,
) -> Result<(rustls::pki_types::CertificateDer<'static>, Vec<u8>), Error> {
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

    let (dir, path) = cache_paths(temp_dir);
    ensure_cache_dir(&dir)?;
    write_cache_file(&dir, &jsonc)?;

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
    // 旧バージョンが一時ディレクトリ直下に書いた秘密鍵入りキャッシュを初回に除去する
    remove_legacy_cache_once();
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

#[cfg(test)]
mod tests {
    use super::*;

    /// テスト用の一時ディレクトリを作成する
    ///
    /// テストごとに名前を変えて並列実行時の衝突を避ける。呼び出し側が
    /// [`cleanup`] で後始末する。
    fn test_temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("wt-server-test-{}-{name}", std::process::id()));
        // 前回のテストが異常終了して残ったディレクトリを除去する
        if dir.exists() {
            std::fs::remove_dir_all(&dir).expect("テスト用一時ディレクトリを削除できること");
        }
        std::fs::create_dir_all(&dir).expect("テスト用一時ディレクトリを作成できること");
        dir
    }

    /// テスト用の一時ディレクトリを後始末する
    fn cleanup(dir: &Path) {
        std::fs::remove_dir_all(dir).expect("テスト用一時ディレクトリを削除できること");
    }

    /// テスト用に指定パーミッションのディレクトリを作成する
    #[cfg(unix)]
    fn create_dir_with_mode(path: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;

        std::fs::create_dir(path).expect("テスト用ディレクトリを作成できること");
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
            .expect("テスト用ディレクトリのパーミッションを設定できること");
    }

    /// テスト用に指定パーミッションのファイルを作成する
    #[cfg(unix)]
    fn create_file_with_mode(path: &Path, contents: &str, mode: u32) {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true).mode(mode);
        let mut file = options
            .open(path)
            .expect("テスト用ファイルを作成できること");
        file.write_all(contents.as_bytes())
            .expect("テスト用ファイルに書き込めること");
        file.set_permissions(std::fs::Permissions::from_mode(mode))
            .expect("テスト用ファイルのパーミッションを設定できること");
    }

    #[test]
    fn test_cache_paths() {
        // 一時ディレクトリ配下の専用ディレクトリとファイル名を返すこと
        let base = Path::new("/tmp/example");
        let (dir, file) = cache_paths(base);
        assert_eq!(dir, Path::new("/tmp/example/wt-server"));
        assert_eq!(
            file,
            Path::new("/tmp/example/wt-server/wt-server-cert.jsonc")
        );
        // ファイルは必ずディレクトリの直下にあること
        assert_eq!(file.parent().expect("ファイルには親がある"), dir);
    }

    #[test]
    fn test_is_secure_cache_dir_rejects_loose_permissions() {
        let base = test_temp_dir("dir-loose-permissions");
        let (dir, _) = cache_paths(&base);

        // 0700 のディレクトリは安全と判定されること
        #[cfg(unix)]
        create_dir_with_mode(&dir, 0o700);
        #[cfg(not(unix))]
        std::fs::create_dir(&dir).expect("テスト用ディレクトリを作成できること");
        let metadata = std::fs::symlink_metadata(&dir).expect("メタデータを取得できること");
        assert!(
            is_secure_cache_dir(&metadata),
            "0700 のディレクトリは安全と判定されること"
        );

        // 0755 のディレクトリは安全でないと判定されること
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755))
                .expect("パーミッションを設定できること");
            let metadata = std::fs::symlink_metadata(&dir).expect("メタデータを取得できること");
            assert!(
                !is_secure_cache_dir(&metadata),
                "0755 のディレクトリは安全でないと判定されること"
            );
        }

        cleanup(&base);
    }

    #[test]
    fn test_is_secure_cache_dir_rejects_regular_file() {
        let base = test_temp_dir("dir-regular-file");
        let (dir, _) = cache_paths(&base);

        // ディレクトリのパスに通常ファイルがある場合は安全でないと判定されること
        std::fs::write(&dir, "not a directory").expect("テスト用ファイルを作成できること");
        let metadata = std::fs::symlink_metadata(&dir).expect("メタデータを取得できること");
        assert!(
            !is_secure_cache_dir(&metadata),
            "通常ファイルはディレクトリとして扱わないこと"
        );

        cleanup(&base);
    }

    #[test]
    fn test_is_secure_cache_dir_rejects_symlink() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let base = test_temp_dir("dir-symlink");
            let (dir, _) = cache_paths(&base);
            let target = base.join("target");
            create_dir_with_mode(&target, 0o700);

            // シンボリックリンクは安全でないと判定されること
            symlink(&target, &dir).expect("テスト用シンボリックリンクを作成できること");
            let metadata = std::fs::symlink_metadata(&dir).expect("メタデータを取得できること");
            assert!(
                !is_secure_cache_dir(&metadata),
                "シンボリックリンクはディレクトリとして扱わないこと"
            );

            cleanup(&base);
        }
    }

    #[test]
    fn test_ensure_cache_dir_creates_private_directory() {
        let base = test_temp_dir("ensure-create");
        let (dir, _) = cache_paths(&base);

        ensure_cache_dir(&dir).expect("キャッシュディレクトリを作成できること");

        let metadata = std::fs::symlink_metadata(&dir).expect("メタデータを取得できること");
        assert!(
            !metadata.file_type().is_symlink(),
            "シンボリックリンクでないこと"
        );
        assert!(metadata.file_type().is_dir(), "ディレクトリであること");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                metadata.permissions().mode() & 0o777,
                0o700,
                "パーミッションが 0700 であること"
            );
        }

        // 2 回目は既存ディレクトリをそのまま使えること
        ensure_cache_dir(&dir).expect("既存のキャッシュディレクトリを再利用できること");

        cleanup(&base);
    }

    #[test]
    fn test_ensure_cache_dir_secures_loose_directory() {
        #[cfg(unix)]
        {
            let base = test_temp_dir("ensure-secure-loose");
            let (dir, _) = cache_paths(&base);
            create_dir_with_mode(&dir, 0o755);

            ensure_cache_dir(&dir).expect("緩いパーミッションのディレクトリを締め直せること");

            use std::os::unix::fs::PermissionsExt;
            let metadata = std::fs::symlink_metadata(&dir).expect("メタデータを取得できること");
            assert_eq!(
                metadata.permissions().mode() & 0o777,
                0o700,
                "パーミッションが 0700 に締め直されること"
            );

            cleanup(&base);
        }
    }

    #[test]
    fn test_ensure_cache_dir_replaces_symlink() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let base = test_temp_dir("ensure-replace-symlink");
            let (dir, _) = cache_paths(&base);
            let target = base.join("target");
            create_dir_with_mode(&target, 0o700);
            symlink(&target, &dir).expect("テスト用シンボリックリンクを作成できること");

            ensure_cache_dir(&dir).expect("シンボリックリンクを実ディレクトリに置き換えられること");

            let metadata = std::fs::symlink_metadata(&dir).expect("メタデータを取得できること");
            assert!(
                !metadata.file_type().is_symlink(),
                "シンボリックリンクでないこと"
            );
            assert!(metadata.file_type().is_dir(), "ディレクトリであること");
            // リンク先のディレクトリは変更されないこと
            let target_metadata =
                std::fs::symlink_metadata(&target).expect("メタデータを取得できること");
            assert!(
                target_metadata.file_type().is_dir(),
                "リンク先のディレクトリが残っていること"
            );

            cleanup(&base);
        }
    }

    #[test]
    fn test_is_secure_cache_file_rejects_loose_permissions() {
        #[cfg(unix)]
        {
            let base = test_temp_dir("file-loose-permissions");
            let (dir, file_path) = cache_paths(&base);
            ensure_cache_dir(&dir).expect("キャッシュディレクトリを作成できること");

            // 0600 のファイルは安全と判定されること
            create_file_with_mode(&file_path, "{}", 0o600);
            let metadata =
                std::fs::symlink_metadata(&file_path).expect("メタデータを取得できること");
            assert!(
                is_secure_cache_file(&metadata),
                "0600 のファイルは安全と判定されること"
            );

            // 0644 のファイルは安全でないと判定されること
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&file_path, std::fs::Permissions::from_mode(0o644))
                .expect("パーミッションを設定できること");
            let metadata =
                std::fs::symlink_metadata(&file_path).expect("メタデータを取得できること");
            assert!(
                !is_secure_cache_file(&metadata),
                "0644 のファイルは安全でないと判定されること"
            );

            cleanup(&base);
        }
    }

    #[test]
    fn test_write_cache_file_creates_private_file() {
        let base = test_temp_dir("write-create");
        let (dir, path) = cache_paths(&base);
        ensure_cache_dir(&dir).expect("キャッシュディレクトリを作成できること");

        write_cache_file(&dir, "{\"key\": \"value\"}").expect("キャッシュファイルを書き込めること");

        let metadata = std::fs::symlink_metadata(&path).expect("メタデータを取得できること");
        assert!(
            !metadata.file_type().is_symlink(),
            "シンボリックリンクでないこと"
        );
        assert!(metadata.file_type().is_file(), "通常ファイルであること");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                metadata.permissions().mode() & 0o777,
                0o600,
                "パーミッションが 0600 であること"
            );
        }
        assert_eq!(
            std::fs::read_to_string(&path).expect("キャッシュファイルを読み込めること"),
            "{\"key\": \"value\"}",
            "書き込んだ内容が読み出せること"
        );

        cleanup(&base);
    }

    #[test]
    fn test_write_cache_file_replaces_existing_file() {
        let base = test_temp_dir("write-replace");
        let (dir, path) = cache_paths(&base);
        ensure_cache_dir(&dir).expect("キャッシュディレクトリを作成できること");

        write_cache_file(&dir, "first").expect("1 回目の書き込みができること");
        // 既存ファイルがあっても削除して作り直せること (再生成経路)
        write_cache_file(&dir, "second").expect("2 回目の書き込みができること");

        assert_eq!(
            std::fs::read_to_string(&path).expect("キャッシュファイルを読み込めること"),
            "second",
            "2 回目の内容で上書きされること"
        );
        cleanup(&base);
    }

    #[test]
    fn test_write_cache_file_does_not_follow_symlink() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let base = test_temp_dir("write-symlink");
            let (dir, path) = cache_paths(&base);
            ensure_cache_dir(&dir).expect("キャッシュディレクトリを作成できること");
            let target = base.join("target");
            std::fs::write(&target, "do not overwrite").expect("リンク先ファイルを作成できること");
            symlink(&target, &path).expect("テスト用シンボリックリンクを作成できること");

            write_cache_file(&dir, "new contents").expect("キャッシュファイルを書き込めること");

            // リンク先は書き換えられず、キャッシュパスは通常ファイルになること
            assert_eq!(
                std::fs::read_to_string(&target).expect("リンク先ファイルを読み込めること"),
                "do not overwrite",
                "シンボリックリンクのリンク先を書き換えないこと"
            );
            let metadata = std::fs::symlink_metadata(&path).expect("メタデータを取得できること");
            assert!(
                !metadata.file_type().is_symlink(),
                "シンボリックリンクでないこと"
            );
            assert_eq!(
                std::fs::read_to_string(&path).expect("キャッシュファイルを読み込めること"),
                "new contents",
                "キャッシュファイルに書き込まれること"
            );

            cleanup(&base);
        }
    }

    #[test]
    fn test_read_cache_file_returns_none_for_missing_file() {
        let base = test_temp_dir("read-missing");
        let (dir, _) = cache_paths(&base);

        // ディレクトリもファイルも無い場合は None を返すこと
        assert!(
            read_cache_file(&base).is_none(),
            "キャッシュが無い場合は None を返すこと"
        );

        // ディレクトリだけある場合も None を返すこと
        ensure_cache_dir(&dir).expect("キャッシュディレクトリを作成できること");
        assert!(
            read_cache_file(&base).is_none(),
            "キャッシュファイルが無い場合は None を返すこと"
        );

        cleanup(&base);
    }

    #[test]
    fn test_read_cache_file_roundtrip() {
        let base = test_temp_dir("read-roundtrip");
        let (dir, path) = cache_paths(&base);
        ensure_cache_dir(&dir).expect("キャッシュディレクトリを作成できること");
        write_cache_file(&dir, "roundtrip").expect("キャッシュファイルを書き込めること");

        let (read_path, text) = read_cache_file(&base).expect("キャッシュファイルを読み込めること");
        assert_eq!(read_path, path, "読み込んだパスが一致すること");
        assert_eq!(text, "roundtrip", "読み込んだ内容が一致すること");

        cleanup(&base);
    }

    #[test]
    fn test_read_cache_file_rejects_loose_permissions() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let base = test_temp_dir("read-loose-permissions");
            let (dir, path) = cache_paths(&base);
            ensure_cache_dir(&dir).expect("キャッシュディレクトリを作成できること");
            write_cache_file(&dir, "secret").expect("キャッシュファイルを書き込めること");

            // パーミッションが緩いファイルは読み込まないこと (再生成させる)
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
                .expect("パーミッションを設定できること");
            assert!(
                read_cache_file(&base).is_none(),
                "0644 のキャッシュファイルは読み込まないこと"
            );

            cleanup(&base);
        }
    }

    #[test]
    fn test_generate_and_cache_cert_roundtrip() {
        let base = test_temp_dir("generate-roundtrip");

        // 証明書を生成してキャッシュに保存できること
        let (cert_der, key_bytes) =
            generate_and_cache_cert_in(&base).expect("証明書を生成できること");
        let (_, path) = cache_paths(&base);
        let metadata = std::fs::symlink_metadata(&path).expect("キャッシュファイルがあること");
        assert!(
            metadata.file_type().is_file(),
            "キャッシュが通常ファイルであること"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                metadata.permissions().mode() & 0o777,
                0o600,
                "キャッシュファイルのパーミッションが 0600 であること"
            );
        }

        // 保存したキャッシュを読み込むと生成した証明書と秘密鍵が一致すること
        let (cached_cert, cached_key) =
            load_cached_cert_from(&base).expect("キャッシュを読み込めること");
        assert_eq!(
            cached_cert.as_ref(),
            cert_der.as_ref(),
            "キャッシュされた証明書が生成した証明書と一致すること"
        );
        assert_eq!(
            cached_key, key_bytes,
            "キャッシュされた秘密鍵が生成した秘密鍵と一致すること"
        );

        cleanup(&base);
    }

    #[test]
    fn test_load_cached_cert_rejects_expired_or_soon_expiring() {
        let base = test_temp_dir("load-expiry");
        let (dir, _) = cache_paths(&base);
        ensure_cache_dir(&dir).expect("キャッシュディレクトリを作成できること");
        let now = time::OffsetDateTime::now_utc().unix_timestamp();

        // 有効期間 (13 日) を過ぎたキャッシュは再利用しないこと
        let expired_at = now - (CERT_VALIDITY_DAYS + 1) * 24 * 3600;
        let expired =
            format!("{{\"created_at\": {expired_at}, \"cert\": \"AA==\", \"key\": \"AA==\"}}");
        write_cache_file(&dir, &expired).expect("キャッシュファイルを書き込めること");
        assert!(
            load_cached_cert_from(&base).is_none(),
            "期限切れのキャッシュは再利用しないこと"
        );

        // 残り有効期間が MIN_REMAINING_HOURS 未満のキャッシュも再利用しないこと
        let almost_expired_at =
            now - CERT_VALIDITY_DAYS * 24 * 3600 + (MIN_REMAINING_HOURS * 3600) / 2;
        let almost_expired = format!(
            "{{\"created_at\": {almost_expired_at}, \"cert\": \"AA==\", \"key\": \"AA==\"}}"
        );
        write_cache_file(&dir, &almost_expired).expect("キャッシュファイルを書き込めること");
        assert!(
            load_cached_cert_from(&base).is_none(),
            "まもなく期限切れのキャッシュは再利用しないこと"
        );

        cleanup(&base);
    }

    #[test]
    fn test_legacy_cache_path() {
        // 旧キャッシュは一時ディレクトリ直下の固定名であること
        let base = Path::new("/tmp/example");
        let path = legacy_cache_path(base);
        assert_eq!(path, Path::new("/tmp/example/wt-server-cert.jsonc"));
    }

    #[test]
    fn test_remove_legacy_cache_in_removes_file() {
        let base = test_temp_dir("legacy-remove-file");
        let path = legacy_cache_path(&base);
        std::fs::write(&path, "legacy secret").expect("旧キャッシュファイルを作成できること");

        remove_legacy_cache_in(&base);

        assert!(
            std::fs::symlink_metadata(&path).is_err(),
            "旧キャッシュファイルが削除されること"
        );
        // 一時ディレクトリ自体は残すこと
        assert!(base.is_dir(), "一時ディレクトリは削除しないこと");

        cleanup(&base);
    }

    #[test]
    fn test_remove_legacy_cache_in_keeps_directory() {
        let base = test_temp_dir("legacy-keep-directory");
        let path = legacy_cache_path(&base);
        // パスがディレクトリの場合は削除せず、警告ログのみで継続すること
        std::fs::create_dir(&path).expect("旧キャッシュパスにディレクトリを作成できること");

        remove_legacy_cache_in(&base);

        assert!(
            path.is_dir(),
            "旧キャッシュパスのディレクトリは削除しないこと"
        );

        cleanup(&base);
    }

    #[test]
    fn test_remove_legacy_cache_in_noop_when_missing() {
        let base = test_temp_dir("legacy-missing");

        // 旧キャッシュが無くても panic せず、一時ディレクトリを残すこと
        remove_legacy_cache_in(&base);

        assert!(base.is_dir(), "一時ディレクトリは残ること");

        cleanup(&base);
    }

    #[test]
    fn test_remove_legacy_cache_in_does_not_follow_symlink() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let base = test_temp_dir("legacy-symlink");
            let path = legacy_cache_path(&base);
            let target = base.join("target");
            std::fs::write(&target, "do not remove").expect("リンク先ファイルを作成できること");
            symlink(&target, &path).expect("テスト用シンボリックリンクを作成できること");

            remove_legacy_cache_in(&base);

            // シンボリックリンク自体は削除され、リンク先は残ること
            assert!(
                std::fs::symlink_metadata(&path).is_err(),
                "旧キャッシュのシンボリックリンクが削除されること"
            );
            assert_eq!(
                std::fs::read_to_string(&target).expect("リンク先ファイルを読み込めること"),
                "do not remove",
                "シンボリックリンクのリンク先を削除しないこと"
            );

            cleanup(&base);
        }
    }
}
