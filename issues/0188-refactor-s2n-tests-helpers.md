# tokio-s2n-quic の e2e テストヘルパーを tests/helpers/ に共通化する

- Created: 2026-08-27
- Completed: {YYYY-MM-DD}
- Branch: feature/refactor-s2n-tests-helpers
- Polished: {YYYY-MM-DD}

## 目的

`crates/tokio-s2n-quic/tests/` 配下の e2e テストで重複しているヘルパー関数群 (`build_wt_settings` / `start_server` / `build_client_config`) と、h3 e2e テストで重複している生 s2n-quic クライアントの接続 + SETTINGS 送信コードを `tests/helpers/` に切り出し、`shiguredo-rust` 規約 (「テスト間で共有するヘルパーは `tests/helpers/` に置くこと」) に準拠させる。

## 現状

- `generate_certificate()` は `crates/tokio-s2n-quic/tests/helpers/certs.rs` に共通化済みで、各 e2e テストは `#[path = "helpers/certs.rs"]` で取り込んでいる
- `build_wt_settings` / `start_server` / `build_client_config` は `webtransport_session_close_e2e.rs` と `webtransport_connect_validation_e2e.rs` に完全同一実装が重複したまま残っている。`start_server` は `webtransport_post_close_reset_e2e.rs` と `h3_critical_stream_reset_e2e.rs` にも個別定義がある
- h3 e2e テスト (`h3_stream_reset_e2e.rs` / `h3_critical_stream_reset_e2e.rs` / `h3_webtransport_bidi_rejected_e2e.rs`) に、生の s2n-quic クライアントの構築と接続、制御ストリームへの SETTINGS フレーム送信のコードが重複している
- `shiguredo-rust` 規約は「テスト間で共有するヘルパーは `tests/helpers/` に置くこと」と定めるが、テストを追加するたびに重複が拡大している

## 設計方針

- 既存の `tests/helpers/` と同じ方式 (`#[path = "helpers/<name>.rs"]` で必要なファイルだけを取り込む。`mod.rs` は使わない) で共通化する
- `build_wt_settings` / `start_server` / `build_client_config` を WT サーバー用ヘルパーとして 1 ファイルに移動し `pub` 化する
- h3 e2e の生 QUIC クライアント接続 + SETTINGS 送信を h3 用ヘルパー (例: `tests/helpers/h3_raw_client.rs`) に切り出し、3 ファイルから利用する
- 既存の重複実装は削除する
- テストの検証内容・期待値は変えず、既存テストが引き続き pass することを確認する

## 完了条件

- `webtransport_session_close_e2e.rs` / `webtransport_connect_validation_e2e.rs` / `webtransport_post_close_reset_e2e.rs` / `h3_critical_stream_reset_e2e.rs` の重複ヘルパー (`build_wt_settings` / `start_server` / `build_client_config`) が `tests/helpers/` に移動し、各テストは `#[path = ...]` 経由で利用する
- h3 e2e の 3 ファイル (`h3_stream_reset_e2e.rs` / `h3_critical_stream_reset_e2e.rs` / `h3_webtransport_bidi_rejected_e2e.rs`) の生 QUIC 接続 + SETTINGS 送信コードが `tests/helpers/` に移動し、各テストは `#[path = ...]` 経由で利用する
- 既存テストがすべて pass する
- `cargo test --workspace --tests` / `cargo fmt --all -- --check` / `cargo clippy --workspace --all-targets -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `crates/tokio-s2n-quic/tests/helpers/certs.rs` (既存。`generate_certificate` は共通化済み)
- `crates/tokio-s2n-quic/tests/helpers/` (WT サーバー用 / h3 生クライアント用ヘルパーを追加)
- `crates/tokio-s2n-quic/tests/webtransport_session_close_e2e.rs` (重複削除)
- `crates/tokio-s2n-quic/tests/webtransport_connect_validation_e2e.rs` (重複削除)
- `crates/tokio-s2n-quic/tests/webtransport_post_close_reset_e2e.rs` (重複削除)
- `crates/tokio-s2n-quic/tests/h3_critical_stream_reset_e2e.rs` (重複削除)
- `crates/tokio-s2n-quic/tests/h3_stream_reset_e2e.rs` (重複削除)
- `crates/tokio-s2n-quic/tests/h3_webtransport_bidi_rejected_e2e.rs` (重複削除)
