# `handle_wt_stream_reset` がローカル開始 uni について `local_initiated=false` と誤判定する

- Created: 2026-08-27
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-wt-stream-reset-local-uni-role
- Polished: {YYYY-MM-DD}

## 目的

`Connection::handle_wt_stream_reset` がローカル開始 WebTransport uni ストリームを bidi 判定関数 `is_local_initiated_bidi` のみで扱い、常に `local_initiated=false` と判定する既存バグを修正する。QUIC 層をバイパスして RESET_STREAM が sans-I/O に渡った場合の防御を追加する。

## 現状

- `src/connection/wt_session.rs` の `handle_wt_stream_reset` (0170 修正後) は `is_local_initiated_bidi(kind)` のみを判定に使う
- 0170 で追加した `is_local_initiated_uni(kind)` を考慮していないため、ローカル開始 uni ストリームは常に `local_initiated=false` と判定される
- 仮に QUIC 層をバイパスして RESET_STREAM が sans-I/O に渡ると:
  - `on_remote_stream_closed(is_bidi=false)` が呼ばれ、ピアが開いていない uni のクレジット (WT_MAX_STREAMS) を不正に回復する
  - `wt_uni_streams.remove` で登録が消え、以後の STOP_SENDING が汎用 `Event::StopSending` にフォールスルーする (0170 修正の趣旨を裏返す)
- 通常経路では RFC 9000 Section 19.4 により QUIC 層で STREAM_STATE_ERROR となるため実行時には発生しないが、統合層のバグや将来 QUIC 実装の変更で発生し得る

## 設計方針

- `handle_wt_stream_reset` の `local_initiated` 判定を `is_local_initiated_bidi(kind) || is_local_initiated_uni(kind)` に拡張する
- ローカル開始 uni の RESET_STREAM 受信は draft-16 Section 6 相当のクレジット非回復ケースとして扱う (`on_remote_stream_closed` を呼ばない)
- テストを追加する: ローカル開始 uni に対する `stream_reset` 呼び出しでピアのクレジット回復が発生しないこと

## 完了条件

- `handle_wt_stream_reset` がローカル開始 uni ストリームに対してクレジット回復を行わない
- テストが追加される (ローカル開始 uni の登録 → 疑似 stream_reset → WT_MAX_STREAMS 変化なし / SessionClosed 経路)
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/connection/wt_session.rs` (`Connection::handle_wt_stream_reset`)
- `src/connection/wt_stream.rs` (`is_local_initiated_bidi` / `is_local_initiated_uni`)
- `src/connection/mod.rs` (テスト追加)

### 一次資料

- `refs/quic/rfc9000.txt` Section 3.5 / Section 19.4 (RESET_STREAM)
- `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 5.3 (WT_MAX_STREAMS / クレジット) / Section 6

### 関連 issue

- 0170 (ローカル開始 WT uni ストリームの登録 API 拡張。本 issue はその副作用として存在する既存バグの修正)
