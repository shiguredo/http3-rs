# リクエスト / レスポンスの FIN が QUIC 層に届かない

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-send-fin-not-delivered
- Polished: 2026-08-08

## 目的

`send_request` / `send_body` / `send_response` に `fin=true` を渡しても FIN が QUIC 層へ渡らず、ピアがリクエスト / レスポンスの終端を検知できずハングする問題を修正する。

RFC 9114 Section 4.1 はストリームの送信方向クローズによる終端表現を MUST で要求している:

> An HTTP request/response exchange fully consumes a client-initiated bidirectional QUIC stream. After sending a request, a client MUST close the stream for sending. Unless using the CONNECT method (see Section 4.4), clients MUST NOT make stream closure dependent on receiving a response to their request.

> After sending a final response, the server MUST close the stream for sending.

## 現状

- `src/stream/request.rs` の `RequestStream::get_send_data` は `fin` を `SendBuffer::is_fin() && !SendBuffer::has_pending()` で返す
- `src/stream/mod.rs` の `SendBuffer::has_pending` は `(fin && !fin_sent)` を含むため、データ消費後は `(fin && !fin_sent)` が恒 true となり、`get_send_data` の fin は**恒 false** になる
- `RequestStream::mark_fin_sent` はプロダクションコードで呼び出し 0 件（`fin_sent` が立てられる経路がない）
- 結果: `Connection::get_stream_data` / `Connection::take_stream_data` は fin=false のまま返し、`send_request(headers, true)` / `send_body(data, true)` / `send_response(headers, true)` の FIN が失われる。`Connection::writable_streams` は FIN のみのストリームを報告し続けるため、この API をループで使うイベントループは busy loop に陥る
- 統合層 (tokio-s2n-quic / examples/wt_server) は QUIC 側の `finish()` で代替しているため、テストでも検出されていない

## 設計方針

- `RequestStream::get_send_data` の fin 計算を「FIN 設定済み && 送信バッファのデータ全消費済み && 未交付 (`!fin_sent`)」に変更する。現行式 `is_fin() && !has_pending()` のままだと `mark_fin_sent` 後に恒 true になり、FIN が二重交付されるため
  - 「データ全消費済み」の判定は `has_pending()` を流用しない（`(fin && !fin_sent)` を含むため、`!has_pending()` との組合せでは恒 false になる）。`SendBuffer` にデータ専用の判定メソッド（`consumed >= data.len()` 相当）を追加して使用する
- `get_stream_data` / `take_stream_data` は「データ消費完了 + fin 設定時」に fin=true を返し、そのタイミングで `mark_fin_sent` を呼ぶ。`get_stream_data` が fin=true を返した時点で `mark_fin_sent` を呼ぶ形にし（`take_stream_data` は `get_stream_data` を内部で呼ぶため両経路で整合）、`get_stream_data` + `consume_stream_data` の 2 段階 API と `take_stream_data` で fin の意味論が食い違わないようにする
- FIN はデータ消費後の追加呼び出しで `(空, fin=true)` として交付される（データと同時に返るわけではない）
- FIN 交付後は `get_stream_data` / `take_stream_data` がデータ・FIN を返さないようにする（FIN は 1 回だけ交付される）。FIN は交付時点で確定し、QUIC への書き込み失敗時の再交付はない
- 制御ストリーム・QPACK ストリームの FIN 扱いと混同しない（FIN を扱う対象はクライアント開始 bidi のリクエストストリームのみ）
- 統合層 (tokio-s2n-quic / examples/wt_server) の `finish()` 代替は変更しない（本 issue のスコープは sans-I/O API の FIN 交付まで）。ピアからの RESET_STREAM / STOP_SENDING 受信後に `writable_streams` へ残り続ける問題は本 issue のスコープ外とする
- 0148 (ローカル側 CONNECT FIN でセッション終了) は本修正による FIN 交付を前提とする。実装順序は本 issue → 0148 を想定する
- 既存の `pbt/tests/prop_connection.rs` の「全データ消費後に `get_stream_data` が None」を固定するテストは、本修正後も成立することを確認する

## 完了条件

- `send_request(headers, true)` / `send_body(data, true)` / `send_response(headers, true)` の FIN が `take_stream_data` の戻り値で交付され、テストで検証される（FIN はデータ消費後の追加呼び出しで `(空, fin=true)` として交付される）
- FIN が 1 回だけ交付され、交付後に `get_stream_data` / `take_stream_data` が FIN を返さない
- FIN 送達後に `writable_streams` にそのストリームが残らない
- リクエスト / レスポンス両方向の FIN 送達と、FIN 送達後の `writable_streams` からの消失を検証するテストが追加される。`get_stream_data` + `consume_stream_data` の 2 段階 API 経路での FIN 交付と交付後の None も検証する
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/stream/request.rs` (`get_send_data` / `mark_fin_sent` / `has_pending_send`) と `get_send_data` の doc コメント
- `src/stream/mod.rs` (`SendBuffer::has_pending` / `SendBuffer::mark_fin_sent` / データ専用の全消費判定メソッド)
- `src/connection/mod.rs` (`get_stream_data` / `take_stream_data` / `consume_stream_data` / `writable_streams`) と各メソッドの doc コメント
- `src/connection/mod.rs` の `send_request` の doc コメントに残る「ストリームの FIN 送信完了を通知」（閉じた issue 0113 で削除された `mark_stream_fin_sent` の残骸）を掃除する
- `src/connection/mod.rs` / `src/connection/client.rs` / `src/connection/server.rs` の `take_stream_data` の doc コメント（「全データを 1 回の呼び出しで返す。ループで繰り返し呼ぶ必要はない」）を、FIN はデータ消費後の追加呼び出しで交付される仕様に合わせて更新する
- テスト: `tests/test_connection.rs` に FIN 送達・消失のテストを追加し、`src/stream/request.rs` の `#[cfg(test)]` モジュールに `get_send_data` の fin 交付の単体テストを追加する。`pbt/tests/prop_connection.rs` の既存テストが本修正後も成立することを確認する
- 一次資料: `refs/h3/rfc9114.txt` Section 4.1 (HTTP Message Framing)
