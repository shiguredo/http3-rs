# tokio-s2n-quic の `H3ServerConnection` を並列複数リクエストに対応させる

- Created: 2026-08-27
- Completed: {YYYY-MM-DD}
- Branch: feature/add-s2n-h3-concurrent-requests
- Polished: {YYYY-MM-DD}

## 目的

`H3ServerConnection::accept_request` が接続共有イベントキューを排他ドレインする現行制約 (「1 接続 1 リクエスト逐次処理」) を解消し、ピアが並列に複数のリクエストストリームを開けるようにする。

## 現状

- `crates/tokio-s2n-quic/src/h3/server.rs` の `H3ServerConnection::accept_request` は `&mut self` で接続共有イベントキューを排他ドレインする
- match アームに `Event::Header { stream_id: sid } if sid == stream_id` などのガードを追加しており、他ストリームのイベントは `_ => {}` で捨てられる
- そのため、ピアが並列に 2 本目のリクエストストリームを開いた場合、そのストリームは復旧できない
- 0159 の設計方針で「並列リクエスト対応は別 issue で扱う」と明記された
- クライアント側 (`H3Client::send_request`) も `&mut self` により構造上 1 本しか送信できないが、サーバー側と同じ排他ドレイン構造を持つ

## 設計方針

- イベントキューを per-stream に切り分ける、または接続レベルの受信タスクを別途 spawn して各ストリームに mpsc で配送する構造にする
- WebTransport 側 (`crates/tokio-s2n-quic/src/webtransport/session.rs`) の `WtSession::recv_event` / `event_rx: mpsc::Receiver<WebTransportEvent>` の設計を参考にする
- `H3ServerConnection::accept_request` の戻り値 `H3Request` に per-stream の `event_rx` を持たせ、`H3Request::read_body()` / `H3Request::headers()` などが自身のストリームのイベントだけを消費できるようにする
- クライアント側も同様に `H3ClientResponse` に per-stream event_rx を持たせる (別 issue でも可)
- 実装は大掛かりになるため、段階的にサーバー側から先行するのが望ましい

## 完了条件

- ピアが並列に複数のリクエストストリームを開いても、`H3ServerConnection::accept_request` を並列に呼び出せるようになる
- サーバー側の `H3ServerConnection::accept_request` の doc から「1 接続 1 リクエスト逐次処理」の制約が削除される
- 統合テストを追加する (複数リクエストを並列に送信し、レスポンスが正しく届くことを検証)
- `cargo test --workspace --tests` / `cargo fmt --all -- --check` / `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `crates/tokio-s2n-quic/src/h3/server.rs` (`H3ServerConnection::accept_request` の全面再構成、`H3Request` の設計変更)
- `crates/tokio-s2n-quic/src/h3/client.rs` (別 issue でも可)
- 参考: `crates/tokio-s2n-quic/src/webtransport/session.rs` (`WtSession::recv_event` / `event_rx`)

### 一次資料

- `refs/h3/rfc9114.txt` Section 4 (HTTP/3 リクエスト / レスポンス多重化)
