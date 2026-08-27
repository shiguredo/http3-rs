# tokio-s2n-quic の H3 リクエスト受信ループで `drain_events` が `StreamError` を返した場合の RESET_STREAM 送出方針を整理する

- Created: 2026-08-27
- Completed: {YYYY-MM-DD}
- Branch: feature/refactor-s2n-h3-drain-error-reset-policy
- Polished: {YYYY-MM-DD}

## 目的

`H3Client::send_request` / `H3ServerConnection::accept_request` のループ先頭で呼ぶ `drain_events()` が `StreamError` を返した場合の RESET_STREAM 送出方針を整理し、旧実装との一貫性を確保する。

## 現状

- 0159 の修正で受信ブランチは `feed_stream_only` + `reset_stream_on_stream_error` を呼ぶが、ループ先頭の `drain_events()?` は `?` で直接伝播しており、`StreamError` を受け取っても RESET_STREAM を送らずに Err を返す
- 旧実装は `process_stream_data` (feed + drain) のエラーを一括で `reset_stream_on_stream_error` に流していた
- 実際に `drain_events` が `StreamError` を返す経路は限定的 (`check_error_state` の再現、`retry_blocked_streams` 内部の他ストリームエラー)。他ストリーム由来のエラーで本ストリームを reset するのは誤りなので現行挙動の方が正しい可能性が高いが、本ストリーム自身の `StreamError` を latch している場合の挙動は要検討
- 現状は `drain_events` の Err 経路が RESET を送らず、`Err` を返すのみのため、ピアには通知されない

## 設計方針

- `drain_events` が返す `StreamError` の発生源 (本ストリーム / 他ストリーム / 接続レベル) をコード上で判別できるかを検討する
- 判別できる場合: 本ストリーム由来の `StreamError` のみ `reset_stream_on_stream_error` を呼び、他ストリーム由来はスキップする
- 判別できない場合: 現行挙動 (`?` で伝播、RESET 送らず) を維持し、その理由を doc コメントに明記する
- どちらの方針をとるかは sans-I/O 層の実装 (`src/connection/mod.rs::drain_events`) を精査してから決める

## 完了条件

- `drain_events` の `StreamError` に対する RESET_STREAM 送出方針が明確になる (doc または実装で明示)
- 旧実装との一貫性が保たれる (退行しない)
- `cargo test --workspace --tests` / `cargo fmt --all -- --check` / `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `crates/tokio-s2n-quic/src/h3/client.rs` (`H3Client::send_request` の受信ループ)
- `crates/tokio-s2n-quic/src/h3/server.rs` (`H3ServerConnection::accept_request` の受信ループ)
- 参考: `src/connection/mod.rs::drain_events` / `check_error_state` / `retry_blocked_streams`

### 一次資料

- `refs/h3/rfc9114.txt` Section 8 (エラーハンドリング)
