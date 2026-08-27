# `PendingWtBidiPreNegotiation` に `Default` derive を追加する

- Created: 2026-08-28
- Completed: {YYYY-MM-DD}
- Branch: feature/refactor-pending-wt-bidi-pre-negotiation-default

## 目的

`PendingWtBidiPreNegotiation` 構造体に `#[derive(Default)]` を追加し、`or_insert_with(|| PendingWtBidiPreNegotiation { data: Vec::new(), fin: false })` を `.or_default()` に置換して、将来のフィールド追加時に初期化式の追従漏れを防ぐ。

## 現状

- `src/connection/mod.rs` の `PendingWtBidiPreNegotiation` は `#[derive(Debug)]` のみ
- `buffer_pre_negotiation_wt_bidi` 内で `entry(stream_id).or_insert_with(|| PendingWtBidiPreNegotiation { data: Vec::new(), fin: false })` と手書きで初期化している
- フィールド (`data: Vec<u8>`, `fin: bool`) はいずれも `Default` を実装しているため、`#[derive(Default)]` を付けても意味論は変わらない

## 設計方針

- `PendingWtBidiPreNegotiation` の derive を `#[derive(Debug, Default)]` に変更する
- `or_insert_with(...)` を `.or_default()` に置換する
- 将来フィールドを追加した際、`Default` derive で自動的に初期化される (追従漏れが起きない)

## 完了条件

- `PendingWtBidiPreNegotiation` に `Default` derive が追加される
- 呼び出し側が `.or_default()` に置換される
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/connection/mod.rs` (`PendingWtBidiPreNegotiation` の定義と呼び出し側)

### 関連 issue

- 0178 (本 issue の起源。フィールド追加時の追従漏れ防止)
