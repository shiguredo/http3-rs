# 0073: PBT ファイルの過大・重複問題

Created: 2026-05-14
Model: deepseek-v4-pro

## 概要

### 問題 1: prop_webtransport.rs が過大 (1439 行)

`pbt/tests/prop_webtransport.rs` が 1795 行あり、CLAUDE.md L86-88 の分割基準を満たしている。
`src/webtransport/` はディレクトリモジュールのため、
`pbt/tests/prop_webtransport/main.rs` にサブモジュール分割する必要がある。

### 問題 2: PBT 間の Capsule/Datagram テスト重複

- `prop_capsule.rs:78-248` と `prop_webtransport.rs:60-243` で Capsule ラウンドトリップが重複
- `prop_datagram.rs:36-65` と `prop_webtransport.rs:982-1041` で Datagram ラウンドトリップが重複

専用の PBT ファイルがあるにもかかわらず、`prop_webtransport.rs` で再実装されている。

### 問題 3: error.rs に PBT 不在

`ErrorCode::from_code` と `ErrorCode::code` はラウンドトリップ可能なペアだが、
`pbt/tests/prop_error.rs` が存在しない (CLAUDE.md L92, L95)。

## 修正方針

1. `prop_webtransport.rs` → `prop_webtransport/main.rs` に変換しサブモジュール化
2. 重複テストを削除し、各モジュール対応 PBT に一本化
3. `pbt/tests/prop_error.rs` を追加

## 影響範囲

- `pbt/tests/prop_webtransport.rs`
- `pbt/tests/prop_capsule.rs`
- `pbt/tests/prop_datagram.rs`
- 新規: `pbt/tests/prop_error.rs`
- 新規: `pbt/tests/prop_webtransport/main.rs` (+ サブモジュール)
