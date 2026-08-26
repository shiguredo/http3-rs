# tokio-s2n-quic の CONNECT ストリーム受信側がドロップされカプセルチャネルが断絶する

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
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

### 関連ファイル

- `crates/tokio-s2n-quic/src/webtransport/client.rs` (`WtClient::connect`)
- `crates/tokio-s2n-quic/src/webtransport/server.rs` (`WtSessionRequest` の構造体フィールドと `from_connection` / `accept` の受け渡し)
- `crates/tokio-s2n-quic/src/webtransport/session.rs` (`WtSession` に受信タスク・接続状態・イベント受信端を追加)
- `crates/tokio-s2n-quic/src/internal/connection_state.rs` (アブラプトクローズ時に `stream_reset` を sans-I/O 層へ渡す経路)
- 一次資料: `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 6、`refs/quic/rfc9000.txt` Section 3.5
