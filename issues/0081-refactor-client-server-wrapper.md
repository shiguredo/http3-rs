# 0081: ClientConnection / ServerConnection の薄いラッパーの責務強化

- Priority: Low
- Created: 2026-05-14
- Model: deepseek-v4-pro
- Branch: feature/refactor-client-server-wrapper

## 目的

`ClientConnection` (`src/connection/client.rs`) と `ServerConnection` (`src/connection/server.rs`) は `Connection` への単なる委譲ラッパーであり、追加ロジックはゼロ。`Connection` が `send_request` / `send_response` の両方を `pub` で露出しており、ラッパーのカプセル化効果が不十分（クライアントが `send_response` を呼べてしまう、等）。

## 優先度根拠

Low: 型安全性の改善であり、現状で誤使用はコンパイル時に検出されないだけで動作上の問題は発生しない。

## 現状

- `Connection::send_request` と `Connection::send_response` が共に `pub` メソッド
- `ClientConnection` はそのまま `Connection::send_request` に委譲
- `ServerConnection` はそのまま `Connection::send_response` に委譲
- ロール違反のメソッド呼び出しが型レベルで防げない

## 設計方針

`Connection` 側の `send_request` / `send_response` を `pub(crate)` に落とし、`ClientConnection` / `ServerConnection` 経由でのみ呼べるようにする。型レベルで役割を区別する意図は妥当なため、ラッパーは残す。

## 完了条件

- `Connection::send_request` と `Connection::send_response` が `pub(crate)` になっていること
- `ClientConnection` / `ServerConnection` 経由のみでアクセス可能であること
- `cargo test` が全て pass すること
- examples が正常にコンパイルできること

## 後方互換性

`Connection` の `send_request` / `send_response` を直接呼んでいる外部コードは影響を受ける。ただし `ClientConnection` / `ServerConnection` を使うのが正しい使い方であり、`[CHANGE]` として記録する。

## 影響範囲

- `src/connection/mod.rs`: `send_request`, `send_response` のアクセス修飾子変更
- `src/connection/client.rs`: 変更なし（既に委譲している）
- `src/connection/server.rs`: 変更なし（既に委譲している）

## CHANGES.md エントリ案

```
- [CHANGE] Connection::send_request / Connection::send_response を pub(crate) に変更し ClientConnection / ServerConnection 経由でのみ呼び出し可能にする
  - @担当者
```
