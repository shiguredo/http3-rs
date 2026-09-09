# tokio-s2n-quic に RESET_STREAM 受信と WebTransport セッション終了への応答を配線する

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
- Branch: feature/refactor-s2n-stream-error-reset
- Polished: 2026-09-09

## 目的

統合層 (tokio-s2n-quic) に次の 2 つの応答を配線する。

1. ピアが H3 リクエストストリームを RESET_STREAM した場合、sans-I/O 層の `Connection::stream_reset` を呼んで QPACK Stream Cancellation / `blocked_by_ricnt` の掃除とストリーム状態の破棄を行ってから、呼び出し側へエラーを返す。
2. WebTransport の CONNECT ストリームで WT_CLOSE_SESSION を受信した場合、draft-ietf-webtrans-http3-16 Section 6 の MUST「close or reset the stream in response」に従い、CONNECT ストリームの送信端を close (FIN) または reset する。

## 現状

- H3 リクエスト受信ループ (`H3Client::send_request` / `H3ServerConnection::accept_request`) は `recv_stream.receive()` の `Err(e)` を `crate::Error::transport(e)` として返すだけで、sans-I/O の `Connection::stream_reset` を呼ばない。このため QPACK Stream Cancellation と `blocked_by_ricnt` の掃除、`streams` エントリの破棄が行われず、接続を再利用した場合にストリーム状態が残留する
- WT の受信タスク (`run_client_connect_recv_task` / `run_server_connect_recv_task`) は `WebTransportEvent::SessionClosed` をアプリへ通知するだけで、CONNECT ストリームの送信端 (`connect_send`) に close/reset を返さない。送信側 (`WtSession::close`) は WT_CLOSE_SESSION 送信後に `connect_send.finish()` で FIN を送るが、受信側の応答が無いため送信側であるサーバー自身の `recv_event()` が `SessionClosed` を観測できない (実測: サーバーで `close(42, "server bye")` 後にサーバー自身の `recv_event()` を 3 秒 timeout で probe すると Elapsed。受信側であるクライアントの `recv_event()` は WT_CLOSE_SESSION 受信で `SessionClosed` を観測できる)
- STOP_SENDING については、s2n-quic のトランスポート層が `on_stop_sending` で RESET_STREAM を自動送出するため、RFC 9000 Section 3.5 の MUST は満たされる。s2n-quic のストリーム API では STOP_SENDING 受信が通知されないため、通常の受信ループ経路では `Connection::stop_sending` を呼べない (provider event の `Frame::StopSending` を購読すれば観測できるが、接続単位のルーティングが必要で本 issue の範囲外)
- draft-ietf-webtrans-http3-16 Section 6 (1533-1537 行):
  - "An endpoint that sends a WT_CLOSE_SESSION capsule MUST immediately send a FIN on the CONNECT Stream."
  - "The recipient MUST either close or reset the stream in response."

## 設計方針

- H3 リクエスト受信ループの `recv_stream.receive()` の `Err` が `StreamError::StreamReset { error, .. }` の場合、`internal/connection_state.rs` のラッパー経由で sans-I/O の `Connection::stream_reset(stream_id, *error, 0)` を呼んでから `crate::Error::transport(e)` を返す (`final_size` は現行 API から取得できないため 0。`connect_stream_reset` と同じ)。`stream_reset` でバッファされた QPACK Stream Cancellation を送るため `flush_qpack` を呼ぶ
- WT の受信タスクは、`SessionClosed` を検知した時点で CONNECT ストリームの送信端に close (FIN) を送る。実装は `connect_send` を所有する送信タスクを 1 つ置き、`WtSession::close` / `Drop` / 受信タスクから mpsc チャネルで `Fin` / `Reset { error_code }` を送る方式を第一候補とする (`shiguredo-rust` の「共有状態を `Mutex` / `RwLock` で保護する設計を安易に選ばない」に合わせる)。`SendStream::finish` の二重呼び出しは冪等だが、戻り値は念のため握り潰す。`SendStream::reset` は FIN が ACK 済み (送信完了) だと no-op になるため、0185 の H3_MESSAGE_ERROR reset を送る場合は FIN の ACK 前に送る順序を 0185 と調整する
- `WtSession::close` と `Drop for WtSession` の `connect_send.finish()` は送信タスク経由に整理し、重複送出を避ける
- 送信側が受信側の close 応答を `SessionClosed` として観測できることを統合テストで検証する (FIN のみの応答は `close_error_code=0` / `close_message=""` と等価。draft-16 Section 6)
- STOP_SENDING は s2n-quic のトランスポート層が処理するため本 issue の対象外とし、その旨をコメントに残す

## 完了条件

- ピアが H3 リクエストストリームを RESET_STREAM した場合、`send_request` / `accept_request` が `h3_conn.stream_reset` を呼んでからエラーで return する
- ピアが H3 リクエストストリームを RESET_STREAM するケースの統合テスト、または `internal/connection_state.rs` のラッパーに対する `#[cfg(test)]` 単体テストを追加する
- サーバー側で `WtSession::close(code, msg)` を呼び出した後、送信側の `recv_event()` で受信側の close 応答による `SessionClosed` (`close_error_code=0` / `close_message=""`) が届く。逆方向 (クライアント → サーバー) も対称に届く
- 統合テスト (`crates/tokio-s2n-quic/tests/webtransport_session_close_e2e.rs`) に「送信側で close 応答の `SessionClosed` を検知できる」ケースを追加する (既存の `server_close_delivers_session_closed_to_client` / `client_close_delivers_session_closed_to_server` は受信側の検知であり回帰確認として維持する)
- `cargo test --workspace --tests` / `cargo fmt --all -- --check` / `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `crates/tokio-s2n-quic/src/h3/client.rs` (`H3Client::send_request` の Err 分岐)
- `crates/tokio-s2n-quic/src/h3/server.rs` (`H3ServerConnection::accept_request` の Err 分岐)
- `crates/tokio-s2n-quic/src/internal/connection_state.rs` (`Connection::stream_reset` のラッパー追加)
- `crates/tokio-s2n-quic/src/webtransport/session.rs` (`WtSession` / `connect_send` の送信タスク)
- `crates/tokio-s2n-quic/src/webtransport/client.rs` / `server.rs` (`run_*_connect_recv_task` の close 応答)
- `crates/tokio-s2n-quic/tests/webtransport_session_close_e2e.rs` (echo 検証ケース追加)

### 一次資料

- `refs/h3/rfc9114.txt` Section 8 (ストリームエラー / H3 error codes)
- `refs/quic/rfc9000.txt` Section 3.5 / Section 19.4 (STOP_SENDING / RESET_STREAM)
- `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 6 (Session Termination)

### 関連 issue

- 0184 (WT_CLOSE_SESSION 受信側の close/reset 返送。0172 の重複として 2026-09-09 に closed。echo テストと close 応答の FIN/RESET 選択を本 issue に統合)
- 0189 (H3 リクエスト受信ループの RESET_STREAM 処理。0172 に統合して 2026-09-09 に closed。`stream_reset` の sans-I/O 通知を本 issue に統合)
- 0183 (`WtSession::close` を self 消費型にする)。本 issue は `close(&mut self)` を維持して先行し、echo 検証は送信側の `recv_event()` で行う。0183 は本 issue のマージ後に、close 後も close 応答を観測できる API (close 応答待ち専用メソッド等) を伴う形で調整する
- 0185 (WT_CLOSE_SESSION 後の追加データを H3_MESSAGE_ERROR で reset する。本 issue の `connect_send` 共有経路を前提とする)
