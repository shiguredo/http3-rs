# 0081: Connection の send_request / send_response を pub(crate) 化し ClientConnection / ServerConnection 経由のアクセスを強制する

- Priority: Low
- Created: 2026-05-14
- Completed: 2026-05-26
- Polished: 2026-05-26
- Model: deepseek-v4-pro
- Branch: feature/refactor-client-server-wrapper

## 目的

`Connection` が `send_request` / `send_response` を `pub` で露出しており、クライアントが `send_response` を、サーバーが `send_request` を呼べてしまう。この 2 メソッドを `pub(crate)` に落とし、`ClientConnection` / `ServerConnection` 経由でのみ呼べるようにすることで、ロール違反を型レベルで防ぐ。

## 優先度根拠

Low: 型安全性の改善であり、現状で誤使用はコンパイル時に検出されないだけで動作上の問題は発生しない。

## 制約と前提

- `Connection` 自体が `lib.rs:59` で `pub use connection::Connection` として re-export されている。本 issue では `Connection` の re-export は維持する (削除は issue 0083 のスコープ)
- `Connection::client()` / `Connection::server()` も `pub` のままであるため、外部から `Connection` を直接構築してカプセル化を迂回することは理論上可能。この問題は `Connection` の re-export 自体を制限する issue 0083 で対処する
- 本 issue の狙いは「`send_request` / `send_response` のロール制約を型レベルで表現する」最小限の改善であり、`Connection` 全体のアクセス制御見直しは行わない

## 依存関係

- issue 0083 (lib.rs の pub use 整理) で `Connection` の re-export 削除が検討される。 0083 が先に実施された場合、本 issue は 0083 に吸収される可能性がある。 0081 を先に実施する場合は `Connection` の re-export を維持したまま `send_request` / `send_response` のみ `pub(crate)` 化する

## 現状

- `Connection::send_request` (`src/connection/mod.rs:3353`): `pub fn`
- `Connection::send_response` (`src/connection/mod.rs:3506`): `pub fn`
- `ClientConnection`: `Connection::send_request` に委譲
- `ServerConnection`: `Connection::send_response` に委譲
- `examples/wt_server/src/webtransport.rs:106`: `Connection::server(settings)` で `Connection` を直接構築し、`send_response` を直接呼び出している (L579, L618)

## 設計方針

1. `Connection::send_request` と `Connection::send_response` を `pub(crate)` に変更
2. `examples/wt_server` を `ServerConnection` 経由に移行する
   - `Connection::server(settings)` → `ServerConnection::new(settings)`
   - `h3_conn.send_response(...)` は `ServerConnection` が同名メソッドを提供しているため呼び出し側の変更は最小限
   - `wt_server` が使用している `Connection` メソッド (`feed_stream`, `drain_events`, `init_h3_streams`, `take_stream_data`, `send_response`, `set_webtransport_transport_verified`) は全て `ServerConnection` に委譲済みであるため、追加の委譲メソッドは不要

## テスト戦略

- `cargo test --workspace` で全テスト pass (テストは既に `ClientConnection` / `ServerConnection` 経由で使用しているため影響なし)
- `examples/wt_server` が `cargo build` で正常にコンパイルできること
- `Connection::send_request` / `Connection::send_response` が外部クレートから直接呼べないことの確認 (コンパイルエラーになること)

## 完了条件

- `Connection::send_request` と `Connection::send_response` が `pub(crate)` になっていること
- `ClientConnection` / `ServerConnection` 経由でのみアクセス可能であること
- `examples/wt_server` が `ServerConnection` を使用するように移行されていること
- `cargo test --workspace` が全て pass すること
- `examples/wt_server` が正常にコンパイルできること

## 後方互換性

`Connection` の `send_request` / `send_response` を直接呼んでいる外部コードは影響を受ける。 `ClientConnection` / `ServerConnection` を使うのが正しい使い方であり、`[CHANGE]` として記録する。

## 影響範囲

- `src/connection/mod.rs`: `send_request`, `send_response` のアクセス修飾子変更
- `src/connection/client.rs`: 変更なし (既に委譲している)
- `src/connection/server.rs`: `ServerConnection` に未委譲メソッドの追加が必要な場合あり
- `examples/wt_server/src/webtransport.rs`: `Connection` → `ServerConnection` に移行

## 解決方法

1. `src/connection/mod.rs` の `Connection::send_request` / `Connection::send_response` を `pub` → `pub(crate)` に変更
2. `examples/wt_server/src/webtransport.rs` を `Connection::server()` → `ServerConnection::new()` に移行
3. `crates/tokio-s2n-quic/src/internal/connection_state.rs` を `Connection::server()` / `Connection::client()` → `ServerConnection::new()` / `ClientConnection::new()` に移行

issue の影響範囲には `tokio-s2n-quic` が未記載だったが、`pub(crate)` 化の必然的帰結として移行が必要だったため対応した。

## CHANGES.md エントリ案

```markdown
- [CHANGE] `Connection::send_request` / `Connection::send_response` を `pub(crate)` に変更し `ClientConnection` / `ServerConnection` 経由でのみ呼び出し可能にする
  - @voluntas
```
