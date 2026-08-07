# tokio-s2n-quic の CONNECT ストリーム受信側がドロップされカプセルチャネルが断絶する

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-s2n-wt-connect-recv-drop
- Polished: {YYYY-MM-DD}

## 目的

セッション確立後に CONNECT ストリームの受信側を保持し、ピアからのセッションクローズ通知・フロー制御カプセルを受信できるようにする。

## 現状

- `crates/tokio-s2n-quic/src/webtransport/client.rs` の `WtClient::connect` は CONNECT ストリームを `split()` し、レスポンス待ちの後 `recv_stream` を `WtSession::new` に渡さずドロップする。`webtransport/server.rs` の `WtSessionRequest::accept` 経由も同様
- s2n-quic の `ReceiveStream` はドロップ時に STOP_SENDING を送出するため、サーバー → クライアント方向のカプセル (WT_CLOSE_SESSION / WT_MAX_DATA / WT_MAX_STREAMS) が届かずセッション終了検知が不可能になる
- 同一リポジトリの `examples/wt_server/src/webtransport.rs` は「ドロップすると STOP_SENDING が送信されるため保持する」とコメントで自認しており、クレート本体だけがこの認識を欠く
- ピアの WT_CLOSE_SESSION を受信・処理する経路も存在しない (`WtSession::close` は送信専用)

## 設計方針

- `WtSession` に CONNECT ストリームの受信側を保持させ、受信タスクでカプセル (WT_CLOSE_SESSION 等) を処理してセッションクローズを検知できるようにする
- クローズ検知 API (`closed` 待機・イベント通知) を追加する

## 完了条件

- ピアがセッションをクローズするとアプリ側で検知できる
- フロー制御カプセルが受信・処理される
- テストが追加される
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `crates/tokio-s2n-quic/src/webtransport/client.rs` (`WtClient::connect`)
- `crates/tokio-s2n-quic/src/webtransport/server.rs` (セッション受付経路)
- `crates/tokio-s2n-quic/src/webtransport/session.rs` (`WtSession`)
- 一次資料: `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 6
