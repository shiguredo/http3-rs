# 依存ライブラリのバージョン指定をマイナーまで揃え base64 を base64ct に置換する

- Priority: High
- Created: 2026-06-15
- Model: Opus 4.7
- Branch: feature/fix-cargo-dependency-version-and-base64
- Polished: 2026-07-21

## 目的

`examples/wt_server/Cargo.toml` および `fuzz/Cargo.toml` の依存ライブラリの一部がメジャーバージョンのみ (`"1"`) で指定されており AGENTS.md「バージョン番号はマイナーバージョンまで指定すること」に違反している。さらに `examples/wt_server/Cargo.toml:13` の `base64 = "0.22"` は AGENTS.md「base64 は base64ct を使うこと」に違反している。これらを規約に揃える。

## 優先度根拠

High。AGENTS.md の明示的な規約違反 (依存ライブラリ選定 / バージョン指定方針)。例示クレートとはいえ規約は適用されるため、放置すると規約自体が形骸化する。修正コストは軽微。

## 現状

`examples/wt_server/Cargo.toml` の base64 → base64ct 置換とマイナーバージョン指定は完了済み。残りは `fuzz/Cargo.toml:12` のみ:

```toml
arbitrary = { version = "1", features = ["derive"] }
```

AGENTS.md:

> バージョン番号はマイナーバージョンまで指定すること
> - 例: `spam = "0.3.10"` ではなく `spam = "0.3"` とする
> - 例: `egg = "1.0.1"` ではなく `egg = "1.0"` とする

## 設計方針

- `fuzz/Cargo.toml` の `arbitrary = { version = "1" }` を最新マイナーまで指定 (`Cargo.lock` の現在値を参照)
- 全 workspace member の Cargo.toml を確認し、メジャーのみ指定が残っていれば修正する

## 完了条件

- 全 `Cargo.toml` の依存指定がマイナーバージョンまで揃う
- `cargo build --workspace` および `cargo test --workspace --exclude interop_h3 --exclude interop_wt` が成功する
- `make fmt && make clippy && make check` が全て通る

## 解決方法

`fuzz/Cargo.toml`:

```toml
# fuzz 入力生成
arbitrary = { version = "1.4", features = ["derive"] }
```

### 関連ファイル

- 修正対象: `fuzz/Cargo.toml`
- 確認対象: 全 workspace member の `Cargo.toml`
- 規約: `AGENTS.md` (ルート)
