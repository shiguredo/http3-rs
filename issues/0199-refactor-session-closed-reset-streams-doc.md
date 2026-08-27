# `WebTransportEvent::SessionClosed.reset_streams` の docstring にストリーム方向別の指示を明示する

- Created: 2026-08-27
- Completed: {YYYY-MM-DD}
- Branch: feature/refactor-session-closed-reset-streams-doc
- Polished: {YYYY-MM-DD}

## 目的

`WebTransportEvent::SessionClosed.reset_streams` の docstring に、ストリーム方向（bidi / ローカル開始 uni / ピア開始 uni）ごとに送出すべきフレーム種別を明示する。

## 現状

- `src/event.rs` の `WebTransportEvent::SessionClosed.reset_streams` の docstring は次のように記述している:
  - 「`reset_streams` に含まれる全ストリームに対して `error_code` を使用して `RESET_STREAM_AT` (reliable_size を伴う) と STOP_SENDING を送信すること」
- しかし、送出すべきフレームはストリーム方向で異なる:
  - **双方向 (bidi)**: 両方向あるので `RESET_STREAM_AT` (送信方向) と `STOP_SENDING` (受信方向) を送出する
  - **ローカル開始 uni** (送信専用): `RESET_STREAM_AT` (送信方向) のみ送出する。ピア側は receive-only なので `STOP_SENDING` を送るとピア側で RFC 9000 Section 19.5 の STREAM_STATE_ERROR になる
  - **ピア開始 uni** (受信専用): `STOP_SENDING` (受信方向) のみ送出する。ローカル側は send-only ではないため `RESET_STREAM_AT` は送れない
- 0170 でローカル開始 uni が `reset_streams` に含まれるようになった結果、統合層が docstring 通りに STOP_SENDING を送出するとピア側で接続エラーを引き起こす

## 設計方針

- `WebTransportEvent::SessionClosed.reset_streams` の docstring を書き換え、ストリーム方向別の指示を明示する:
  - bidi: RESET_STREAM_AT + STOP_SENDING
  - ローカル開始 uni: RESET_STREAM_AT のみ
  - ピア開始 uni: STOP_SENDING のみ
- ストリーム方向の判定は `stream_id` の下位 2 ビットとロールから統合層が判定する (既存の `StreamKind::from_stream_id` / `is_local_initiated` 相当のロジックを統合層側で使う想定)
- 判定ヘルパーを sans-I/O 層で提供するかは実装時に判断する

## 完了条件

- `src/event.rs` の `WebTransportEvent::SessionClosed.reset_streams` の docstring がストリーム方向別の指示を明示する
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る (docstring 変更のみ、実装ロジックは変えない)

## 解決方法

### 関連ファイル

- `src/event.rs` (`WebTransportEvent::SessionClosed` の docstring)

### 一次資料

- `refs/quic/rfc9000.txt` Section 19.4 (RESET_STREAM) / Section 19.5 (STOP_SENDING) / Section 19.8 (STREAM)
- `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 4.4 / Section 6

### 関連 issue

- 0170 (ローカル開始 WT uni ストリームの登録 API 拡張。本 issue は同変更で顕在化した docstring の不備を修正)
