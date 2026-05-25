# 0073: PBT ファイルの過大・重複問題

- Priority: Low
- Created: 2026-05-14
- Model: deepseek-v4-pro
- Branch: feature/refactor-split-oversized-pbt

## 目的

`pbt/tests/prop_webtransport.rs` が 1802 行に肥大化しており、AGENTS.md の分割基準を満たしている。`src/webtransport/` はディレクトリモジュールであるため `pbt/tests/prop_webtransport/main.rs` にサブモジュール分割する必要がある（AGENTS.md: ディレクトリモジュールの場合は `pbt/tests/prop_<module>/main.rs` にサブモジュール対応で分割すること）。

また、専用 PBT ファイル (`prop_capsule.rs`, `prop_datagram.rs`) と `prop_webtransport.rs` 間でラウンドトリップテストが重複している。

## 優先度根拠

Low: テストの動作自体に問題はない。ファイル構成の改善であり、保守性向上が目的。

## 現状

1. `pbt/tests/prop_webtransport.rs`: 1802 行（AGENTS.md の分割基準超過）
2. Capsule ラウンドトリップが `prop_capsule.rs` と `prop_webtransport.rs` で重複
3. Datagram ラウンドトリップが `prop_datagram.rs` と `prop_webtransport.rs` で重複

## 設計方針

1. `pbt/tests/prop_webtransport.rs` → `pbt/tests/prop_webtransport/main.rs` に変換
2. サブモジュール分割: `capsule.rs`, `datagram.rs`, `connect.rs`, `session.rs`, `stream.rs`, `settings.rs`, `error.rs` 等
3. `prop_capsule.rs` / `prop_datagram.rs` と重複するテストを `prop_webtransport/` 側から削除し、専用ファイルに一本化

## 完了条件

- `prop_webtransport/main.rs` + サブモジュールへの分割が完了していること
- 重複テストが削除されていること
- `cargo test -p pbt` が全て pass すること

## 影響範囲

- `pbt/tests/prop_webtransport.rs` → `pbt/tests/prop_webtransport/main.rs` + サブモジュール
- `pbt/tests/prop_capsule.rs`: 一本化先（変更なし）
- `pbt/tests/prop_datagram.rs`: 一本化先（変更なし）

## CHANGES.md エントリ案

```
### misc

- [UPDATE] prop_webtransport.rs をディレクトリモジュールに分割し PBT 間の重複テストを削除する
  - @担当者
```
