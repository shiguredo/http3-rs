# block_stream されたストリームが unblock されず送信が永久停止する

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-ngtcp2-stream-unblock
- Polished: 2026-08-16

## 目的

フロー制御で `block_stream` されたストリームが、QUIC のフロー制御ウィンドウ解放後も unblock されず永久に送信停止する問題を修正する。

## 現状

- `crates/ngtcp2-rs/src/h3.rs` の `Http3Connection::block_stream` は `StreamDataBlocked` エラー時に呼ばれるが、`unblock_stream` の呼び出しは `crates/tokio-ngtcp2/src/webtransport.rs` の 1 箇所のみ (WT クライアントの再試行ループ。このループはコールバック実装後も維持する)
- `crates/ngtcp2-rs/src/conn.rs` の `create_client_callbacks` / `create_server_callbacks` にフロー制御コールバックの登録がない。ngtcp2 の `extend_max_stream_data` コールバック (MAX_STREAM_DATA 受信やローカルストリームの初期ウィンドウ設定時に呼ばれる。RFC 9000 Section 4.1) を登録して `nghttp3_conn_unblock_stream` を呼ぶ必要がある
- `TransportParams` の `initial_max_stream_data_bidi_remote` / `initial_max_stream_data_bidi_local` / `initial_max_stream_data_uni` はすべて 1MB (config.rs) のため、1MB を超えるボディの送信 (アップロード・レスポンスとも) で必ず発動し永久停止する
- 影響範囲は HTTP/3 クライアント (`client.rs` の `write_h3_streams`)、HTTP/3 サーバー (`server.rs` の `write_and_send_h3_streams`)、WT サーバー (`webtransport.rs` の `write_h3_streams_for_wt_connection`) のすべてで、block 後に unblock 経路がない
- `crates/ngtcp2-rs/src/h3.rs` の `Http3Connection::add_ack_offset` がどこからも呼ばれない問題は 0180 で別途対応する

## 設計方針

- ngtcp2 の `extend_max_stream_data` コールバックを登録し、`nghttp3_conn_unblock_stream` を呼ぶ (ngtcp2 公式 example と同じ方式)
- ngtcp2 コールバックの user_data は `ConnectionUserData` のため、コールバック内から nghttp3 conn (tokio-ngtcp2 側が所有) への到達手段を検討する (例: `Http3Connection` への参照を渡す手段の追加)
- 接続レベル (MAX_DATA) のウィンドウ解放は vendored の ngtcp2 (1.25.90) にコールバックが存在しないため検知できない。本 issue はストリームレベル (MAX_STREAM_DATA) の 1MB 超 10MB 未満のボディを対象とする (RFC 9000 Section 4.1 の 2 レベルのフロー制御)

## 完了条件

- 1MB 超 10MB 未満 (例: 2MB) のボディ送信が詰まらず完走する (クライアントのアップロード / サーバーのレスポンス / WT ストリーム。WT はサーバー経路の `write_h3_streams_for_wt_connection` を対象とする。WT クライアントの再試行ループは修正前でも完走しうるためバグ検証にならない)
- テストが追加される (タイムアウト付きの実 I/O テスト。送信 API 呼び出し後にイベントループを回して完了を待つ構成。`crates/tokio-ngtcp2/tests/` 配下)
- `CHANGES.md` の develop セクションに変更履歴が記載される
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `crates/ngtcp2-rs/src/conn.rs` (`create_client_callbacks` / `create_server_callbacks` への `extend_max_stream_data` コールバック登録)
- `crates/ngtcp2-rs/src/h3.rs` (`Http3Connection::unblock_stream` をコールバックから呼び出す経路)
- `crates/tokio-ngtcp2/src/client.rs` / `server.rs` / `webtransport.rs` (`write_h3_streams` 系の `StreamDataBlocked` 処理)
- テストヘルパー `crates/tokio-ngtcp2/tests/helpers/multi_conn_client.rs` も同じ unblock 欠落を持つが、本 issue のスコープ外とする
- 一次資料: `refs/quic/rfc9000.txt` Section 4.1
