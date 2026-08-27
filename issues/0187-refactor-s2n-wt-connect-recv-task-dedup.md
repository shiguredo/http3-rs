# tokio-s2n-quic の CONNECT ストリーム受信タスク実装を client / server で共通化する

- Created: 2026-08-27
- Completed: {YYYY-MM-DD}
- Branch: feature/refactor-s2n-wt-connect-recv-task-dedup
- Polished: {YYYY-MM-DD}

## 目的

`tokio-s2n-quic` の CONNECT ストリーム受信タスクとその関連ヘルパーで client 側と server 側にほぼ完全同一のコードが 2 系統存在する状態を解消し、片方だけの更新漏れによるバグを構造的に防ぐ。

## 現状

- `crates/tokio-s2n-quic/src/webtransport/client.rs::run_client_connect_recv_task` (約 90 行) と `crates/tokio-s2n-quic/src/webtransport/server.rs::run_server_connect_recv_task` (約 90 行) は接続状態型 (`ClientConnectionState` / `ServerConnectionState`) の違い以外は byte-for-byte で同一
- `crates/tokio-s2n-quic/src/internal/connection_state.rs::ClientConnectionState::connect_stream_reset` と `ServerConnectionState::connect_stream_reset` も完全同一
- 前 issue 0156 のレビューで「片方だけの更新漏れによる regression リスク」が繰り返し指摘された

## 設計方針

- `shiguredo-rust` の「トレイトを作らないこと」規約に反しない範囲で共通化する
- 選択肢:
  - (a) `internal` に private trait `H3ConnectionOps` を定義し `process_stream_data` / `connect_stream_reset` / `drain_events` を trait method として抽出、`run_connect_recv_task<S: H3ConnectionOps>` に統合する (規約は「どうしても必要な場合は許可を得ること」の余地あり)
  - (b) `enum ConnStateRef { Client(Arc<Mutex<ClientConnectionState>>), Server(Arc<Mutex<ServerConnectionState>>) }` を用意し、内部で match して共通ロジックを呼ぶ
  - (c) `macro_rules!` で受信タスク関数を生成する (`shiguredo-rust` の「マクロを作らないこと」規約に反する)
- 選択方針は実装着手時に決めるが、まず (b) の enum ラッパー方式を優先的に検討する (規約に最も抵触しないため)
- `connect_stream_reset` も同様に共通化する

## 完了条件

- `crates/tokio-s2n-quic/src/webtransport/client.rs` と `server.rs` の受信タスク実装が 1 本に統合されている、または各々が薄いエントリポイント (数行) となっている
- `internal/connection_state.rs` の `connect_stream_reset` が Client / Server で 1 本に統合されている
- 既存の統合テスト (`crates/tokio-s2n-quic/tests/webtransport_session_close_e2e.rs`) が pass する
- `cargo test --workspace --tests` / `cargo fmt --all -- --check` / `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `crates/tokio-s2n-quic/src/webtransport/client.rs` (`run_client_connect_recv_task`)
- `crates/tokio-s2n-quic/src/webtransport/server.rs` (`run_server_connect_recv_task`)
- `crates/tokio-s2n-quic/src/internal/connection_state.rs` (`ClientConnectionState::connect_stream_reset` / `ServerConnectionState::connect_stream_reset`)
- `crates/tokio-s2n-quic/src/webtransport/session.rs` (共通ヘルパー配置場所)
