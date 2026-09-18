# tokio-ngtcp2 サーバーに接続数上限とレート制限を追加する

- Created: 2026-08-13
- Completed: {YYYY-MM-DD}
- Branch: feature/add-server-dos-mitigations
- Polished: {YYYY-MM-DD}

## 目的

アドレス検証前のサーバーのリソース消費を攻撃から守る。Retry によるアドレス検証は crates.io の `shiguredo_ngtcp2_tokio` が提供するようになったが、接続数上限と新規接続のレート制限は無く、DCID を変えながら Initial を送り続けるとサーバーのメモリを枯渇させられる。

## 現状

- `crates/tokio-ngtcp2/src/server.rs` の `Server::run` / `crates/tokio-ngtcp2/src/webtransport.rs` の `ServerWebTransportSession::run` は、接続数上限とレート制限を持たない。`accept` した接続はハンドシェイク完了後にすべて状態として保持される
- Retry とトークンによるアドレス検証は `shiguredo_ngtcp2_tokio::ServerConfig::with_retry` / `with_new_token` / `with_retry_token_timeout` が実装済みだが、`tokio-ngtcp2` の `bind_with_settings` は `ServerConfig` を外から指定できない
- QUIC の `max_idle_timeout` (既定 30 秒) の間は接続状態が保持される

## 設計方針

- Retry によるアドレス検証 (RFC 9000 Section 8.1.2) は `shiguredo_ngtcp2_tokio` の `ServerConfig::with_retry(RetrySecret)` を使う。`tokio-ngtcp2` の `Server::bind_with_settings` / `ServerWebTransportSession::bind` に `shiguredo_ngtcp2_tokio::ServerConfig` を渡せるようにする
- 接続数上限: `Server` / `ServerWebTransportSession` に最大接続数の設定を追加する。上限に達した場合は `accept` した接続を状態に加えずに閉じる (新しい接続を拒否し、既存接続には影響を与えない)
- 新規接続のレート制限: 新規接続の作成レートをトークンバケットなどで制限する。超過分は接続状態を作らずに破棄する。既存接続のパケット処理は制限しない
- 制限超過時もサーバーループは継続させる (サーバーを停止しない)
- クライアント側は変更しない

## 完了条件

- `ServerConfig::with_retry` を指定したサーバーで Retry パケットが送信され、トークン付き Initial でハンドシェイクが継続できる
- 接続数上限を超えた新規接続が拒否され、既存接続は影響を受けない
- レート制限超過時の新規接続が破棄され、サーバーは継続する
- テストが追加される (Retry 経路、接続数上限、レート制限のそれぞれでサーバーが継続すること)
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `crates/tokio-ngtcp2/src/server.rs` (`bind_with_settings` / `run` / `add_connection`)
- `crates/tokio-ngtcp2/src/webtransport.rs` (`bind` / `run` / `recv_once` / `add_connection`)
- `crates/tokio-ngtcp2/Cargo.toml` (`shiguredo_ngtcp2_tokio` の `RetrySecret` / `ServerConfig` の利用)
- 一次資料: `refs/quic/rfc9000.txt` Section 8.1 (Address Validation)、Section 8.1.1 (Token Construction)、Section 8.1.2 (Retry)、Section 5.2.2 (Server Packet Handling)
