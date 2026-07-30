# 楽観的カプセル送信のサーバー側バッファリングを実装する

- Created: 2026-07-31
- Completed: {YYYY-MM-DD}
- Branch: feature/add-optimistic-capsule-buffering
- Polished: {YYYY-MM-DD}

## 目的

draft-ietf-webtrans-http3-16 Section 3.2 の楽観的カプセル送信要件に対応する。0135 から分割。

## 現状

draft-16 追加要件:

> To reduce latency at the start of a WebTransport session, a client MAY optimistically send capsules on the CONNECT stream before receiving the server's response. A server MUST NOT process these bytes as capsules until it sends a 2xx response accepting the session. Bytes received before the server sends the response are processed once the session is accepted or discarded if the session is rejected.

現在、`src/connection/wt_capsule.rs` の `handle_wt_data_frame` は `WtSessionState::Pending` 時に draft-07/14/15 で `H3_MESSAGE_ERROR` を返している。

## 設計方針

- **サーバー側のみ** `handle_wt_data_frame` の Pending 分岐でカプセルデータをバッファリングし、`src/connection/wt_session.rs` の `establish_wt_session_server` (2xx 送信時) でバッファを処理する
- セッションが拒否された場合はバッファを破棄する
- クライアント側は現行の `H3_MESSAGE_ERROR` を維持する (楽観的送信は client → server 方向のみ)
- バッファは `WtSession.capsule_buf` を再利用する。DoS 対策としてバッファ上限を設け、超過時は `H3_MESSAGE_ERROR` でストリームをリセットする

## 完了条件

- サーバーが 2xx 前に受信したカプセルデータをバッファリングし、2xx 送信後に処理する
- セッション拒否時にバッファが破棄される
- クライアント側の挙動が変更されない
- テストが追加される
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/connection/wt_capsule.rs` (`Connection::handle_wt_data_frame`)
- `src/connection/wt_session.rs` (`establish_wt_session_server`)
- 一次資料: `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 3.2
