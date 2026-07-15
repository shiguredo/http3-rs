use std::path::PathBuf;
use std::process::Command;

/// Cargo.toml から外部依存関係のメタデータを読み取る
fn read_external_dependency(name: &str) -> shiguredo_toml::Table {
    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("build script must succeed"));
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
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("build script must succeed"));

    // Cargo.toml からメタデータを読み取る
    let dep = read_external_dependency("nghttp3");
    let git_url = dep
        .get("git")
        .and_then(|v| v.as_str())
        .expect("Missing 'git' field in nghttp3 dependency");
    // nghttp3 は webtransport ブランチがリリースされたら version (タグ) に切り替える
    let branch = dep.get("branch").and_then(|v| v.as_str());
    let version = dep.get("version").and_then(|v| v.as_str());

    // nghttp3 をクローンする (クローン済みの場合は fetch して最新化する)
    let nghttp3_dir = out_dir.join("nghttp3");
    let fetched = if nghttp3_dir.exists() {
        // 既存クローンが stale なまま使われると bindings やビルドが上流と乖離するため、
        // fetch で最新に追従する (失敗時は既存のチェックアウトで続行する)
        let status = Command::new("git")
            .current_dir(&nghttp3_dir)
            .args(["fetch", "origin"])
            .status();
        let fetched = matches!(status, Ok(s) if s.success());
        if !fetched {
            println!("cargo:warning=Failed to fetch nghttp3; using existing checkout");
        }
        fetched
    } else {
        // git clone
        let status = Command::new("git")
            .args([
                "clone",
                git_url,
                nghttp3_dir.to_str().expect("build script must succeed"),
            ])
            .status()
            .expect("Failed to execute git clone");
        if !status.success() {
            panic!("Failed to clone nghttp3");
        }
        true
    };

    // branch または version (タグ) でチェックアウト
    if let Some(branch_name) = branch {
        let status = Command::new("git")
            .current_dir(&nghttp3_dir)
            .args(["checkout", branch_name])
            .status()
            .expect("Failed to execute git checkout");
        if !status.success() {
            panic!("Failed to checkout branch {branch_name}");
        }
        // fetch 済みの場合はリモートブランチの最新コミットにリセットする
        if fetched {
            let status = Command::new("git")
                .current_dir(&nghttp3_dir)
                .args(["reset", "--hard", &format!("origin/{branch_name}")])
                .status()
                .expect("Failed to execute git reset");
            if !status.success() {
                panic!("Failed to reset to origin/{branch_name}");
            }
        }
    } else if let Some(ver) = version {
        let tag = format!("v{ver}");
        let status = Command::new("git")
            .current_dir(&nghttp3_dir)
            .args(["checkout", &tag])
            .status()
            .expect("Failed to execute git checkout");
        if !status.success() {
            panic!("Failed to checkout tag {tag}");
        }
    }

    // submodule を初期化・更新する (reset でコミットが変わる場合があるため毎回実行する)
    let status = Command::new("git")
        .current_dir(&nghttp3_dir)
        .args(["submodule", "update", "--init", "--recursive"])
        .status()
        .expect("Failed to execute git submodule update");
    if !status.success() {
        panic!("Failed to update submodules");
    }

    // nghttp3 ビルド
    let nghttp3_dst = shiguredo_cmake::Config::new(&nghttp3_dir)
        .define("ENABLE_STATIC_LIB", "ON")
        .define("ENABLE_SHARED_LIB", "OFF")
        .define("ENABLE_LIB_ONLY", "ON")
        .define("BUILD_TESTING", "OFF")
        .build();

    // ライブラリパス
    let lib_dir = if nghttp3_dst.join("lib64").exists() {
        nghttp3_dst.join("lib64")
    } else {
        nghttp3_dst.join("lib")
    };

    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=static=nghttp3");

    // 依存クレートに情報を渡す
    println!("cargo:include={}/include", nghttp3_dst.display());

    #[cfg(feature = "overwrite")]
    overwrite_bindgen(&out_dir);
}

#[cfg(feature = "overwrite")]
fn overwrite_bindgen(out_dir: &PathBuf) {
    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("build script must succeed"));
    // ビルド後の include ディレクトリ (version.h が生成される場所)
    let nghttp3_installed_include = out_dir.join("include");
    // ソースの include ディレクトリ (nghttp3.h がある場所)
    let nghttp3_source_include = out_dir.join("nghttp3/lib/includes");

    bindgen::Builder::default()
        .header(
            manifest_dir
                .join("src/wrapper.h")
                .to_str()
                .expect("build script must succeed"),
        )
        .clang_arg(format!("-I{}", nghttp3_installed_include.display()))
        .clang_arg(format!("-I{}", nghttp3_source_include.display()))
        .allowlist_function("nghttp3_.*")
        .allowlist_type("nghttp3_.*")
        .allowlist_var("NGHTTP3_.*")
        .generate()
        .expect("Failed to generate nghttp3 bindings")
        .write_to_file(manifest_dir.join("src/bindings.rs"))
        .expect("Failed to write nghttp3 bindings");
}
