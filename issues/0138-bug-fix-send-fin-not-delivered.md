# send_request / send_body の FIN が QUIC 層に届かない

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-send-fin-not-delivered
- Polished: {YYYY-MM-DD}

## 目的

`send_request` / `send_body` に `fin=true` を渡しても FIN が QUIC 層へ渡らず、ピアがリクエスト / レスポンスの終端を検知できずハングする問題を修正する。

## 現状

- `src/stream/request.rs` の `RequestStream::get_send_data` は `fin` を `SendBuffer::is_fin() && !SendBuffer::has_pending()` で返す
- `src/stream/mod.rs` の `SendBuffer::has_pending` は `(fin && !fin_sent)` を含むため、データ消費後は `(fin && !fin_sent)` が恒真となり、`get_send_data` の fin は**恒 false** になる
- `RequestStream::mark_fin_sent` はワークスペース全体で呼び出し 0 件（`fin_sent` が立てられる経路がない）
- 結果: `Connection::get_stream_data` / `Connection::take_stream_data` は fin=false のまま返し、`send_request(headers, true)` / `send_body(data, true)` の FIN が失われる。`Connection::writable_streams` は FIN のみのストリームを報告し続け、イベントループが busy loop に陥る
- 統合層 (tokio-s2n-quic / examples/wt_server) は QUIC 側の `finish()` で代替しているため、テストでも検出されていない

## 設計方針

- `consume_stream_data` / `take_stream_data` 経路で「データ消費完了 + fin 設定時」に fin=true を返し、そのタイミングで `mark_fin_sent` を呼ぶ
- 制御ストリーム・QPACK ストリームの FIN 扱いと混同しない（これらはクライアント開始 bidi のリクエストストリームの話）

## 完了条件

- `send_request(headers, true)` / `send_body(data, true)` の FIN が `take_stream_data` の戻り値で QUIC 層に届く
- FIN 送達後に `writable_streams` にそのストリームが残らない
- テストが追加される
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/stream/request.rs` (`get_send_data` / `mark_fin_sent` / `has_pending_send`)
- `src/stream/mod.rs` (`SendBuffer::has_pending` / `SendBuffer::mark_fin_sent`)
- `src/connection/mod.rs` (`get_stream_data` / `take_stream_data` / `consume_stream_data` / `writable_streams` / `send_request` / `send_body`)
- 一次資料: `refs/h3/rfc9114.txt` Section 4.1 (メッセージ終端の意味論)
