# tokio-s2n-quic の H3 uni ストリームタスクがグローバルイベントキューを破棄する

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-s2n-h3-event-drain
- Polished: {YYYY-MM-DD}

## 目的

QPACK エンコーダーストリーム処理で生成されるイベント (ブロック解除された応答ヘッダー等) が破棄され、レスポンスが欠落・ハングする問題を修正する。

## 現状

- `crates/tokio-s2n-quic/src/h3/client.rs` と `h3/server.rs` の uni ストリームタスクは `Connection::process_stream_data` (= feed + drain) を呼び、戻りイベントを `let _ = ...` で破棄する
- `Connection::drain_events` は接続全体のイベントキューを空にし、QPACK ブロック解除 (`retry_blocked_streams`) もキューへのイベント生成として行う
- ピアのエンコーダーストリームデータを uni タスクが処理した瞬間に、QPACK ブロック中だった応答 / リクエストヘッダーのイベントが生成され、そのまま破棄される。動的テーブル使用時 (デフォルト 4096 バイト) に応答欠落・ハングを引き起こす
- WT 側コード (`webtransport/client.rs` / `server.rs`) は `feed_stream_only` + notify 方式で同じ問題を回避しており、H3 側の 2 タスクだけが欠陥

## 設計方針

- uni ストリームタスクを `feed_stream_only` + notify 方式に変更し、イベントキューはメインループが drain する

## 完了条件

- 動的テーブル使用時にレスポンス / リクエストヘッダーが欠落しない
- テストが追加される
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `crates/tokio-s2n-quic/src/h3/client.rs` (uni ストリームタスク)
- `crates/tokio-s2n-quic/src/h3/server.rs` (uni ストリームタスク)
- 参考実装: `crates/tokio-s2n-quic/src/webtransport/client.rs` の `feed_stream_only` 方式
