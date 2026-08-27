# `pending_bidi_dispatch` に上限チェックを導入する

- Created: 2026-08-28
- Completed: {YYYY-MM-DD}
- Branch: feature/refactor-pending-bidi-dispatch-limit

## 目的

`Connection::pending_bidi_dispatch: HashMap<u64, Vec<u8>>` に上限 (ストリーム数・1 ストリームあたりのバイト数) を導入し、DoS 経路を塞ぐ。

## 現状

- `src/connection/mod.rs` の `Connection::pending_bidi_dispatch` は client-initiated bidi ストリームの先頭 varint (signal value) が複数チャンクにまたがる場合の一時バッファ
- 上限チェックが一切なく、`dispatch_client_bidi_stream` 内で `entry(stream_id).or_default()` / `or_insert(buf)` によって無制限に成長しうる
- 0178 で `feed_stream` の `is_wt_fully_negotiated()` ゲートを除去したため、SETTINGS 未受信中の bidi ストリームも `dispatch_client_bidi_stream` を経由するようになり、ピアが先頭 1 バイトだけ流して RESET_STREAM を送るパターンで `pending_bidi_dispatch` を消費させる経路が拡大している
- 0178 の後続対策として `Connection::stream_reset` に `pending_bidi_dispatch.remove(&stream_id)` を追加してリークは防いだが、上限そのものはまだない

## 設計方針

- `pending_bidi_dispatch` のストリーム数上限を導入する。値は既存の `MAX_BUFFERED_STREAMS` (100) と揃えるか、bidi の signal value は 2 バイト varint なので更に小さい値でも可
- 1 ストリームあたりのバイト数上限は事実上不要 (signal value varint は最大 8 バイト、超過は接続エラー FRAME_ERROR で扱う既存経路がある)
- 上限超過時の挙動: `WT_BUFFERED_STREAM_REJECTED` または `H3_EXCESSIVE_LOAD` 相当のエラーコードでストリームリセットするか、汎用の接続エラーにするか実装時に判断する

## 完了条件

- `pending_bidi_dispatch` にストリーム数上限が導入される
- 上限到達時の挙動をテストで固定する
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/connection/mod.rs` (`Connection::dispatch_client_bidi_stream` / フィールド定義)

### 一次資料

- `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 4.6 (MUST limit)
- `refs/h3/rfc9114.txt` Section 5.2 / Section 11 (エラーコード)

### 関連 issue

- 0178 (本 issue の起源。ゲート除去で流入経路が拡大した)
