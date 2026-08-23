# SETTINGS_WT_ENABLED > 1 の検証 (draft-16 Section 3.1) にテストがない

- Created: 2026-08-08
- Completed: 2026-08-23
- Branch: feature/test-add-settings-wt-enabled-validation
- Polished: {YYYY-MM-DD}

## 目的

draft-ietf-webtrans-http3-16 追従で追加された「クライアントが `SETTINGS_WT_ENABLED > 1` を受信したら `H3_SETTINGS_ERROR` で接続を閉じる」検証のテストを追加する。

## 現状

- 実装は `src/connection/mod.rs` の SETTINGS 処理に存在する (draft-16 Section 3.1 の MUST)
- `Setting::from_wire` (`src/settings.rs`) は `SETTINGS_WT_ENABLED` に bool 検査をかけず値をそのまま通すため、`wt_enabled = 2` の SETTINGS はデコードに成功し検証経路に到達可能
- 全テストスイート (`tests/` / `pbt/` / inline / crates/) で `wt_enabled > 1` を構築・注入したテストが 1 件もない
- `ErrorCode::SettingsError` の Connection レベルでの発生経路としても唯一の場所であり、エラーコード網羅の観点でも欠落している

## 設計方針

- クライアント設定に `wt_enabled(VarInt::from_static(2))` を持つサーバー制御ストリームを feed して `Error::ConnectionError(ErrorCode::SettingsError)` を assert するテストを追加する

## 完了条件

- `SETTINGS_WT_ENABLED > 1` を受信したクライアントが `H3_SETTINGS_ERROR` になるテストが追加される
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/connection/mod.rs` (SETTINGS 処理 / `ErrorCode::SettingsError`)
- `src/webtransport/settings.rs` (`Settings::wt_enabled`)
- 一次資料: `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 3.1

### 修正内容

- `src/connection/mod.rs` の inline テストモジュールに `test_wt_enabled_greater_than_one_returns_settings_error` を追加した。`wt_enabled(VarInt::from_static(2))` を設定持つサーバーの制御ストリームを `feed_stream` し、`Error::ConnectionError(ErrorCode::SettingsError)` を検証する
