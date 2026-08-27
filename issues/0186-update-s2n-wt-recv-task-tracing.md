# tokio-s2n-quic の CONNECT ストリーム受信タスクの Err パスに tracing ログを追加する

- Created: 2026-08-27
- Completed: {YYYY-MM-DD}
- Branch: feature/update-s2n-wt-recv-task-tracing
- Polished: {YYYY-MM-DD}

## 目的

`tokio-s2n-quic` の CONNECT ストリーム受信タスクが sans-I/O 層のエラー (H3_MESSAGE_ERROR 等) や s2n-quic のストリームエラーで終了する際に、原因を追跡できるように `tracing` 経由でエラーログを出力する。運用時の問題切り分けを容易にする。

## 現状

- `crates/tokio-s2n-quic/src/webtransport/client.rs::run_client_connect_recv_task` および `crates/tokio-s2n-quic/src/webtransport/server.rs::run_server_connect_recv_task` の Err 分岐 (`Ok(Some)` / `Ok(None)` / `Err(_)` / sans-I/O 層 Err の後段) では、エラー内容を失った状態で `synthesized_session_closed` を送るのみでログ出力がない
- 受信タスクが早期終了した場合、アプリからは `recv_event()` が `None` を返すだけで「なぜ終了したのか」が追跡できない (`SessionClosed` の正常終了と protocol error による強制終了が区別不能)
- `crates/tokio-s2n-quic` は `tracing` を依存に持たない (`Cargo.toml` 参照)

## 設計方針

- `tokio-s2n-quic` の依存に `tracing = "0.1"` を追加する (`shiguredo-rust` 規約: ログは tracing を使う)
- 受信タスクの各 Err 分岐で以下をログ出力する:
  - `tracing::warn!(session_id, error = ?e, "process_stream_data failed")`
  - `tracing::warn!(session_id, error = ?e, "connect_stream_reset failed")`
  - `tracing::warn!(session_id, "connect stream ended abruptly")` (recv_stream.receive() Err)
- ログメッセージは英語 (`AGENTS.md` 規約)
- ログレベル: 通信断は `warn`、実装バグ相当は `error` を使い分ける
- `WtClient::connect` / `WtSessionRequest::from_connection` のハンドシェイクエラーには本 issue の範囲としない

## 完了条件

- `tokio-s2n-quic` の `Cargo.toml` に `tracing` 依存が追加される
- 受信タスクの Err 分岐に `tracing::warn!` / `tracing::error!` が追加される (session_id をフィールドとして含める)
- ログメッセージは英語で、AGENTS.md 規約に準拠する
- `cargo test --workspace --tests` / `cargo fmt --all -- --check` / `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `crates/tokio-s2n-quic/Cargo.toml` (`tracing` 依存追加)
- `crates/tokio-s2n-quic/src/webtransport/client.rs` (`run_client_connect_recv_task` の Err 分岐)
- `crates/tokio-s2n-quic/src/webtransport/server.rs` (`run_server_connect_recv_task` の Err 分岐)
- `crates/tokio-s2n-quic/src/webtransport/session.rs` (`synthesized_session_closed` を呼ぶ経路周辺)

### 一次資料

- 参考: `crates/tokio-msquic/` や `examples/wt_server/src/webtransport.rs` の既存 tracing 使用例
