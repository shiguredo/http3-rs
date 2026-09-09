# tokio-s2n-quic の受信側で WT_CLOSE_SESSION 受信後の追加ストリームデータを H3_MESSAGE_ERROR で reset しない

- Created: 2026-08-27
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-s2n-wt-post-close-message-error
- Polished: 2026-09-09

## 目的

WT_CLOSE_SESSION 受信後に CONNECT ストリームで追加のストリームデータを受信した場合、H3_MESSAGE_ERROR でストリームを reset するようにし、draft-ietf-webtrans-http3-16 Section 6 の MUST を満たす。

## 現状

- WT_CLOSE_SESSION を受信すると sans-I/O 層の `handle_wt_capsule` が `terminate_wt_session_with` を呼び、セッションを `wt_sessions` から除去して `closed_wt_sessions` に tombstone 登録する (`src/connection/wt_session.rs`)。このため以後の CONNECT ストリームの追加 DATA は `src/connection/wt_capsule.rs` の `handle_wt_data_frame` の `closed_wt_sessions` 分岐で `Err(Error::StreamError(ErrorCode::MessageError))` を返す (`process_wt_capsule_data` の `close_session_received` ガードは、フラグ設定とセッション除去が同時に起きるため到達しない)
- tokio-s2n-quic の受信タスク (`run_client_connect_recv_task` / `run_server_connect_recv_task`) はこの `Err(_)` を握るが、`drain_events` で実 `SessionClosed` を先に取り出して `event_tx` に送るだけで、CONNECT ストリームに RESET_STREAM (H3_MESSAGE_ERROR = 0x10E) を送出しない (`synthesized_session_closed` は実 `SessionClosed` が無い場合のみ使う)
- 受信タスクは `SessionClosed` を `event_tx` に送ると直ちに return するため、WT_CLOSE_SESSION と追加 DATA が別々の `recv_stream.receive()` チャンクで届く場合、追加 DATA は観測されない。現状 `Err(MessageError)` に到達するのは、WT_CLOSE_SESSION カプセルと追加 DATA フレームが同一 `process_stream_data` 呼び出し (同一 receive チャンク) に含まれる場合に限られる
- 同一 DATA フレーム内で WT_CLOSE_SESSION カプセルに続く追加バイトは、`process_wt_capsule_data` の while ループが WT_CLOSE_SESSION 処理によるセッション除去で終了し、残りの capsule_buf をデコードしないため `Err` にならず破棄される
- s2n-quic の `ReceiveStream::drop` は STOP_SENDING を送るがエラーコードが `UNKNOWN` (実質 0) になり、H3_MESSAGE_ERROR とは異なる
- draft-ietf-webtrans-http3-16 Section 6 (1539-1541 行):
  - "If any additional stream data is received on the CONNECT stream after receiving a WT_CLOSE_SESSION capsule, the stream MUST be reset with code H3_MESSAGE_ERROR."

## 設計方針

- 受信タスクが sans-I/O 層から `StreamError(MessageError)` を受け取った際に、CONNECT ストリームの送信端 (`connect_send`) に対して s2n-quic の `SendStream::reset(error_code)` で H3_MESSAGE_ERROR を送出する
- 別 receive チャンクで届く追加 DATA も MUST の対象とするため、受信タスクは `SessionClosed` を転送した直後に return せず、CONNECT ストリームの読み取りを継続する。`Ok(None)` (FIN) または `Err` でタスクを終了し、`Err(StreamError(MessageError))` のときだけ RESET_STREAM(H3_MESSAGE_ERROR) を送る
- 同一 DATA フレーム内で WT_CLOSE_SESSION に続く追加バイトも MUST の対象とする。`process_wt_capsule_data` は WT_CLOSE_SESSION の処理でセッションが除去されると残りの capsule_buf をデコードせずに破棄するため、終了済みセッションへの追加バイトが残っている場合は `Err(StreamError(MessageError))` を返すようにする
- `connect_send` の共有経路は 0172 が実装する (チャネル方式を第一候補とする。`Arc<Mutex<SendStream>>` を採る場合は `WtSession::close` の `send().await` を跨ぐロックを避ける根拠をコメントに残す)。0172 が送信方向を RESET してもピアの送信方向は閉じないため、WT_CLOSE_SESSION 後の追加 DATA は依然到着しうる。競合点は CONNECT ストリームの送信方向をどちらが RESET するかとエラーコードであり、0172 の完了後に実装順序と優先関係を調整する
- H3_MESSAGE_ERROR 定数は `shiguredo_http3::ErrorCode::MessageError` を利用する (0x10E)
- reset 送出後は実 `SessionClosed` を優先して転送し、無い場合のみ `synthesized_session_closed` で終了する。実 `SessionClosed` を転送済みの場合は `synthesized_session_closed` を再送しない (終端イベントの二重配送防止)

## 完了条件

- ピアが WT_CLOSE_SESSION 送出後に CONNECT ストリームへ追加 DATA を送ってきた場合、受信側は CONNECT ストリームに RESET_STREAM(H3_MESSAGE_ERROR) を送出する。追加データが別 DATA フレーム、別 receive チャンク、同一 DATA フレーム内の後続バイトのいずれであっても適用する
- 統合テスト (`crates/tokio-s2n-quic/tests/webtransport_session_close_e2e.rs`。実装ケース名: `raw_bytes_after_close_triggers_message_error_reset` 等) で、WT_CLOSE_SESSION と追加 DATA フレームを同一 write に含めるケース、WT_CLOSE_SESSION 送信後に別 write で追加 DATA を送るケース、同一 DATA フレーム内で WT_CLOSE_SESSION に続けて追加バイトを送るケースを検証し、ピア側が RESET_STREAM(H3_MESSAGE_ERROR) を観測できることを確認する。`WtSession::close` は WT_CLOSE_SESSION 送信直後に FIN を送るため追加 DATA を後から注入できず、raw QUIC クライアント等で CONNECT ストリームを直接操作するテスト基盤を用意する
- `cargo test --workspace --tests` / `cargo fmt --all -- --check` / `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `crates/tokio-s2n-quic/src/webtransport/client.rs` (`run_client_connect_recv_task` の Err 分岐 / `SessionClosed` 後の読み取り継続)
- `crates/tokio-s2n-quic/src/webtransport/server.rs` (`run_server_connect_recv_task` の Err 分岐 / `SessionClosed` 後の読み取り継続)
- `crates/tokio-s2n-quic/src/webtransport/session.rs` (`connect_send` の共有経路。0172 の実装に依存)
- `src/connection/wt_capsule.rs` (`handle_wt_data_frame` の `closed_wt_sessions` 分岐 / `process_wt_capsule_data` の同一 DATA フレーム内後続バイト検査)
- `crates/tokio-s2n-quic/tests/webtransport_session_close_e2e.rs` (RESET 検知ケース追加)

### 一次資料

- `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 6 (Session Termination) の H3_MESSAGE_ERROR MUST 記述
- `refs/h3/rfc9114.txt` Section 8.1 (HTTP/3 error codes) の H3_MESSAGE_ERROR 定義

### 関連 issue

- 0172 (`tokio-s2n-quic` に STOP_SENDING / セッション終了への RESET 応答を配線する。`connect_send` の共有経路と `SessionClosed` 時の CONNECT ストリーム RESET を担当)
- 0184 (WT_CLOSE_SESSION 受信側の close/reset 返送。0172 の重複として 2026-09-09 に closed。本 issue の設計は 0184 ではなく 0172 を前提とする)
