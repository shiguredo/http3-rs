# 0-RTT 再開時のフロー制御値減少を検出して H3_SETTINGS_ERROR で接続を閉じる

- Created: 2026-07-31
- Completed: {YYYY-MM-DD}
- Branch: feature/add-0rtt-flow-control-validation
- Polished: {YYYY-MM-DD}

## 目的

draft-ietf-webtrans-http3-16 Section 3.2 の 0-RTT 再開時クライアント検証要件に対応する。0135 から分割。

## 現状

draft-16 追加要件:

> A client MUST close the connection with H3_SETTINGS_ERROR if the SETTINGS frame received in the resumed connection reduces any flow control values from the cached previous values.

現在、0-RTT 再開時のフロー制御値比較ロジックは存在しない。

## 設計方針

- 比較対象: `wt_initial_max_streams_uni`、`wt_initial_max_streams_bidi`、`wt_initial_max_data` の 3 フィールド
- Sans I/O ライブラリとして、呼び出し側が前回の SETTINGS を保持して比較できる API を提供する
- API の具体的なシグネチャは実装時に決定する

## 完了条件

- クライアントが 0-RTT 再開時にフロー制御値の減少を検出して `H3_SETTINGS_ERROR` で接続を閉じる
- テストが追加される
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/connection/mod.rs` (SETTINGS 処理経路)
- `src/webtransport/settings.rs` (Settings 型)
- 一次資料: `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 3.2
