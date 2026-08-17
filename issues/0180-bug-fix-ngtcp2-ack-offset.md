# nghttp3 の add_ack_offset が呼ばれず ACK 済みデータの資源解放が行われない

- Created: 2026-08-16
- Completed: {YYYY-MM-DD}
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

### 関連ファイル

- `crates/ngtcp2-rs/src/conn.rs` (`create_client_callbacks` / `create_server_callbacks` への `acked_stream_data_offset` コールバック登録)
- `crates/ngtcp2-rs/src/h3.rs` (`Http3Connection::add_ack_offset` をコールバックから呼び出す経路)
