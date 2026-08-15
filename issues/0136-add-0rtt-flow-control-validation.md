# 0-RTT 再開時のフロー制御値減少を検出して H3_SETTINGS_ERROR で接続を閉じる

- Created: 2026-07-31
- Completed: {YYYY-MM-DD}
- Branch: feature/add-0rtt-flow-control-validation
- Polished: 2026-08-15

## 目的

draft-ietf-webtrans-http3-16 Section 3.2 の 0-RTT 再開時クライアント検証要件に対応する。0135 から分割。

## 現状

draft-16 追加要件:

> A client MUST close the connection with H3_SETTINGS_ERROR if the SETTINGS frame received in the resumed connection reduces any flow control values from the cached previous values.

現在、0-RTT 再開時のフロー制御値比較ロジックは存在しない。

## 設計方針

- 比較対象は `wt_initial_max_streams_uni`、`wt_initial_max_streams_bidi`、`wt_initial_max_data` の 3 フィールド (draft-16 Section 5.5 が SETTINGS 経由の初期フロー制御値として定義する 3 項目と一致)
- 本検証はクライアントのみが行う (draft-16 Section 3.2 の要件はクライアント向け)
- Sans I/O ライブラリは接続をまたいだ状態を持てないため、ピア (サーバー) の前回 SETTINGS は呼び出し側が保持し、0-RTT 再開が確定した接続でのみライブラリへ注入する。0-RTT が拒否された接続は resumed connection ではないため比較の対象外 (draft-16 Section 3.2 の MUST は resumed connection 限定)
- 前回値が注入されている場合、SETTINGS フレーム受信時に今回の値と比較し、いずれかのフィールドが前回値より減少していれば `H3_SETTINGS_ERROR` で接続を閉じる。同値は違反ではない (draft-16 Section 3.2 の reduces は厳密減少)
- 今回の SETTINGS でフィールドが省略された場合は draft-16 Section 5.5 のデフォルト値 0 として扱う。前回値が 0 より大きい状態で省略された場合は減少として検出する
- 注入は 0-RTT 受諾が判明した時点で行う (0-RTT 拒否時は注入しない。既存の `set_webtransport_transport_verified` のようなセッター方式を想定)
- エラーの返し方は `src/connection/mod.rs` の `process_control_stream` にある既存の `SETTINGS_WT_ENABLED > 1` 検証と同様に `Err(Error::ConnectionError(ErrorCode::SettingsError))` とする
- 注入 API の具体的なシグネチャは実装時に決定する

## 完了条件

- 前回セッションの SETTINGS を注入して比較できる API が提供される
- クライアントが 0-RTT 再開時にフロー制御値の減少を検出して `H3_SETTINGS_ERROR` で接続を閉じる
- テストが追加される
- CHANGES.md の `## develop` セクションに変更履歴が記載される
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/connection/mod.rs` (SETTINGS 処理経路)
- `src/connection/client.rs` (ClientConnection)
- `src/webtransport/settings.rs` (Settings 型)
- 一次資料: `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 3.2
