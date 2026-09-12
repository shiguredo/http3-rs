# tokio-s2n-quic の接続単位タスクが接続終了まで残る構造を見直す

- Created: 2026-09-12
- Completed: {YYYY-MM-DD}
- Branch: feature/refactor-s2n-task-lifetime
- Polished: {YYYY-MM-DD}

## 目的

`WtSessionRequest` / `H3ServerConnection` 等が保持する制御ストリーム保持タスク (`std::future::pending::<()>()` 待機) の寿命を見直し、接続終了やセッション終了時にタスクが確実に終了する構造にする。

## 現状

- `crates/tokio-s2n-quic/src/webtransport/server.rs` の `WtSessionRequest::from_connection` は control / encoder / decoder ストリームを `control_task` で保持し、`std::future::pending::<()>()` で永久待機する
- 同様のタスクが `H3ServerConnection` / `H3Client` / `WtClient` にもある
- `JoinHandle` の drop は detach のため、セッションや接続が終了してもタスクは接続 (またはランタイム) が終わるまで残る
- 0206 のレビューで「新しい Err 経路でも同じ既存設計」として検出された (本差分由来ではない)

## 設計方針

- タスクの終了条件を明示する (接続ハンドルの drop、接続クローズ通知、チャネルクローズ等)
- `JoinHandle` を保持して明示的に abort するか、接続クローズを待つ future に置き換える
- 接続単位でタスクが残らないことをテストできる構造にする

## 完了条件

- 接続終了後に制御タスクが残らないことをテストで確認できる
- 既存の e2e テストが pass する
- `cargo test --workspace --tests` / `cargo fmt --all -- --check` / `cargo clippy --workspace --all-targets -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `crates/tokio-s2n-quic/src/webtransport/server.rs` / client.rs
- `crates/tokio-s2n-quic/src/h3/server.rs` / client.rs

### 一次資料

- `refs/quic/rfc9000.txt` (接続終了)
- `refs/h3/rfc9114.txt` Section 6.2.1 (クリティカルストリーム)

### 関連 issue

- 0206 (レビューで検出。本 issue はタスク寿命の見直し)
