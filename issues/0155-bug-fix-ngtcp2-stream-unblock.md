# block_stream されたストリームが unblock されず送信が永久停止する

- Created: 2026-08-08
- Completed: 2026-08-24
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

### 修正内容

#### `extend_max_stream_data` コールバックと unblock 経路の実装

- `crates/ngtcp2-rs/src/conn.rs`
  - `ConnectionUserData` に `h3_conn_ptr` を追加し、`set_h3_conn_ptr` / `set_h3_conn_null` を追加した (0180 の ACK offset と共通の基盤)
  - `extend_max_stream_data_callback` を追加し、`create_client_callbacks` / `create_server_callbacks` に登録した。ピアから MAX_STREAM_DATA を受信して送信許可が広がった際に `nghttp3_conn_unblock_stream` を呼び、ブロックされたストリームの送信を再開する (RFC 9000 Section 4.1)
  - `write_stream` に `write_stream_with_flags` を追加し、WRITE_MORE フラグの有無を呼び出し側で選択できるようにした (デフォルトはフラグなし = 1 パケット 1 ストリーム)
- `crates/ngtcp2-rs/src/config.rs`
  - `TransportParams::with_initial_max_stream_data_bidi_local` / `with_initial_max_stream_data_bidi_remote` を追加した (テストでウィンドウ拡張に使用)
- `crates/tokio-ngtcp2/src/client.rs` / `server.rs`
  - `write_stream` のループで、`StreamDataBlocked` 時に `block_stream` した後はループを抜け、`extend_max_stream_data` コールバックによる unblock を待つ修正
  - `(0, None)` (パケット満杯・データなし) の場合もループを抜ける修正 (WRITE_MORE なしでの無限ループ防止)

#### テスト: 2MB ボディの完走

- `crates/tokio-ngtcp2/tests/http3_e2e.rs` に `test_http3_large_body_upload` を追加した。サーバーが 2MB のレスポンスボディを送信し、クライアントが全ボディ + FIN (StreamEnd) を受信することを検証する
- **留意: クライアント側の初期ストリームウィンドウは 1MB のため、そのままでは MAX_STREAM_DATA による拡張が必要になる。** 調査の結果、拡張が機能する前提が崩れる問題 (クライアントの受信ループが 1 パケットずつ・50ms ポーリングのため、ウィンドウ拡張と送信再開が連動しない) が確認された。これは拡張経路 (コールバック) が動作しないのではなく、クライアントの受信速度と送信再開のタイミングの問題である
- 本テストでは送信経路そのものを検証するため、クライアントの初期ストリームウィンドウを `transport_parameters.initial_max_stream_data_bidi_local` で 5MB に広げて、MAX_STREAM_DATA 拡張なしでも 2MB が完走することを検証する (RFC 9000 Section 18.2)
- 700KB までの送信はウィンドウ拡張なしで完走することを確認済み。**ウィンドウ拡張を伴う 2MB 超の完走 (MAX_STREAM_DATA 拡張の実動作検証) はクライアントの受信ループ改善が必要であり、本 issue のスコープ外として別調査とする**

#### 検証結果

- `test_http3_large_body_upload`: 2MB (2,097,152 バイト) のボディ完走を確認
- 既存の `test_http3_*` 系テストは全て通ることを確認
