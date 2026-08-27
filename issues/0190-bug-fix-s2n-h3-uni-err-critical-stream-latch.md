# tokio-s2n-quic の H3 uni タスクが `recv_stream.receive()` の Err 経路で `ClosedCriticalStream` を誤ラッチする

- Created: 2026-08-27
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-s2n-h3-uni-err-critical-stream-latch
- Polished: {YYYY-MM-DD}

## 目的

H3 uni タスクが `recv_stream.receive()` から `Err` を受け取った際に、トランスポート層の Err を sans-I/O 層への FIN として誤伝達し、`ClosedCriticalStream` を接続エラーとしてラッチする副作用を修正する。

## 現状

- `crates/tokio-s2n-quic/src/h3/client.rs` / `server.rs` の uni ストリーム受信タスクは `while let Ok(Some(data))` パターンで受信し、ループ抜け後に無条件で `feed_stream_only(stream_id, &[], true)` を呼び FIN を伝達する
- `recv_stream.receive()` が `Err(_)` (トランスポート層エラー、例: 接続タイムアウト、リセット等) を返した場合もこのループを抜け、Err と FIN の区別なく sans-I/O に FIN として通知する
- sans-I/O 層は QPACK 制御ストリーム等のクリティカルストリームで FIN を受け取ると `ClosedCriticalStream` を接続エラーとしてラッチし、次の `drain_events` で顕在化する
- 0159 の修正で `accept_request` / `send_request` の受信ループが冒頭で `drain_events` を回すようになったため、この副作用が **リクエストデータ到着前** に露出するようになり、interop テストで低確率の flake として現れる可能性がある

## 設計方針

- uni タスクの受信ループを `Ok(Some(data))` / `Ok(None)` / `Err(_)` の 3 パターンに分ける
- `Ok(None)` (クリーンな FIN 受信) では従来通り `feed_stream_only(_, &[], true)` を呼ぶ
- `Err(_)` (トランスポート層エラー) では FIN を伝達せず、エラーログのみ記録してタスク終了する
- WebTransport 側の uni タスク実装 (`crates/tokio-s2n-quic/src/webtransport/client.rs` の `route_uni_stream`) も同じパターンを持っており、そちらは既に `Err(_) => return` で扱っているので参考にする

## 完了条件

- `recv_stream.receive()` が `Err(_)` を返しても sans-I/O 層に FIN が伝達されず、`ClosedCriticalStream` がラッチされないこと
- interop_h3 の関連テスト (advanced 等) で flake が発生しないこと (50 回連続で pass)
- `cargo test --workspace --tests` / `cargo fmt --all -- --check` / `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `crates/tokio-s2n-quic/src/h3/client.rs` (uni ストリーム受信タスク)
- `crates/tokio-s2n-quic/src/h3/server.rs` (uni ストリーム受信タスク)
- 参考: `crates/tokio-s2n-quic/src/webtransport/client.rs` の `route_uni_stream` (`Err(_) => return`)

### 一次資料

- `refs/h3/rfc9114.txt` Section 6.2 (制御ストリーム / QPACK ストリームのクローズ禁止)
- `refs/h3/rfc9204.txt` Section 4.2 (QPACK ストリームのクローズ禁止)
