# 0065: interop/h3 と interop/wt が workspace 継承を使用していない

Created: 2026-05-14
Model: deepseek-v4-pro

## 概要

CHANGES.md に「edition と rust-version を `[workspace.package]` で共通化し、workspace member は
`.workspace = true` で継承するようにする」と記載されているが、
`interop/h3` と `interop/wt` は `edition = "2024"` / `rust-version = "1.88"` と直書きされている。

## 対象箇所

- `interop/h3/Cargo.toml:4-5`
- `interop/wt/Cargo.toml:4-5`

## 修正方針

```toml
# 修正前
edition = "2024"
rust-version = "1.88"

# 修正後
edition.workspace = true
rust-version.workspace = true
```

## 影響範囲

- `interop/h3/Cargo.toml`
- `interop/wt/Cargo.toml`
