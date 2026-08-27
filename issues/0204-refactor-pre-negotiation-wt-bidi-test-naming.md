# `test_pre_negotiation_wt_bidi_*` テスト名の `_stream_` プレフィックス統一

- Created: 2026-08-28
- Completed: {YYYY-MM-DD}
- Branch: feature/refactor-pre-negotiation-wt-bidi-test-naming

## 目的

`src/connection/mod.rs` の 0178 で追加された `test_pre_negotiation_wt_bidi_*` テストで、`_stream_` プレフィックスの有無が不統一な状態を統一する。

## 現状

- 0178 で追加された 13 件のテストのうち、命名パターンが 2 系統に分かれている
- `_stream_` あり (多数派)
  - `test_pre_negotiation_wt_bidi_stream_dispatched_after_settings`
  - `test_pre_negotiation_wt_bidi_stream_dropped_when_wt_disabled_settings`
  - `test_pre_negotiation_wt_bidi_stream_is_buffered_not_error`
  - `test_pre_negotiation_wt_bidi_stream_exceeds_stream_limit`
  - `test_pre_negotiation_wt_bidi_stream_exceeds_data_limit`
  - `test_pre_negotiation_wt_bidi_stream_multi_chunk_dispatched_after_settings`
  - `test_pre_negotiation_wt_bidi_stream_fin_dispatched_after_settings`
  - `test_pre_negotiation_wt_bidi_stream_reset_cleans_pending`
  - `test_pre_negotiation_wt_bidi_stream_post_settings_wt_disabled_rejected_immediately`
  - `test_pre_negotiation_wt_bidi_stop_sending_absorbed_silently` (これは `_stop_sending_` を含むため独自命名)
- `_stream_` なし (少数派)
  - `test_pre_negotiation_wt_bidi_rejected_subsequent_chunks_are_ignored`
  - `test_pre_negotiation_wt_bidi_dropped_settings_subsequent_chunks_ignored`
  - `test_pre_negotiation_bidi_dispatch_cleared_on_stream_reset` (これは `_dispatch_` を含むため独自命名)

## 設計方針

- 多数派の `_stream_` を挿入する形に統一する
- 少数派 2 件を以下に改名する
  - `test_pre_negotiation_wt_bidi_rejected_subsequent_chunks_are_ignored` → `test_pre_negotiation_wt_bidi_stream_rejected_subsequent_chunks_are_ignored`
  - `test_pre_negotiation_wt_bidi_dropped_settings_subsequent_chunks_ignored` → `test_pre_negotiation_wt_bidi_stream_dropped_settings_subsequent_chunks_ignored`
- `test_pre_negotiation_wt_bidi_stop_sending_absorbed_silently` と `test_pre_negotiation_bidi_dispatch_cleared_on_stream_reset` は対象が異なる (前者は STOP_SENDING、後者は `pending_bidi_dispatch`) ため命名パターンから外れているのは妥当。改名対象外

## 完了条件

- 2 件のテスト名が `_stream_` を含む形に統一される
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/connection/mod.rs` (テスト関数の rename)

### 関連 issue

- 0178 (本 issue の起源。テスト命名の一貫性)
