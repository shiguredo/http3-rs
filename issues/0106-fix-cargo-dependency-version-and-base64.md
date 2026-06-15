# 依存ライブラリのバージョン指定をマイナーまで揃え base64 を base64ct に置換する

- Priority: High
- Created: 2026-06-15
- Completed:
- Model: Opus 4.7
- Branch: feature/fix-cargo-dependency-version-and-base64
- Polished:

## 目的

`examples/wt_server/Cargo.toml` および `fuzz/Cargo.toml` の依存ライブラリの一部がメジャーバージョンのみ (`"1"`) で指定されており CLAUDE.md「バージョン番号はマイナーバージョンまで指定すること」に違反している。さらに `examples/wt_server/Cargo.toml:13` の `base64 = "0.22"` は CLAUDE.md「base64 は base64ct を使うこと」に違反している。これらを規約に揃える。

## 優先度根拠

High。CLAUDE.md の明示的な規約違反 (依存ライブラリ選定 / バージョン指定方針)。例示クレートとはいえ規約は適用されるため、放置すると規約自体が形骸化する。修正コストは軽微。

## 現状

`examples/wt_server/Cargo.toml`:

```toml
base64 = "0.22"
aws-lc-rs = "1"
tokio = { version = "1", features = [...] }
bytes = "1"
```

`fuzz/Cargo.toml`:

```toml
arbitrary = { version = "1", features = ["derive"] }
```

CLAUDE.md:

> バージョン番号はマイナーバージョンまで指定すること
> - 例: `spam = "0.3.10"` ではなく `spam = "0.3"` とする
> - 例: `egg = "1.0.1"` ではなく `egg = "1.0"` とする

> 依存ライブラリには用途をコメントで明記すること
> base64 は base64ct を使うこと

## 設計方針

- `aws-lc-rs`, `tokio`, `bytes`, `arbitrary` 等のメジャーのみ指定を最新マイナーまで指定 (`Cargo.lock` の現在値を参照)
- `examples/wt_server` の `base64` 依存を `base64ct` に置換し、利用箇所 (Base64 エンコード / デコード) を `base64ct::Base64` 等の API に差し替える
- 全 Cargo.toml の `[dependencies]` 上に用途コメントを 1 行ずつ付ける (本 issue で同時対応するか、別 issue として分離するかは判断。本 issue では併せて対応する)
- workspace member の Cargo.toml すべてを確認 (`crates/*/Cargo.toml`, `interop/*/Cargo.toml`, `pbt/Cargo.toml`)

## 完了条件

- 全 `Cargo.toml` の依存指定がマイナーバージョンまで揃う
- `examples/wt_server` の `base64` が `base64ct` に置換される
- 全 `[dependencies]` の各エントリに用途コメントが付く
- `cargo build --workspace` および `cargo test --workspace --exclude interop_h3 --exclude interop_wt` が成功する
- `examples/wt_server` の動作が変わらないことを確認する (WHIP シグナリング統合テストは `WHIP_ENDPOINT` 必須なのでスキップ可だが、`cargo build -p wt_server` は通すこと)
- `make fmt && make clippy && make check` が全て通る

## 解決方法

`examples/wt_server/Cargo.toml`:

```toml
[dependencies]
# Base64 エンコード / デコード
base64ct = { version = "1.6", features = ["alloc"] }
# TLS 用暗号ライブラリ (workspace 規約)
aws-lc-rs = "1.17"
# 非同期ランタイム
tokio = { version = "1.52", features = [...] }
# バイトバッファ
bytes = "1.11"
```

`fuzz/Cargo.toml`:

```toml
# fuzz 入力生成
arbitrary = { version = "1.4", features = ["derive"] }
```

base64 の利用箇所では、`base64::engine::general_purpose::STANDARD.encode(...)` を `base64ct::Base64::encode_string(...)` 等に書き換える。

### 関連ファイル

- 修正対象:
  - `examples/wt_server/Cargo.toml`
  - `examples/wt_server/src/*.rs` (base64 利用箇所)
  - `fuzz/Cargo.toml`
  - 全 workspace member の `Cargo.toml` (用途コメント追加)
- 規約: `CLAUDE.md` (ルート)
