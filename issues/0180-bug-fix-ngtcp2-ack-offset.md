# nghttp3 の add_ack_offset が呼ばれず ACK 済みデータの資源解放が行われない

- Created: 2026-08-16
- Completed: 2026-08-23
- Branch: feature/fix-ngtcp2-ack-offset
- Polished: {YYYY-MM-DD}

## 目的

`nghttp3_conn_add_ack_offset` を呼び出して、ピアに ACK されたストリームデータの資源を nghttp3 が解放できるようにする。

## 現状

- `crates/ngtcp2-rs/src/h3.rs` の `Http3Connection::add_ack_offset` は定義されているが、ワークスペース内に呼び出し元が存在しない
- ngtcp2 の `acked_stream_data_offset` コールバック (ピアに ACK されたストリームデータの範囲を通知する。`crates/ngtcp2-sys/src/bindings.rs` の `ngtcp2_callbacks::acked_stream_data_offset`) も `crates/ngtcp2-rs/src/conn.rs` の `create_client_callbacks` / `create_server_callbacks` に登録されていない
- 結果として ACK 済みデータの解放が nghttp3 に通知されず、大容量・長時間の送信で nghttp3 内部の資源が解放されない

## 設計方針

- ngtcp2 の `acked_stream_data_offset` コールバックを登録し、`Http3Connection::add_ack_offset` を呼ぶ (ngtcp2 公式 example と同じ方式)
- ngtcp2 コールバックの user_data は `ConnectionUserData` のため、コールバック内から nghttp3 conn (tokio-ngtcp2 側が所有) への到達手段を検討する (例: `Http3Connection` への参照を渡す手段の追加)

## 完了条件

- `acked_stream_data_offset` コールバックが登録され、ACK されたデータ量が nghttp3 に通知される
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 修正内容

- `crates/ngtcp2-rs/src/conn.rs`
  - `ConnectionUserData` に `h3_conn_ptr: *mut c_void` フィールドを追加
  - `Connection::set_h3_conn_ptr` / `set_h3_conn_null` を追加 (nghttp3_conn へのポインタを設定・クリア。`Connection` と `Http3Connection` が別オブジェクトのため)
  - `acked_stream_data_offset` コールバックを実装し、`create_client_callbacks` / `create_server_callbacks` に登録。コールバック内で `nghttp3_conn_add_ack_offset(conn, stream_id, offset + datalen)` を呼ぶ (ACK 済みデータの資源解放は nghttp3 の責務)
  - `nghttp3_conn_add_ack_offset` は `&mut Http3Connection` 経由ではなく生ポインタから直接呼ぶ (コールバック中は ngtcp2 の元に `&mut self` を渡せないため)
- `crates/ngtcp2-rs/src/h3.rs`
  - `Http3Connection::as_mut_ptr` を追加 (`nghttp3_conn` 生ポインタを返す。`&mut self` ではなく `&self` で受ける)
- `crates/tokio-ngtcp2/src/client.rs` / `server.rs` / `webtransport.rs`
  - `Connection` 生成後に `set_h3_conn_ptr` で nghttp3_conn へのポインタを設定 (4 箇所)
  - SAFETY: `Connection` と `Http3Connection` は同一構造体で保持され、フィールド宣言順 (conn が先) でドロップされるため、コールバック実行中にポインタが無効になることはない
- 動作検証: 既存の E2E テスト (http3_e2e / webtransport) が全てパスすることを確認 (ACK 通知は ngtcp2 / nghttp3 内部の資源解放であり、外部動作の変化はレスポンス成功・ボディ送信の完走として観察される)

### 関連ファイル

- `crates/ngtcp2-rs/src/conn.rs` (`create_client_callbacks` / `create_server_callbacks` への `acked_stream_data_offset` コールバック登録)
- `crates/ngtcp2-rs/src/h3.rs` (`Http3Connection::add_ack_offset` をコールバックから呼び出す経路)
