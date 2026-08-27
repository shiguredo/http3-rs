# tokio-s2n-quic の e2e テストヘルパーを tests/helpers/ に共通化する

- Created: 2026-08-27
- Completed: {YYYY-MM-DD}
- Branch: feature/refactor-s2n-tests-helpers
- Polished: {YYYY-MM-DD}

## 目的

`crates/tokio-s2n-quic/tests/` 配下の e2e テストで重複しているヘルパー関数群 (`generate_certificate` / `build_wt_settings` / `start_server` / `build_client_config`) を `tests/helpers/` に切り出し、`shiguredo-rust` 規約 (「テスト間で共有するヘルパーは `tests/helpers/` に置くこと」) に準拠させる。

## 現状

- `crates/tokio-s2n-quic/tests/webtransport_session_close_e2e.rs` (0156 で追加) の 19-56 行
- `crates/tokio-s2n-quic/tests/webtransport_connect_validation_e2e.rs` (0157 で追加) の 22-56 行

上記 2 ファイルに以下 4 関数の完全同一実装が重複している:

- `generate_certificate() -> (String, String)`: rcgen で自己署名証明書とキーを生成する
- `build_wt_settings() -> webtransport::Settings`: テスト用 WebTransport SETTINGS (draft-15) を返す
- `start_server() -> (WtServer, SocketAddr, String)`: ポート 0 でサーバーを起動しリッスンアドレスと証明書 PEM を返す
- `build_client_config(server_addr, ca_cert_pem) -> ClientConfig`: サーバー証明書を CA として渡すクライアント設定を構築する

`shiguredo-rust` 規約は「テスト間で共有するヘルパーは `tests/helpers/` に置くこと」と定めるが、2 ファイル目 (0157) の時点で共通化されておらず、規約違反状態が発生している。同 crate に新規 e2e テストを追加するたびに重複が拡大する。

## 設計方針

- `crates/tokio-s2n-quic/tests/helpers/` ディレクトリを作成し、`mod.rs` は使わず `tests/helpers.rs` を新規追加する (`shiguredo-rust`「`mod.rs` を使わないこと」に準拠)
- 4 関数を `helpers` モジュールに移動し `pub` 化する
- 各 e2e テストファイルの先頭に `mod helpers;` を宣言してインポートする
- 既存の重複実装は削除する
- 新規 e2e テストは追加せず、既存の 4 テスト (`webtransport_session_close_e2e.rs` 3 件 + `webtransport_connect_validation_e2e.rs` 1 件) が引き続き pass することを確認する

## 完了条件

- `crates/tokio-s2n-quic/tests/helpers.rs` が新規追加され、4 関数の共通実装を持つ
- `webtransport_session_close_e2e.rs` と `webtransport_connect_validation_e2e.rs` から重複ヘルパーが削除され、`mod helpers;` 経由で利用する
- 4 テスト (`server_close_delivers_session_closed_to_client` / `client_close_delivers_session_closed_to_server` / `client_drop_delivers_clean_close_to_server` / `server_reject_causes_client_error`) が pass する
- `cargo test --workspace --tests` / `cargo fmt --all -- --check` / `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `crates/tokio-s2n-quic/tests/helpers.rs` (新規)
- `crates/tokio-s2n-quic/tests/webtransport_session_close_e2e.rs` (重複削除 + `mod helpers;`)
- `crates/tokio-s2n-quic/tests/webtransport_connect_validation_e2e.rs` (重複削除 + `mod helpers;`)
