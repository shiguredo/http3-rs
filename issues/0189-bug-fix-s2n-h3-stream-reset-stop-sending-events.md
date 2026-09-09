# tokio-s2n-quic の H3 リクエスト受信ループが `Event::StreamReset` / `Event::StopSending` を無視する

- Created: 2026-08-27
- Completed: 2026-09-09
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
- `refs/quic/rfc9000.txt` Section 3.5 / Section 19.4 (STOP_SENDING / RESET_STREAM)

### closed にする理由

本 issue の設計方針「受信ループの match アームに `Event::StreamReset` / `Event::StopSending` を追加する」は成立しない。両イベントは統合層が `Connection::stream_reset` / `Connection::stop_sending` を呼んだときだけ生成され、H3 リクエスト経路はこれを呼んでいないため、追加しても到達不能なデッドコードになる。RESET_STREAM は現行の `recv_stream.receive()` の `Err(e) => return Err(crate::Error::transport(e))` で既に即時エラー終了しており、完了条件は満たされている。STOP_SENDING は s2n-quic のトランスポート層が自動で RESET_STREAM を送るため受信ループから観測できない。

これらは 0172 (`tokio-s2n-quic` に STOP_SENDING / セッション終了への RESET 応答を配線する) の対象と重複するため、本 issue は 0172 に統合して closed にする。

統合にあたり、RESET_STREAM 受信時に sans-I/O の `Connection::stream_reset` を呼んで QPACK Stream Cancellation / `blocked_by_ricnt` を掃除する経路も 0172 の対象に含めること (現行は transport エラーを返すだけで sans-I/O のストリーム状態が残る)。
