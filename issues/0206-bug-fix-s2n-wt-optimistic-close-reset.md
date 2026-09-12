# tokio-s2n-quic の楽観的カプセル送信経路で WT_CLOSE_SESSION 後の追加 DATA が H3_MESSAGE_ERROR で reset されない

- Created: 2026-09-10
- Completed: 2026-09-12
- Branch: feature/fix-s2n-wt-optimistic-close-reset
- Polished: {YYYY-MM-DD}

## 目的

楽観的カプセル送信 (2xx 応答前に CONNECT ストリームへ送出される DATA) 経路でも、WT_CLOSE_SESSION 受信後の追加ストリームデータを RESET_STREAM(H3_MESSAGE_ERROR) で拒否し、draft-ietf-webtrans-http3-16 Section 6 の MUST を満たす。

## 現状

- `src/connection/wt_session.rs` の `establish_wt_session_server` は、Pending セッションにバッファリングされたカプセルを `process_wt_capsule_data` で処理するが、`Err` を握りつぶして `terminate_wt_session` を呼ぶだけである。WT_CLOSE_SESSION 処理で既にセッションが `wt_sessions` から除去されているため `terminate_wt_session` は no-op となり、RESET_STREAM が送られない
- `crates/tokio-s2n-quic/src/webtransport/client.rs` / `server.rs` の受信タスクは、`pending_wt_events` に終端 `SessionClosed` が含まれる場合にそれを転送した時点で return する。このため確立前にバッファリングされた追加 DATA を読まず、確立後経路の reset 送出にも到達しない
- 確立後の経路は 0185 で対応済みで、`handle_wt_data_frame` の tombstone 分岐と `process_wt_capsule_data` の後続バイト検査が `Err(MessageError)` を返し、受信タスクが RESET_STREAM(H3_MESSAGE_ERROR) を送る
- 通常の `WtClient` は 2xx 前に CONNECT ストリームへ追加 DATA を送らないため通常経路では到達しないが、raw ピアや将来の実装では到達し得る
- draft-ietf-webtrans-http3-16 Section 6 (1539-1541 行):
  - "If any additional stream data is received on the CONNECT stream after receiving a WT_CLOSE_SESSION capsule, the stream MUST be reset with code H3_MESSAGE_ERROR."

## 設計方針

- `establish_wt_session_server` が `process_wt_capsule_data` の `Err` を握りつぶさず、呼び出し元 (`send_response` 経由) へ伝播する。戻り値の変更が広い影響を持つ場合は、「reset すべき」情報をイベントとして受信タスクへ引き継ぐ方式も検討する
- 受信タスクの `pending_wt_events` 先頭ループは終端 `SessionClosed` で return せず、確立後の経路と同様にピアの FIN / Err まで読み続ける
- H3_MESSAGE_ERROR 定数は `shiguredo_http3::ErrorCode::MessageError` を利用する (0x10E)

## 完了条件

- ピアが 2xx 応答前に WT_CLOSE_SESSION + 追加 DATA を送った場合、受信側は RESET_STREAM(H3_MESSAGE_ERROR) を送出する
- raw QUIC テストで 2xx 前に注入するケースを検証し、ピア側が RESET_STREAM(H3_MESSAGE_ERROR) を観測できることを確認する
- `cargo test --workspace --tests` / `cargo fmt --all -- --check` / `cargo clippy --workspace --all-targets -- -D warnings` が通る

## 解決方法

### 修正内容

- `Connection::establish_wt_session_server` の戻り値を `Result<(), Error>` に変更し、楽観的カプセル送信でバッファリングされたデータの検証に失敗した場合はセッションを終了してからエラーを伝播する。`send_response` はこのエラーを呼び出し元へ返す。検証は `qpack_encoder.track_section` より前に行い、未送出セクションがエンコーダーに残らないようにする
- `WtSessionRequest::accept` は `send_response` のエラー時に 2xx を送らず、`reset_stream_on_stream_error` で CONNECT ストリームへ RESET_STREAM(H3_MESSAGE_ERROR) を送ってからエラーを返す
- `run_server_connect_recv_task_inner` / `run_client_connect_recv_task_inner` の `pending_wt_events` ループは終端 `SessionClosed` を転送しても return せず、確立後と同じくピアの FIN / Err まで読み続ける (`session_closed_delivered` は pending ループで転送した場合も記録する)

### テスト

- `src/connection/mod.rs` に `test_wt_optimistic_capsule_trailing_rejected_on_establish` を追加する (2xx 前にバッファリングした WT_CLOSE_SESSION + 追加 DATA が `send_response` の `Error::StreamError(MessageError)` になることを検証)
- `crates/tokio-s2n-quic/tests/webtransport_post_close_reset_e2e.rs` に 2 件追加する
  - `optimistic_pre_response_data_triggers_message_error_reset`: CONNECT と同時 (2xx 前) に送った WT_CLOSE_SESSION + 追加 DATA でピアが RESET_STREAM(H3_MESSAGE_ERROR) を観測する
  - `pending_session_closed_then_additional_data_triggers_message_error_reset`: 確立前にバッファリングした WT_CLOSE_SESSION を受けて確立した後、追加 DATA を送るとピアが RESET_STREAM(H3_MESSAGE_ERROR) を観測する
- `tests/helpers/wt_raw_client.rs` に `connect_with_optimistic_payload` を追加する (CONNECT リクエストと 2xx 前ペイロードを同一 write で送り、確立を待たずに返る)
- 各修正を個別に戻すと対応テストが失敗する (FIN を観測する) ことを確認し、回帰検知が機能することを確かめる

### 検証結果

- `cargo test --workspace --tests` / `cargo fmt --all -- --check` / `cargo clippy --workspace --all-targets -- -D warnings` が通る

### 関連ファイル

- `src/connection/wt_session.rs` (`establish_wt_session_server`)
- `crates/tokio-s2n-quic/src/webtransport/client.rs` / `server.rs` (`pending_wt_events` 先頭ループ)
- `crates/tokio-s2n-quic/tests/webtransport_post_close_reset_e2e.rs` (raw QUIC テスト基盤)

### 一次資料

- `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 6 (Session Termination) の H3_MESSAGE_ERROR MUST 記述
- `refs/h3/rfc9114.txt` Section 8.1 (HTTP/3 error codes) の H3_MESSAGE_ERROR 定義

### 関連 issue

- 0185 (確立後の WT_CLOSE_SESSION 追加 DATA を H3_MESSAGE_ERROR で reset する。本 issue はその未対応経路)
