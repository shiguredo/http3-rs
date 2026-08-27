# tokio-s2n-quic の CONNECT ストリーム受信側がドロップされカプセルチャネルが断絶する

- Created: 2026-08-08
- Completed: 2026-08-27
- Branch: feature/fix-s2n-wt-connect-recv-drop
- Polished: 2026-08-26

## 目的

セッション確立後に CONNECT ストリームの受信側を保持し、ピアからのセッションクローズ通知・フロー制御カプセルを受信できるようにする。

## 現状

- `crates/tokio-s2n-quic/src/webtransport/client.rs` の `WtClient::connect` は CONNECT ストリームを `split()` し、レスポンス待ちの後 `recv_stream` を `WtSession::new` に渡さずドロップする。`webtransport/server.rs` の `WtSessionRequest` も `from_connection` の終端で同様にドロップする (`accept` にも受信側の受け渡しがない)
- s2n-quic の `ReceiveStream` はドロップ時に STOP_SENDING を送出する (`State::drop`。s2n-quic-transport の `stream/api.rs`)。そのため、サーバー → クライアント方向のカプセル (WT_CLOSE_SESSION / WT_MAX_DATA / WT_MAX_STREAMS) が届かずセッション終了検知が不可能になる。クライアント → サーバー方向のカプセル (例: `WtSession::close` の WT_CLOSE_SESSION) も受信側が読んでいないため処理されない
- 同一リポジトリの `examples/wt_server/src/webtransport.rs` は「ドロップすると STOP_SENDING が送信されるため保持する」とコメントで自認しており、クレート本体だけがこの認識を欠く
- sans-I/O 層 (`src/connection/wt_capsule.rs` の `process_wt_capsule_data`) には WT_CLOSE_SESSION の受信処理が既に存在するが、tokio-s2n-quic が CONNECT ストリームの受信データを sans-I/O 層にフィードしていないため到達しない

## 設計方針

- CONNECT ストリームの受信側 (`ReceiveStream`) はクローン・再 split ができないため受信タスクへ移動する。`WtSession` は受信タスクのハンドルと、アプリ向けイベント受信用の mpsc チャネルの受信端、接続状態 (`ClientConnectionState` / `ServerConnectionState` の `Arc`) を保持する。`WtSession` ドロップ時に受信タスクを abort する (受信側がドロップされ STOP_SENDING が送出されるが、セッション終了時の挙動として妥当)
- 受信タスクは `feed_stream` + `drain_events` で sans-I/O 層の既存カプセル処理 (`process_wt_capsule_data`) を利用してセッションクローズを検知する。FIN 経由のクローズ検知は sans-I/O 層の既存機能 (`handle_wt_stream_end`) を利用できる
- 受信タスクの終了条件: SessionClosed 検知時 / FIN 受信時 / `recv_stream.receive()` が `Err` を返したとき (RESET_STREAM 等の abrupt close。draft-16 Section 6 はクリーン / アブラプト両方の CONNECT ストリームクローズを終了条件とする) / `WtSession` ドロップ時
- アブラプトクローズ時もアプリへセッション終了を通知する。受信タスクは sans-I/O 層の `stream_reset` API (`ClientConnection` / `ServerConnection::stream_reset`) に CONNECT ストリームのリセットを渡し、`handle_wt_stream_reset` の CONNECT ストリーム分岐 (`terminate_wt_session`) で `WebTransportEvent::SessionClosed` を生成させる。s2n-quic の `StreamError` は final size を公開しないため `final_size` は 0 を渡す (CONNECT ストリームのリセットはセッション終了に直行し final_size を使用しないため影響なし)
- イベント通知は mpsc チャネル方式を基本とする。受信タスクが `drain_events` で回収したイベントのうちセッション状態系の WebTransport 関連 (`WebTransportEvent::SessionClosed` 等) を選別し、mpsc チャネル経由でアプリに伝える (`WebTransportEvent::Capsule` の行き先は 0181 で定める)。アプリが受信する公開 API (例: `WtSession::recv_event`) を追加する
- ハンドシェイクループ (`WtClient::connect` / `WtSessionRequest::from_connection`) が `drain_events` で回収したイベントのうち `HeadersEnd` 以外 (`SessionClosed` / `Capsule` 等) は破棄せず、確立後のアプリ通知経路 (mpsc) に渡す。セッション確立と同時に WT_CLOSE_SESSION が到着する場合でも通知が失われないようにする
- フロー制御カプセルは受信して sans-I/O 層でイベント化 (`WebTransportEvent::Capsule`) するところまでを本 issue の範囲とし、`webtransport::Session::process_capsule` による送信クレジット更新は 0181 で対応する
- セッションクローズ検知後の関連ストリームの後始末 (draft-16 Section 6 の WT_SESSION_GONE での RESET) は、tokio-s2n-quic のストリーム型 (`WtBiStream` / `WtSendStream` / `WtRecvStream`) にリセット API がなくアプリ所有であるため、本 issue の範囲外とし別 issue で対応する

## 完了条件

