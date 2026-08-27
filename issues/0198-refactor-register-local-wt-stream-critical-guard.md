# `register_local_wt_stream` に critical stream との衝突検出を追加する

- Created: 2026-08-27
- Completed: {YYYY-MM-DD}
- Branch: feature/refactor-register-local-wt-stream-critical-guard
- Polished: {YYYY-MM-DD}

## 目的

`Connection::register_local_wt_stream` に critical stream (`control_send` / QPACK encoder stream / QPACK decoder stream) との衝突検出を追加し、防御性を高める。

## 現状

- 0170 で `Connection::register_local_wt_stream` が bidi / uni 両対応に拡張された
- 現状の重複チェックは `wt_bidi_streams` / `wt_uni_streams` / `streams` のみを対象とし、以下の critical stream ID との衝突を検出しない:
  - `control_send.stream_id()` (ローカル制御ストリーム)
  - `encoder_stream_id` (ローカル QPACK エンコーダーストリーム)
  - `decoder_stream_id` (ローカル QPACK デコーダーストリーム)
- 通常のアプリケーションでは critical stream ID と WebTransport ストリーム ID は衝突しないが、テストや誤用時に不正な状態が生じる可能性がある
- `stop_sending` は `is_critical` チェック (`mod.rs:2109-2114`) で先に弾かれるため、runtime 上の実害は現状はないが、API の一貫性・防御性として検出が望ましい

## 設計方針

- `register_local_wt_stream` の重複チェックに `control_send.stream_id()` / `encoder_stream_id` / `decoder_stream_id` との照合を追加する
- 衝突時は `Error::ConnectionError(ErrorCode::StreamCreationError)` を返す
- テストを追加する (control / QPACK encoder / QPACK decoder それぞれとの衝突ケース)

## 完了条件

- `register_local_wt_stream` が critical stream ID との衝突を検出して `StreamCreationError` を返す
- テストが追加される
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/connection/wt_stream.rs` (`Connection::register_local_wt_stream` の critical チェック追加)
- `src/connection/mod.rs` (テスト追加)

### 関連 issue

- 0170 (ローカル開始 WT uni ストリームの登録 API 拡張。本 issue はその防御性向上)
