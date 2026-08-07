# block_stream されたストリームが unblock されず送信が永久停止する

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-ngtcp2-stream-unblock
- Polished: {YYYY-MM-DD}

## 目的

フロー制御で `block_stream` されたストリームが、QUIC のフロー制御ウィンドウ解放後も unblock されず永久に送信停止する問題を修正する。

## 現状

- `crates/ngtcp2-rs/src/h3.rs` の `Http3Connection::block_stream` は `StreamDataBlocked` エラー時に呼ばれるが、`unblock_stream` の呼び出しは `crates/tokio-ngtcp2/src/webtransport.rs` の 1 箇所のみ (WT クライアントの再試行ループ)
- `on_acked_stream_data` コールバック (`ngtcp2-rs/src/h3.rs`) は空実装で、ngtcp2 公式 example が行う「ACK 後の unblock」処理がない
- `ngtcp2-rs/src/conn.rs` の `recv_max_data` / `recv_max_stream_data` コールバックも未登録で、フロー制御ウィンドウ増加の通知経路がない
- `TransportParams` の `initial_max_stream_data_bidi_remote` は 1MB (config.rs) のため、1MB を超えるボディの送信で必ず発動し永久停止する
- `add_ack_offset` (h3.rs) もどこからも呼ばれず、ACK 系の資源解放経路が死んでいる

## 設計方針

- `on_acked_stream_data` コールバックで `nghttp3_conn_unblock_stream` を呼ぶ (ngtcp2 公式 example と同じ方式)
- または `recv_max_data` / `recv_max_stream_data` コールバックを登録してウィンドウ増加時に unblock する

## 完了条件

- 1MB 超のボディ送信が詰まらず完走する
- テストが追加される (1MB 超のアップロード / レスポンス)
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `crates/ngtcp2-rs/src/h3.rs` (`Http3Connection::block_stream` / `unblock_stream` / `on_acked_stream_data`)
- `crates/ngtcp2-rs/src/conn.rs` (フロー制御コールバック登録)
- `crates/tokio-ngtcp2/src/client.rs` / `server.rs` / `webtransport.rs` (`write_h3_streams` 系の `StreamDataBlocked` 処理)