- ピアがセッションをクローズするとアプリ側で検知できる (クリーンクローズ / WT_CLOSE_SESSION / アブラプトクローズのすべて)
- フロー制御カプセルが受信され sans-I/O 層でイベント化される
- テストが追加される (実 QUIC 接続のループバック統合テスト。モック・スタブは使わない。0158 の修正後に `WtSession::close` でピアをクローズして検知を確認する。0158 の未修正時は `WtSession::close` のカプセルが DATA フレームで包まれずピアに届かないため)
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 変更内容

- `crates/tokio-s2n-quic/src/webtransport/session.rs` の `WtSession`:
  - `event_rx: mpsc::Receiver<WebTransportEvent>` と `recv_task: JoinHandle<()>` フィールドを追加
  - 公開 API `recv_event() -> Option<WebTransportEvent>` を追加。届き得るイベントは `SessionClosed` / `SessionDraining` / `BufferedStreamRejected`
  - `Drop for WtSession` を追加し、drop 時に `connect_send.finish()` (FIN 送出) と `recv_task.abort()` を呼ぶ
  - `is_forwardable_wt_event` と `synthesized_session_closed` を共通ヘルパーとして追加
- `crates/tokio-s2n-quic/src/webtransport/client.rs` の `WtClient::connect`:
  - ハンドシェイクループで forwardable WebTransport イベントを `pending_wt_events` にバッファする (200 レスポンスと同一 receive で到着した終端カプセル等の取りこぼしを防ぐ)
  - CONNECT ストリームの受信タスク `run_client_connect_recv_task` を追加
- `crates/tokio-s2n-quic/src/webtransport/server.rs` の `WtSessionRequest`:
  - `recv_stream` / `pending_wt_events` フィールドを追加
  - `from_connection` のハンドシェイクループで forwardable WebTransport イベントを `pending_wt_events` にバッファする
  - `accept()` で `send_response` 後に `drain_events` を追加実施し、`establish_wt_session_server` で発火した楽観バッファ由来のイベントを `pending_wt_events` に append する
  - CONNECT ストリームの受信タスク `run_server_connect_recv_task` を追加
- `crates/tokio-s2n-quic/src/internal/connection_state.rs`:
  - `Client/ServerConnectionState` に `connect_stream_reset` メソッドを追加 (sans-I/O 層の `stream_reset` を呼び `final_size = 0` で `SessionClosed` を生成させる)
- 受信タスクの動作:
  - `pending_wt_events` を先に流したあと、CONNECT ストリームからの受信ループを開始
  - `Ok(Some)` で `process_stream_data` を呼び、forwardable WebTransport イベントを `event_tx` に流す
  - `Ok(None)` (FIN) で `process_stream_data(_, &[], true)` を呼び、sans-I/O 層の `terminate_wt_session` に `SessionClosed { close_error_code: 0, close_message: "" }` を発火させる (draft-16 Section 6 のクリーンクローズ等価)
  - `Err(_)` (RESET_STREAM 等) で `connect_stream_reset` を呼び `SessionClosed` を発火させる
  - sans-I/O 層が `Err` を返した場合は先に `drain_events` を試み、`SessionClosed` を優先配信する。それも無い場合は `synthesized_session_closed` (close_error_code=0 / message="") をフォールバック配信する
  - `SessionClosed` を配信したらタスク終了
- `crates/tokio-s2n-quic/tests/webtransport_session_close_e2e.rs` を新規追加し、実 QUIC ループバック統合テスト 3 件を実装:
  - `server_close_delivers_session_closed_to_client`: サーバー側 `WtSession::close(42, "server bye")` を検知
  - `client_close_delivers_session_closed_to_server`: クライアント側 `WtSession::close(7, "client bye")` を検知
  - `client_drop_delivers_clean_close_to_server`: クライアント側の `drop` によるクリーンクローズ (FIN のみ) で `close_error_code = 0`, `close_message = ""` を検知
- `CHANGES.md` の `## develop` セクションに `[FIX]` エントリを追加した

### 対象外

- draft-16 Section 6 の追加 MUST 対応: 「受信側は WT_CLOSE_SESSION 受信後に相手に close または reset を返す」「WT_CLOSE_SESSION 受信後の追加データは H3_MESSAGE_ERROR で reset する」「Application Error Message 1024 バイト制限」「STOP_SENDING の error code を明示的に WT_SESSION_GONE にする」「関連ストリームを WT_SESSION_GONE で reset する」は本 issue の範囲外 (別 issue で対応)
- フロー制御カプセル (`WebTransportEvent::Capsule`) のアプリ配信・送信クレジット更新は 0181 で対応する
- WT データストリームの RESET_STREAM / STOP_SENDING を sans-I/O 層へフィードする経路の追加、DATAGRAM フレームの sans-I/O 層フィード追加は別 issue で対応する (現状 `Datagram` / `StreamReset` / `StreamStopSending` は tokio-s2n-quic 内で発火経路が無いため `is_forwardable_wt_event` の対象外)
- `run_client_connect_recv_task` / `run_server_connect_recv_task` の重複除去 (trait 抽出等) は別 issue で対応する
- アブラプトクローズ (真の RESET_STREAM) を検証する統合テストは s2n-quic の API 制約により本 issue では見送る (別 issue で対応)

### 一次資料

- `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 6 (Session Termination)
- `refs/quic/rfc9000.txt` Section 3.5 (STOP_SENDING semantics)
