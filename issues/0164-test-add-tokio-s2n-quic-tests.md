# tokio-s2n-quic クレートにユニットテストがない

- Created: 2026-08-08
- Completed: 2026-08-23
- Branch: feature/test-add-tokio-s2n-quic-tests
- Polished: {YYYY-MM-DD}

## 目的

tokio-s2n-quic クレートのロジック (config / h3 / webtransport / internal) を検証するユニットテストを追加する。

## 現状

- `crates/tokio-s2n-quic/` には `tests/` ディレクトリがなく、`src/` 内の `#[cfg(test)]` / `#[test]` も 0 件
- `Cargo.toml` には `dev-dependencies` として `rcgen = "0.14"` と `tokio` (full + test-util) が定義済みだが、これらを使うテストファイルが存在しない (テストを書く前提の依存だけが残骸として残っている)
- `internal/connection_state.rs` (Sans I/O 本体との状態管理) を含む全ロジックが無検証
- CI (`cargo test --workspace`) ではテスト 0 件でも成功扱いになる。間接的に interop テスト (macOS のみ) で実行されるだけ
- 同クレートは `WtClient::connect` の CONNECT 検証欠如や `WtSession::close` のカプセル形式誤り等、仕様未達のバグを複数抱えており、テスト不在が検出を妨げている

## 設計方針

- `internal/connection_state.rs` の状態管理 (SETTINGS / QPACK ストリーム初期化、イベントドレイン) のユニットテストを追加する
- `webtransport/session.rs` の `WtSession` (ストリームヘッダー検証、セッションクローズ) のユニットテストを追加する
- 可能なら h3 のリクエスト / レスポンス往復の統合テストを追加する

## 完了条件

- tokio-s2n-quic にユニットテストが追加される
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `crates/tokio-s2n-quic/src/internal/connection_state.rs`
- `crates/tokio-s2n-quic/src/webtransport/session.rs` (`WtSession`)
- `crates/tokio-s2n-quic/src/h3/client.rs` / `h3/server.rs`

### 修正内容

- `crates/tokio-s2n-quic/src/internal/connection_state.rs` に `#[cfg(test)]` モジュールを追加し、8 件のユニットテストを実装した
  - `test_client_init_h3_streams` / `test_server_init_h3_streams`: 制御ストリーム・QPACK ストリーム初期化データ (ストリームタイプ) の検証
  - `test_drain_qpack_data_after_init_is_empty`: 初期データが `init_h3_streams` で取り切られ、ドレインに残らないこと
  - `test_client_settings_received`: ピアの SETTINGS 受信で `SettingsReceived` イベントが生成されること
  - `test_feed_stream_only_does_not_generate_events`: QPACK ストリームが `feed_stream_only` で処理され、イベントが生成されないこと
  - `test_client_send_request_generates_qpack_data`: リクエスト送信と FIN 交付の検証
  - `test_server_prepare_response_generates_stream_data`: クライアントからのリクエスト feed → レスポンス準備 → データ + FIN 交付の E2E 検証
  - `test_process_stream_data_error_is_forwarded`: 制御ストリーム上の DATA フレームが接続エラーとして透過されること
- `WtSession` のユニットテストは QUIC 実接続 (BidirectionalStreamAcceptor / SendStream) が必要なため、本 issue では `connection_state.rs` のテストのみを追加した (WtSession の検証は 0156 / 0158 の統合テストで行う)
