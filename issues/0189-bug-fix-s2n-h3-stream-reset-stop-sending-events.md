# tokio-s2n-quic の H3 リクエスト受信ループが `Event::StreamReset` / `Event::StopSending` を無視する

- Created: 2026-08-27
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-s2n-h3-stream-reset-stop-sending-events
- Polished: {YYYY-MM-DD}

## 目的

H3 クライアント / サーバーがピアからの RESET_STREAM / STOP_SENDING を受信した際、リクエスト受信ループがこれを検知して早期にエラー終了できるようにする。

## 現状

- `crates/tokio-s2n-quic/src/h3/client.rs` の `H3Client::send_request` の受信ループ (`match event` の `_ => {}` 分岐) と `crates/tokio-s2n-quic/src/h3/server.rs` の `H3ServerConnection::accept_request` は、sans-I/O 層が生成する `Event::StreamReset` / `Event::StopSending` を無視する
- sans-I/O 層は `Event::StreamReset` を発火する (`src/event.rs`) が、受信ループが観測しないため、次の `recv_stream.receive()` が `Err` を返すまで気付けない
- H3 リクエスト / レスポンスとしては RESET は「レスポンス欠落」に相当するため、明示的に `Err` で早期 return するのが素直

## 設計方針

- `H3Client::send_request` / `H3ServerConnection::accept_request` の match アームに `Event::StreamReset { stream_id: sid, error_code, .. } if sid == stream_id` と `Event::StopSending { stream_id: sid, error_code, .. } if sid == stream_id` を追加する
- `error_code` を含む `Error` (例: `Error::StreamError(u64)`) を返してループを打ち切る
- 新エラー variant の追加が必要かは実装時に判断する

## 完了条件

- ピアが RESET_STREAM または STOP_SENDING を送出した場合、`send_request` / `accept_request` がエラーで即座に return する
- 実 QUIC 統合テストを追加する (ピアが RESET を送るケース)
- `cargo test --workspace --tests` / `cargo fmt --all -- --check` / `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `crates/tokio-s2n-quic/src/h3/client.rs` (`H3Client::send_request` の match アーム)
- `crates/tokio-s2n-quic/src/h3/server.rs` (`H3ServerConnection::accept_request` の match アーム)
- `crates/tokio-s2n-quic/src/error.rs` (必要なら新エラー variant)

### 一次資料

- `refs/h3/rfc9114.txt` Section 4.1.1 / Section 8 (ストリームエラー)
