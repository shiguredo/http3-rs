# tokio-s2n-quic の受信側で WT_CLOSE_SESSION 受信後の追加ストリームデータを H3_MESSAGE_ERROR で reset しない

- Created: 2026-08-27
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-s2n-wt-post-close-message-error
- Polished: {YYYY-MM-DD}

## 目的

WT_CLOSE_SESSION 受信後に CONNECT ストリームで追加のストリームデータを受信した場合、H3_MESSAGE_ERROR でストリームを reset するようにし、draft-ietf-webtrans-http3-16 Section 6 の MUST を満たす。

## 現状

- sans-I/O 層 (`src/connection/wt_capsule.rs::process_wt_capsule_data`) は WT_CLOSE_SESSION 受信済みセッションで追加データを受け取ると `Err(Error::StreamError(ErrorCode::MessageError))` を返す
- tokio-s2n-quic の受信タスク (`run_client_connect_recv_task` / `run_server_connect_recv_task`) はこの `Err(_)` を握って `synthesized_session_closed` フォールバックで終了するのみで、CONNECT ストリームに RESET_STREAM (H3_MESSAGE_ERROR = 0x10E) を送出しない
- s2n-quic の `ReceiveStream::drop` は STOP_SENDING を送るがエラーコードが `UNKNOWN` (実質 0) になり、H3_MESSAGE_ERROR とは異なる
- draft-ietf-webtrans-http3-16 Section 6 (1539-1541 行):
  - "If any additional stream data is received on the CONNECT stream after receiving a WT_CLOSE_SESSION capsule, the stream MUST be reset with code H3_MESSAGE_ERROR."

## 設計方針

- 受信タスクが sans-I/O 層から `StreamError(MessageError)` を受け取った際に、CONNECT ストリームの送信端 (`connect_send`) に対して s2n-quic の `SendStream::reset(error_code)` で H3_MESSAGE_ERROR を送出する
- 実装は 0184 (受信側の close/reset 返送) と同一の共有経路 (`Arc<Mutex<SendStream>>` または reset 指示チャネル) を利用する。両 issue は関連するため 0184 の実装完了後にリベースして継続する
- H3_MESSAGE_ERROR 定数は `shiguredo_http3::ErrorCode::MessageError` を利用する (0x10E)
- reset 送出後は `synthesized_session_closed` のフォールバックで `SessionClosed` を発火し、通常のタスク終了処理を継続する

## 完了条件

- ピアが WT_CLOSE_SESSION 送出後に追加ストリームデータを送ってきた場合、受信側は CONNECT ストリームに RESET_STREAM(H3_MESSAGE_ERROR) を送出する
- 統合テスト (実装ケース: `raw_bytes_after_close_triggers_message_error_reset` 等) を追加し、malformed カプセル or WT_CLOSE_SESSION 後の追加データを注入した際にピア側が H3_MESSAGE_ERROR で reset を観測できることを確認する
- `cargo test --workspace --tests` / `cargo fmt --all -- --check` / `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `crates/tokio-s2n-quic/src/webtransport/client.rs` (`run_client_connect_recv_task` の Err 分岐)
- `crates/tokio-s2n-quic/src/webtransport/server.rs` (`run_server_connect_recv_task` の Err 分岐)
- `crates/tokio-s2n-quic/src/webtransport/session.rs` (`connect_send` の共有経路)
- `crates/tokio-s2n-quic/tests/webtransport_session_close_e2e.rs` (RESET 検知ケース追加)

### 一次資料

- `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 6 (Session Termination) の H3_MESSAGE_ERROR MUST 記述
- `refs/h3/rfc9114.txt` Section 8.1 (HTTP/3 error codes) の H3_MESSAGE_ERROR 定義
