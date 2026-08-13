# Server の run / recv_once ハンドラにコネクション ID を渡す API を追加する

- Created: 2026-08-13
- Completed: {YYYY-MM-DD}
- Branch: feature/add-server-run-by-conn-id
- Polished: {YYYY-MM-DD}

## 目的

同一 SocketAddr から複数接続を張れるようになったサーバーで、アプリケーションがイベントを接続ごとに区別できるようにする。現状のハンドラはアドレスのみを受け取るため、同一アドレスの複数接続ではイベントがどの接続のものか判別できない。

## 現状

- `Server::run` のハンドラは `FnMut(SocketAddr, Http3Event) -> Option<(Vec<Header>, Vec<u8>)>` (crates/tokio-ngtcp2/src/server.rs の `Server::run`)
- `ServerWebTransportSession::run` / `recv_once` のハンドラは `FnMut(SocketAddr, SessionId, Http3Event) -> bool` (crates/tokio-ngtcp2/src/webtransport.rs)
- DCID ルーティング化により同一アドレスから複数接続を張れるようになったが、ハンドラには接続を特定する情報が渡らない。`send_response_by_conn_id` / `open_bidi_stream_by_conn_id` 等のコネクション ID 指定 API はあるが、ハンドラ内でどのコネクション ID を使えばよいかが分からない

## 設計方針

- 既存の `run` / `recv_once` は変更せず (後方互換維持)、コネクション ID をハンドラに渡す新メソッドを追加する (破壊的変更を避ける方針)
- `Server::run_by_conn_id`: `FnMut(ConnectionId, SocketAddr, Http3Event) -> Option<(Vec<Header>, Vec<u8>)>`
- `ServerWebTransportSession::run_by_conn_id`: `FnMut(ConnectionId, SocketAddr, SessionId, Http3Event) -> bool`
- `ServerWebTransportSession::recv_once_by_conn_id`: `FnMut(ConnectionId, SocketAddr, SessionId, Http3Event) -> bool`
- 実装は既存のループを内部メソッドに共通化し、既存メソッドはコネクション ID を捨てる薄いラッパーにする
- 渡すコネクション ID はサーバーが生成した SCID (接続マップのキー) とする

## 完了条件

- 新メソッド 3 本が追加され、既存の `run` / `recv_once` の挙動が変わらない
- 同一アドレスからの 2 接続で、イベントがコネクション ID で区別できることを検証するテストが追加される
- 既存テストが全て通る (`cargo test --all`)
- `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `crates/tokio-ngtcp2/src/server.rs` (`Server::run` / `run_by_conn_id` / 内部ループの共通化)
- `crates/tokio-ngtcp2/src/webtransport.rs` (`ServerWebTransportSession::run` / `recv_once` と `_by_conn_id` 版)
- `crates/tokio-ngtcp2/tests/server_e2e.rs` (新テスト。`tests/helpers/multi_conn_client.rs` を再利用)
