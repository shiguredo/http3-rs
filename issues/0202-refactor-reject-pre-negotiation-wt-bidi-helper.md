# `reject_pre_negotiation_wt_bidi` ヘルパを切り出す

- Created: 2026-08-28
- Completed: {YYYY-MM-DD}
- Branch: feature/refactor-reject-pre-negotiation-wt-bidi-helper

## 目的

`ignored_pre_negotiation_wt_bidi` への挿入と `WebTransportEvent::BufferedStreamRejected` の発火が 4 箇所で個別に手書きされている状態を、専用ヘルパ関数に集約する。

## 現状

- 0178 の実装で、以下の 4 箇所が「pending から remove → ignored に insert → 必要なら `BufferedStreamRejected` イベント発火」の組み合わせを個別に書いている
  - `src/connection/mod.rs` の `buffer_pre_negotiation_wt_bidi` (ストリーム数上限超過)
  - `src/connection/mod.rs` の `buffer_pre_negotiation_wt_bidi` (データ量上限超過)
  - `src/connection/mod.rs` の `dispatch_client_bidi_stream` (post-SETTINGS + WT 非対応時の即拒否)
  - `src/connection/mod.rs` の `process_pending_wt_bidi_pre_negotiation` (WT 非対応 SETTINGS 受信時の破棄)
  - `src/connection/wt_session.rs` の `handle_wt_stream_reset` (RESET while pending。イベント発火なし)
- RESET パスはイベント発火なし、他はあり、と微妙にずれている
- 将来「拒否時の共通処理」を追加する際 (例: 統合層向けの追加通知、統計値のインクリメント) に、5 箇所を横断修正することになる

## 設計方針

- `Connection::reject_pre_negotiation_wt_bidi(&mut self, stream_id: u64, emit_event: bool)` のようなヘルパを追加する
- 実装は以下を行う:
  - `self.pending_wt_bidi_pre_negotiation.remove(&stream_id)` を呼ぶ
  - `self.ignored_pre_negotiation_wt_bidi.insert(stream_id)` を呼ぶ
  - `emit_event == true` なら `WebTransportEvent::BufferedStreamRejected` を発火する
- 4 箇所を本ヘルパの呼び出しに置換する

## 完了条件

- ヘルパが導入され、4 箇所が呼び出しに置換される
- 既存テストが全て通ることを確認する
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/connection/mod.rs` (ヘルパ追加、4 箇所の置換)
- `src/connection/wt_session.rs` (`handle_wt_stream_reset` の置換)

### 関連 issue

- 0178 (本 issue の起源。ヘルパ集約による可読性向上)
