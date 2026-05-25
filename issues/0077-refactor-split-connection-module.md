# 0077: src/connection/mod.rs が過大 — モジュール分割が必要

- Priority: Low
- Created: 2026-05-14
- Model: deepseek-v4-pro
- Branch: feature/refactor-split-connection-module

## 目的

`src/connection/mod.rs` が 5737 行に肥大化しており、HTTP/3 接続管理、QPACK ストリーム処理、WebTransport セッション管理、Capsule Protocol 処理が単一ファイルに混在している。AGENTS.md: 「テストが長くなるのはモジュール自体が大きすぎるサインなので src/<module>.rs 側の分割を検討すること」。

## 優先度根拠

Low: 機能的な問題はないが保守性が低下している。private 関数が 40 個超、変更の影響範囲把握が困難。ただし分割は大規模リファクタリングであり慎重に進める必要がある。

## 現状

- `src/connection/mod.rs`: 5737 行
- WebTransport 関連コード（セッション管理、ストリーム dispatch、データグラム処理）: 約 1500 行
- QPACK エンコーダー/デコーダーストリーム処理: 約 200 行
- GOAWAY 処理: 約 100 行
- 残り: HTTP/3 接続ステートマシン本体

## 設計方針

WebTransport 関連ロジックを以下のサブモジュールに分割する:

1. `src/connection/wt_session.rs` — WebTransport セッションのライフサイクル管理
2. `src/connection/wt_receive.rs` — WebTransport 受信パス（ストリーム/データグラム）
3. `src/connection/wt_dispatch.rs` — WebTransport ストリーム dispatch と配送

`mod.rs` から `pub(super)` メソッドとして切り出し、`Connection` への `&mut self` 参照を維持する。

## 完了条件

- `src/connection/mod.rs` が 4000 行以下になっていること
- WebTransport ロジックが適切なサブモジュールに分離されていること
- `cargo test` が全て pass すること
- 相互運用テストが pass すること

## 影響範囲

- `src/connection/mod.rs`: 大幅縮小
- 新規: `src/connection/wt_session.rs`, `src/connection/wt_receive.rs`, `src/connection/wt_dispatch.rs`

## CHANGES.md エントリ案

```
### misc

- [UPDATE] connection/mod.rs から WebTransport ロジックをサブモジュールに分割する
  - @担当者
```
