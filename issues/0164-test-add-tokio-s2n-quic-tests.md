# tokio-s2n-quic クレートにユニットテストがない

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
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
