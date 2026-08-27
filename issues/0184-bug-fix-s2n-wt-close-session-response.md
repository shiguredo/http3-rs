# tokio-s2n-quic の受信側で WT_CLOSE_SESSION 受信後に close または reset を返さない

- Created: 2026-08-27
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-s2n-wt-close-session-response
- Polished: {YYYY-MM-DD}

## 目的

WT_CLOSE_SESSION 受信側が CONNECT ストリームに close (FIN) または reset を送出するようにし、draft-ietf-webtrans-http3-16 Section 6 の MUST を満たす。

## 現状

- `crates/tokio-s2n-quic/src/webtransport/session.rs` の `WtSession` は WT_CLOSE_SESSION 受信を検知して `WebTransportEvent::SessionClosed` をアプリに通知するが、CONNECT ストリームの送信端 (`connect_send`) からピアに close (FIN) や reset を送り返さない
- 送信側 (`WtSession::close`) が WT_CLOSE_SESSION 送信後に `connect_send.finish()` (FIN) を送出しても、受信側はアプリの手動 close 呼び出しがない限り peer 側の close 応答を検知できず、送信側の `recv_event()` が `SessionClosed` を受け取れないケースが発生する (実測: サーバー側で `close(42, "server bye")` を呼び出したあと `recv_event()` を 3 秒 timeout で probe すると Elapsed になる)
- draft-ietf-webtrans-http3-16 Section 6 (1533-1537 行):
  - "An endpoint that sends a WT_CLOSE_SESSION capsule MUST immediately send a FIN on the CONNECT Stream."
  - "The recipient MUST either close or reset the stream in response."

## 設計方針

- 受信側 (`run_client_connect_recv_task` / `run_server_connect_recv_task`) が `WebTransportEvent::SessionClosed` を検知した時点で、CONNECT ストリームの送信端に close (FIN) または reset を送出する
- 実装アプローチ:
  - 受信タスクに `connect_send` の共有ハンドル (`Arc<Mutex<SendStream>>`) または close 指示用の別チャネル (`oneshot`) を渡す
  - タスク側で FIN 送出 (`connect_send.finish()`) を実施
  - `WtSession::close` が既に呼ばれている場合との排他は idempotent な finish 呼び出しで担保する (`SendStream::finish` は 2 回目以降エラーになるが握り潰す)
- 併せて `Drop for WtSession` の `connect_send.finish()` との重複を整理する

## 完了条件

- サーバー側で `WtSession::close(code, msg)` を呼び出した後、クライアント側の `recv_event()` で `SessionClosed { close_error_code: code, close_message: msg, .. }` が届く (現状は届かない)
- 逆方向 (クライアント → サーバー) も同様に対称的に届く
- 統合テスト (`crates/tokio-s2n-quic/tests/webtransport_session_close_e2e.rs`) に「送信側でも `SessionClosed` の echo を検知できる」ケースを追加する
- `cargo test --workspace --tests` / `cargo fmt --all -- --check` / `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `crates/tokio-s2n-quic/src/webtransport/session.rs` (`WtSession` の Drop、`connect_send` の受信タスクへの共有)
- `crates/tokio-s2n-quic/src/webtransport/client.rs` (`run_client_connect_recv_task`)
- `crates/tokio-s2n-quic/src/webtransport/server.rs` (`run_server_connect_recv_task`)
- `crates/tokio-s2n-quic/tests/webtransport_session_close_e2e.rs` (echo 検証ケース追加)

### 一次資料

- `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 6 (Session Termination) の MUST 記述
