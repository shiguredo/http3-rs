# tokio-s2n-quic の `H3Client::send_request` にも「1 接続 1 リクエスト逐次処理」の doc を追加する

- Created: 2026-08-27
- Completed: {YYYY-MM-DD}
- Branch: feature/refactor-s2n-h3-client-send-request-doc
- Polished: {YYYY-MM-DD}

## 目的

`H3Client::send_request` の doc コメントに、`H3ServerConnection::accept_request` と対称的な「1 接続 1 リクエスト逐次処理」の設計制約を明示する。

## 現状

- `crates/tokio-s2n-quic/src/h3/server.rs` の `H3ServerConnection::accept_request` には「接続共有イベントキューを本メソッドが独占ドレインするため、H3Request を保持したまま次を accept する使い方はしないこと」と「ピア側にも同時に 1 本のリクエストストリームしか開かないことを暗黙に要求する」ことが doc に明示されている
- `crates/tokio-s2n-quic/src/h3/client.rs` の `H3Client::send_request` は同構造 (接続共有キューを排他ドレイン、`match` アームで `stream_id` guard) を持つが、doc コメントが 1 行のみで設計制約が明示されていない
- `&mut self` により構造上 1 本しか送信できないため呼び出し側の誤用リスクは server ほど高くないが、対称性が欠けている

## 設計方針

- `H3Client::send_request` の doc に「1 接続 1 リクエスト逐次処理」の設計制約を追記する (server 側と対称)
- サーバー側と同じ文言・構成にする

## 完了条件

- `H3Client::send_request` の doc に設計制約が明示される
- `cargo test --workspace --tests` / `cargo fmt --all -- --check` / `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `crates/tokio-s2n-quic/src/h3/client.rs` (`H3Client::send_request` の doc)
- 参考: `crates/tokio-s2n-quic/src/h3/server.rs` (`H3ServerConnection::accept_request` の doc)
