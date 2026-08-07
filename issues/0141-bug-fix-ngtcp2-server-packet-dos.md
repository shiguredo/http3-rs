# tokio-ngtcp2 サーバーが単一パケットでプロセス全体停止する

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-ngtcp2-server-packet-dos
- Polished: {YYYY-MM-DD}

## 目的

不正パケット 1 個でサーバー全体が終了するリモート DoS 経路を修正する。

## 現状

- `crates/tokio-ngtcp2/src/server.rs` の `Server::run` は `connections: HashMap<SocketAddr, ServerConnection>` をキーにパケットを配送する。同一 `SocketAddr` からの新規 Initial (2 接続目) が既存接続の `read_pkt` に渡され、DCID 不一致でエラーになり `?` で `run()` 全体が Err 終了する
- `read_pkt` の**あらゆる**負リターンがサーバー停止になる。ngtcp2 の API 契約上、`NGTCP2_ERR_RETRY` / `NGTCP2_ERR_DROP_CONN` / `NGTCP2_ERR_DISCARD_PACKET` は非致命的に扱うべき (公式 example は DISCARD_PACKET を無視して継続)
- `crates/tokio-ngtcp2/src/webtransport.rs` の `run` も同構造

## 設計方針

- パケット処理のエラーを接続単位で握りつぶして continue する
- エラー種別を弁別し、`NGTCP2_ERR_RETRY` / `NGTCP2_ERR_DROP_CONN` / `NGTCP2_ERR_DISCARD_PACKET` は非致命的として扱う
- 同一 `SocketAddr` からの新規 Initial は新規接続として処理する

## 完了条件

- 同一アドレスから 2 接続目を張ってもサーバーが継続する
- 不正パケット (DCID 不一致・破損) を送ってもサーバーが継続する
- テストが追加される
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `crates/tokio-ngtcp2/src/server.rs` (`Server::run` のパケット配送経路)
- `crates/tokio-ngtcp2/src/webtransport.rs` (`run` のパケット配送経路)
