# 0065: interop/h3 と interop/wt が workspace 継承を使用していない

- Priority: Low
- Created: 2026-05-14
- Model: deepseek-v4-pro
- Branch: feature/fix-interop-workspace-inheritance

## 目的

CHANGES.md に「edition と rust-version を `[workspace.package]` で共通化し、workspace member は `.workspace = true` で継承するようにする」と記載されており、ルートの `Cargo.toml` にも `[workspace.package]` が定義されている。しかし `interop/h3` と `interop/wt` は `edition = "2024"` / `rust-version = "1.88"` と直書きされたままであり、workspace 継承に移行し忘れている。

## 優先度根拠

Low: 機能に影響しない。workspace の edition/rust-version と同じ値が直書きされているだけなので現状で動作上の問題はない。ただし将来 edition や rust-version を更新する際に interop クレートだけ取り残されるリスクがある。

## 現状

- `interop/h3/Cargo.toml:4-5`: `edition = "2024"` / `rust-version = "1.88"` と直書き
- `interop/wt/Cargo.toml:4-5`: 同上
- 他の workspace member（`shiguredo_http3`, `pbt`, `examples/wt_server` 等）は既に `edition.workspace = true` / `rust-version.workspace = true` を使用

## 設計方針

```toml
# 修正前
edition = "2024"
rust-version = "1.88"

# 修正後
edition.workspace = true
rust-version.workspace = true
```

`fuzz/Cargo.toml` にも `edition = "2024"` 直書きがあるが、fuzz は `[workspace]` の `exclude` に含まれているため対象外。

## 完了条件

- `interop/h3/Cargo.toml` と `interop/wt/Cargo.toml` が workspace 継承を使用していること
- `cargo check --workspace` が pass すること
- `cargo test --workspace` が pass すること

## 影響範囲

- `interop/h3/Cargo.toml`
- `interop/wt/Cargo.toml`

## CHANGES.md エントリ案

既存エントリ「edition と rust-version を `[workspace.package]` で共通化...」の対象漏れの修正であり、新規エントリは不要（misc セクションに記載する場合のみ以下）:

```
### misc

- [UPDATE] interop/h3 と interop/wt の edition / rust-version を workspace 継承に変更する
  - @担当者
```
