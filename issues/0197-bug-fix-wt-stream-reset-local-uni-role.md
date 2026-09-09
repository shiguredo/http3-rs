# `handle_wt_stream_reset` がローカル開始 uni の RESET_STREAM でクレジット回復・登録除去してしまう

- Created: 2026-08-27
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-wt-stream-reset-local-uni-role
- Polished: 2026-09-09

## 目的

`Connection::handle_wt_stream_reset` がローカル開始 WebTransport uni ストリームに対する RESET_STREAM を bidi と同様に処理し、ピアのクレジット (WT_MAX_STREAMS) を不正に回復し、`wt_uni_streams` の登録を除去してしまう問題を修正する。QUIC 層をバイパスして RESET_STREAM が sans-I/O に渡った場合の防御を追加する。

## 現状

- `src/connection/wt_session.rs` の `handle_wt_stream_reset` は `let local_initiated = self.is_local_initiated_bidi(kind);` のみを使い、`is_local_initiated_uni(kind)` を考慮しないため、ローカル開始 uni ストリームは常に `local_initiated=false` と判定される (この判定は 0145 由来で、0170 のローカル uni 登録により顕在化した)
- 同じ `handle_wt_stream_reset` から先に呼ばれる `account_wt_stream_reset` も `header_len` 判定に `is_local_initiated_bidi(kind)` しか使わないため、ローカル開始 uni でピアが送っていないヘッダー長を `final_size` から減算する
- 仮に QUIC 層をバイパスして RESET_STREAM が sans-I/O に渡ると:
  - `on_remote_stream_closed(is_bidi=false)` が呼ばれ、ピアが開いていない uni のクレジット (WT_MAX_STREAMS) を不正に回復する
  - `wt_uni_streams.remove` で登録が消え、以後の STOP_SENDING が汎用 `Event::StopSending` にフォールスルーする (0170 修正の趣旨を裏返す)
- 通常経路では RFC 9000 Section 19.4 により QUIC 層で STREAM_STATE_ERROR となるため実行時には発生しないが、統合層のバグや将来 QUIC 実装の変更で発生し得る

## 設計方針

- `handle_wt_stream_reset` で、`wt_uni_streams` に登録済みのローカル開始 uni (`is_local_initiated_uni(kind)` が true) の RESET_STREAM は RFC 9000 Section 19.4 の STREAM_STATE_ERROR に相当する不正入力として防御的に無視する。`session_id` 解決後に `true` を返して早期 return し、次を行わない:
  - `on_remote_stream_closed` による WT_MAX_STREAMS クレジットの回復 (根拠: draft-16 Section 5.3 は「ピアが開始するストリーム」の数の制限)
  - `account_wt_stream_reset` のデータ FC ヘッダー減算
  - `wt_uni_streams` の登録除去 (0170 の STOP_SENDING → `WebTransportEvent::StreamStopSending` 通知を維持する)
  - `WebTransportEvent::StreamReset` の発火
- 早期 return により `handle_wt_stream_reset` 内の `local_initiated` 判定 (bidi 用) にはローカル開始 uni が到達しなくなる。bidi の判定は現状のままでよい
- WT 以外のローカル開始 uni (H3 の push 等) は従来どおり `session_id` 解決前に `false` を返して汎用 `Event::StreamReset` を発火する (本 issue のスコープ外)
- テストを追加する: ローカル開始 uni に対する `stream_reset` 呼び出しでクレジット回復・ヘッダー減算・登録除去・`StreamReset` 発火が起きないこと

## 完了条件

- `handle_wt_stream_reset` がローカル開始 uni の RESET_STREAM を無視し、クレジット回復・ヘッダー減算・登録除去・`StreamReset` 発火を行わない
- テストが追加される:
  - ローカル開始 uni を登録 → `stream_reset` → `MaxStreams` カプセルが生成されない
  - ローカル開始 uni を登録 → `stream_reset` → `wt_uni_streams` に残り、その後の `stop_sending` が `WebTransportEvent::StreamStopSending` として通知される
  - ローカル開始 uni を登録 → `stream_reset` → `WebTransportEvent::StreamReset` が発火しない
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/connection/wt_session.rs` (`Connection::handle_wt_stream_reset` / `Connection::account_wt_stream_reset`)
- `src/connection/wt_stream.rs` (`is_local_initiated_bidi` / `is_local_initiated_uni` / `register_local_wt_stream`)
- `src/connection/mod.rs` (テスト追加)

### 一次資料

- `refs/quic/rfc9000.txt` Section 2.1 (ストリーム種別) / Section 3.5 / Section 19.4 (RESET_STREAM)
- `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 5.3 (WT_MAX_STREAMS / クレジット)

### 関連 issue

- 0170 (ローカル開始 WT uni ストリームの登録 API 拡張。本 issue はそれにより顕在化した既存バグの修正)
