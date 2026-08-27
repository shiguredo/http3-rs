# tokio-s2n-quic の H3 uni ストリームタスクがグローバルイベントキューを破棄する

- Created: 2026-08-08
- Completed: 2026-08-27
- Branch: feature/fix-s2n-h3-event-drain
- Polished: 2026-08-26

## 目的

QPACK ブロック解除イベント (ブロック解除された応答ヘッダー等) が破棄され、レスポンスが欠落する問題を修正する。

## 現状

- `crates/tokio-s2n-quic/src/h3/client.rs` と `h3/server.rs` の uni ストリームタスクは `ClientConnectionState` / `ServerConnectionState` の `process_stream_data` (= feed + drain。`internal/connection_state.rs`) を呼び、戻りイベントを `let _ = ...` で破棄する
- sans-I/O 層の `Connection::drain_events` (src/connection/mod.rs) は接続全体のイベントキューを空にし、QPACK ブロック解除 (`retry_blocked_streams`) もキューへのイベント生成として行う。ブロック解除は `drain_events` / `poll_event` 経由でしか起きず、1 回きり (解除済みストリームは `blocked_by_ricnt` から除去される) ため、破棄されたイベントは再生成されない
- ピアのエンコーダーストリームデータを uni タスクが処理した瞬間に、QPACK ブロック中だった応答 / リクエストヘッダーがブロック解除され、ヘッダー・ボディ・StreamEnd のイベントが一括生成され、そのまま破棄される。動的テーブル使用時に応答欠落 (status 0・空ヘッダー・空ボディ) を引き起こす。Section Acknowledgment はイベント消費と独立にヘッダーデコード時に生成され送信されるため ack は届く。影響はヘッダー・ボディの欠落のみ
- 受信ループ (`send_request` / `accept_request`) は `recv_stream.receive()` しか待たず、ヘッダーがブロック中のまま `fin` で break するため、uni タスクの修正だけでは欠落が残る
- WT 側コード (`webtransport/client.rs` / `server.rs`) は `feed_stream_only` + notify + ループ先頭の `drain_events` 確認 + 10ms フォールバックで同じ問題を回避しており、H3 側だけがこの構造を持たない
- 自実装エンコーダーは動的テーブルへ挿入しない (`src/qpack/encoder.rs`) ため、s2n↔s2n のループバックではこのバグは発生せず、動的テーブル参照を行うピア (nghttp3 等) との接続でのみ発生する

## 設計方針

- uni ストリームタスクを `feed_stream_only` + notify 方式に変更し、イベントキューは受信ループが drain する (WT 側の参考実装と同じ構造。RFC 9204 Section 2.2.1: ブロック解除は Required Insert Count が Insert Count 以下になった時点)
- `send_request` / `accept_request` の受信ループに notify の `select!` 待ちとループ先頭での `drain_events` 確認を追加し、ヘッダーがブロック中の fin で break しないよう再構成する (WT 側と同じ notify 取りこぼし対策の 10ms フォールバックを含む)。fin 受信後にブロック解除を待つ場合、ピアがエンコーダーストリーム更新を送らないとハングし得る点に留意する (RFC 9204 Section 2.1.2: 挿入命令とヘッダーの到着順は保証されない。本バグの発生前提)
- uni タスクが行う `flush_qpack` (QPACK ストリームの送信) は維持する (WT 側の参考実装にはなく、そのまま写すと H3 側の QPACK 送信が失われる)

## 完了条件

- 動的テーブル使用時にレスポンス / リクエストヘッダーが欠落しない
- テストが追加される (nghttp3 ピアとの interop テスト。自実装エンコーダーは動的テーブルを使わないため s2n↔s2n では再現不能。ブロック解除は並行タスクの競合に依存するため、修正前コードでの欠落は毎回発生するとは限らない。複数リクエストの連続送信で、修正前コードでは欠落 (status 0 / 空ヘッダー / 空ボディ) が観察され、修正後コードでは常に完全なレスポンスが得られることを確認する。テストは 0165 (interop テストの空振り修正) の後に追加する)
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 変更内容

- `crates/tokio-s2n-quic/src/h3/client.rs` / `server.rs` の uni ストリーム受信タスクを `process_stream_data` (feed + drain) から `feed_stream_only` に切り替え、`Arc<Notify>` (`qpack_unblock_notify`) の `notify_one()` でリクエスト受信ループに通知する
- リクエスト受信ループ (`H3Client::send_request` / `H3ServerConnection::accept_request`) を `tokio::select!` に置き換え、以下の条件で待機・再確認する:
  - ループ先頭で `drain_events` を回し、QPACK ブロック解除で生成されたヘッダー・ボディ・StreamEnd (サーバーでは HeadersEnd も) をストリーム ID フィルタ付きで取り出す
  - `recv_stream.receive()` (guard `if !peer_fin`) / `qpack_unblock_notify.notified()` / 10ms フォールバックポーリングを `select!` で待つ
  - 受信ブランチも `feed_stream_only` で feed のみ実施し、`drain_events` は次周のループ先頭に一本化する (受信ブランチで `process_stream_data` の戻り値 `Vec<Event>` を破棄する構造を回避する)
- s2n-quic の `ReceiveStream::receive` は `DataRead` 状態で常に `Ok(None)` を返すため、FIN 受信後は `peer_fin` フラグを立てて `select!` の受信ブランチを停止し、busy loop を防ぐ
- `H3ServerConnection::accept_request` の doc に「1 接続 1 リクエスト逐次処理」の設計制約 (呼び出し側 / ピア側両方の前提) を明示する
- `Notify` フィールド名を `qpack_unblock_notify` に統一する
- `CHANGES.md` の `## develop` セクションに `[FIX]` エントリを追加する

### 対象外 (別 issue で対応)

- 動的テーブル参照を行うピア (nghttp3 等) との interop テストの追加は 0165 の後に対応する (issue で明記)
- `Event::StreamReset` / `Event::StopSending` の無視、リクエスト間 permit 持ち越し、`feed_stream_only` の Err を `let _` で捨てる問題への tracing ログ (0186 で対応)、uni ストリームタスクの client / server 重複 4 系統の共通化 (0187 で対応)、H3 サーバーの並列複数リクエスト対応、H3 サーバーの WT 有効化ケースの相互作用、uni タスクの `while let Ok(Some(data))` エラー経路が `ClosedCriticalStream` をラッチする副作用の修正、`send_request` にも同様の逐次処理 doc を追加、`drain_events` の `StreamError` に対する RESET 送出方針の整理、`peer_fin` かつ終了条件未達時の HTTP レベルタイムアウト機構は別 issue で対応する

### 一次資料

- `refs/h3/rfc9204.txt` Section 2.1.2 (Blocked Streams: 挿入命令とヘッダーの到着順は保証されない)
- `refs/h3/rfc9204.txt` Section 2.2.1 (Blocked Decoding: Required Insert Count が Insert Count 以下になった時点で解除)
