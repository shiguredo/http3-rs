# client 側の pending SessionClosed 後の追加 DATA reset を検証するテストがない

- Created: 2026-09-12
- Completed: {YYYY-MM-DD}
- Branch: feature/test-s2n-client-pending-close-reset
- Polished: {YYYY-MM-DD}

## 目的

0206 で `run_client_connect_recv_task_inner` の `pending_wt_events` ループを変更し、終端 SessionClosed を転送しても FIN / Err まで読み続けるようにしたが、client 側経路を検証するテストがない。raw サーバー役のヘルパーを追加して回帰を検知できるようにする。

## 現状

- `crates/tokio-s2n-quic/src/webtransport/client.rs` の `run_client_connect_recv_task_inner` は pending の SessionClosed を転送したあとも受信を継続する
- client の pending 経路は「2xx レスポンスと WT_CLOSE_SESSION が同一 receive チャンクで届き、SessionEstablished 処理後に SessionClosed が pending に積まれる場合」に到達する
- サーバー側は `crates/tokio-s2n-quic/tests/webtransport_post_close_reset_e2e.rs` の `pending_session_closed_then_additional_data_triggers_message_error_reset` で検証済みだが、client 側は raw サーバー役のヘルパーがなく未検証
- `RawWtClient` はクライアント役のみで、raw サーバー役のヘルパーは存在しない (0206 のレビューで検出)

## 設計方針

- `crates/tokio-s2n-quic/tests/helpers/` に raw サーバー役のヘルパーを追加する (`WtServer` ではなく生の s2n-quic server で CONNECT 応答とカプセルを直接操作する)
- 2xx レスポンスと WT_CLOSE_SESSION を同一 write で返し、その後に追加 DATA を送るケースで、client が RESET_STREAM(H3_MESSAGE_ERROR) を送出することを検証する
- 既存の `RawWtClient` / `certs.rs` と同じ流儀 (`#[path = "helpers/..."]`、モック・スタブ不使用) に従う

## 完了条件

- raw サーバー役ヘルパーが追加される
- client 側の pending SessionClosed + 追加 DATA で RESET_STREAM(H3_MESSAGE_ERROR) が送出されるテストが追加される
- 0206 の client 側変更を戻すとテストが失敗する (回帰検知できる)
- `cargo test --workspace --tests` / `cargo fmt --all -- --check` / `cargo clippy --workspace --all-targets -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `crates/tokio-s2n-quic/tests/helpers/` (raw サーバー役ヘルパーを追加)
- `crates/tokio-s2n-quic/tests/webtransport_post_close_reset_e2e.rs` (テスト追加)
- `crates/tokio-s2n-quic/src/webtransport/client.rs` (`run_client_connect_recv_task_inner`)

### 一次資料

- `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 6

### 関連 issue

- 0206 (server 側の同種テストを追加。本 issue は client 側の対称テスト)
