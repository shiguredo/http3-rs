use std::path::PathBuf;
use std::process::Command;

/// aws-lc-sys の links 名を環境変数から自動検出する
/// Cargo は依存クレートの links 属性に基づいて DEP_{LINKS}_INCLUDE を設定するため、
/// バージョンが変わっても自動で追従できる
fn detect_aws_lc_links_name() -> String {
    for (key, _) in std::env::vars() {
        if key.starts_with("DEP_AWS_LC_") && key.ends_with("_INCLUDE") {
            // "DEP_AWS_LC_0_38_0_INCLUDE" → "aws_lc_0_38_0"
            let middle = key
                .strip_prefix("DEP_")
                .unwrap()
                .strip_suffix("_INCLUDE")
                .unwrap();
            return middle.to_lowercase();
        }
    }
    panic!("DEP_AWS_LC_*_INCLUDE not found - aws-lc-sys dependency required");
}

/// Cargo.toml から外部依存関係のメタデータを読み取る
fn read_external_dependency(name: &str) -> shiguredo_toml::Table {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let cargo_toml = std::fs::read_to_string(manifest_dir.join("Cargo.toml"))
        .expect("Failed to read Cargo.toml");
    let parsed = shiguredo_toml::from_str(&cargo_toml).expect("Failed to parse Cargo.toml");

    parsed
        .get("package")
        .and_then(|p| p.get("metadata"))
        .and_then(|m| m.get("external-dependencies"))
        .and_then(|e| e.get(name))
        .and_then(|d| d.as_table())
        .cloned()
        .unwrap_or_else(|| panic!("Missing [package.metadata.external-dependencies.{name}]"))
}

fn main() {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());

    // Cargo.toml からメタデータを読み取る
    let dep = read_external_dependency("ngtcp2");
    let git_url = dep
        .get("git")
        .and_then(|v| v.as_str())
        .expect("Missing 'git' field in ngtcp2 dependency");
    let branch = dep.get("branch").and_then(|v| v.as_str());
    let version = dep.get("version").and_then(|v| v.as_str());

    // ngtcp2 をクローン
    let ngtcp2_dir = out_dir.join("ngtcp2");
    if !ngtcp2_dir.exists() {
        // git clone
        let status = Command::new("git")
            .args(["clone", git_url, ngtcp2_dir.to_str().unwrap()])
            .status()
            .expect("Failed to execute git clone");
        if !status.success() {
            panic!("Failed to clone ngtcp2");
        }

        // branch または version (タグ) でチェックアウト
        if let Some(branch_name) = branch {
            let status = Command::new("git")
                .current_dir(&ngtcp2_dir)
                .args(["checkout", branch_name])
                .status()
                .expect("Failed to execute git checkout");
            if !status.success() {
                panic!("Failed to checkout branch {branch_name}");
            }
        } else if let Some(ver) = version {
            let tag = format!("v{ver}");
            let status = Command::new("git")
                .current_dir(&ngtcp2_dir)
                .args(["checkout", &tag])
                .status()
                .expect("Failed to execute git checkout");
            if !status.success() {
                panic!("Failed to checkout tag {tag}");
            }
        }
    }

    // aws-lc-sys の links 名を自動検出
    let aws_lc_links = detect_aws_lc_links_name();
    let include_env = format!("DEP_{}_INCLUDE", aws_lc_links.to_uppercase());
    let aws_lc_include = std::env::var(&include_env)
        .unwrap_or_else(|_| panic!("{include_env} not set - aws-lc-sys dependency required"));

    // include パスの親ディレクトリが OUT_DIR
    // ライブラリは {OUT_DIR}/build/artifacts/ にある
    let aws_lc_out_dir = PathBuf::from(&aws_lc_include)
        .parent()
        .expect("Failed to get parent directory of include path")
        .to_path_buf();
    let aws_lc_lib_dir = aws_lc_out_dir.join("build").join("artifacts");

    // ngtcp2 ビルド (aws-lc を使用)
    // Windows (MSVC) では .lib、それ以外では lib*.a
    let (ssl_lib, crypto_lib) = if cfg!(target_env = "msvc") {
        (
            aws_lc_lib_dir.join(format!("{}_ssl.lib", aws_lc_links)),
            aws_lc_lib_dir.join(format!("{}_crypto.lib", aws_lc_links)),
        )
    } else {
        (
            aws_lc_lib_dir.join(format!("lib{}_ssl.a", aws_lc_links)),
            aws_lc_lib_dir.join(format!("lib{}_crypto.a", aws_lc_links)),
        )
    };
    // Windows のバックスラッシュを CMake が無効なエスケープとして解釈するため、
    // スラッシュに変換する
    let ssl_lib_str = ssl_lib.to_str().unwrap().replace('\\', "/");
    let crypto_lib_str = crypto_lib.to_str().unwrap().replace('\\', "/");
    let boringssl_libraries = format!("{ssl_lib_str};{crypto_lib_str}");

    let mut ngtcp2_config = shiguredo_cmake::Config::new(&ngtcp2_dir);
    ngtcp2_config
        .define("ENABLE_STATIC_LIB", "ON")
        .define("ENABLE_SHARED_LIB", "OFF")
        .define("ENABLE_LIB_ONLY", "ON")
        .define("BUILD_TESTING", "OFF")
        .define("ENABLE_OPENSSL", "OFF")
        .define("ENABLE_BORINGSSL", "ON")
        .define("BORINGSSL_INCLUDE_DIR", &aws_lc_include)
        .define("BORINGSSL_LIBRARIES", &boringssl_libraries);

    let ngtcp2_dst = ngtcp2_config.build();

    // ライブラリパス
    let lib_dir = if ngtcp2_dst.join("lib64").exists() {
        ngtcp2_dst.join("lib64")
    } else {
        ngtcp2_dst.join("lib")
    };

    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=static=ngtcp2");
    println!("cargo:rustc-link-lib=static=ngtcp2_crypto_boringssl");

    // 依存クレートに情報を渡す
    println!("cargo:include={}/include", ngtcp2_dst.display());

    #[cfg(feature = "overwrite")]
    overwrite_bindgen(&out_dir);
}

#[cfg(feature = "overwrite")]
fn overwrite_bindgen(out_dir: &PathBuf) {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    // ビルド後の include ディレクトリ (version.h が生成される場所)
    let ngtcp2_installed_include = out_dir.join("include");
    // ソースの include ディレクトリ (ngtcp2.h がある場所)
    let ngtcp2_source_include = out_dir.join("ngtcp2/lib/includes");
    // aws-lc のインクルードディレクトリ (openssl/ssl.h がある場所)
    let aws_lc_links = detect_aws_lc_links_name();
    let include_env = format!("DEP_{}_INCLUDE", aws_lc_links.to_uppercase());
    let aws_lc_include =
        std::env::var(&include_env).unwrap_or_else(|_| panic!("{include_env} not set"));

    bindgen::Builder::default()
        .header(manifest_dir.join("src/wrapper.h").to_str().unwrap())
        .clang_arg(format!("-I{}", ngtcp2_installed_include.display()))
        .clang_arg(format!("-I{}", ngtcp2_source_include.display()))
        .clang_arg(format!("-I{}", aws_lc_include))
        .allowlist_function("ngtcp2_.*")
        .allowlist_type("ngtcp2_.*")
        .allowlist_var("NGTCP2_.*")
        .generate()
        .expect("Failed to generate ngtcp2 bindings")
        .write_to_file(manifest_dir.join("src/bindings.rs"))
        .expect("Failed to write ngtcp2 bindings");
}
