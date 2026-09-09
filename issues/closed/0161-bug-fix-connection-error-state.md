# Connection のエラー状態 (self.error) が本番経路で設定されない

- Created: 2026-08-08
- Completed: 2026-08-23
- Branch: feature/fix-connection-error-state
- Polished: {YYYY-MM-DD}

## 目的

接続エラー後の `Connection` の状態管理を実装し、エラー後の継続呼び出しによるデータ処理・イベント生成を防ぐ。

## 現状

- `src/connection/mod.rs` の `Connection.error` フィールドは本番コードでは一度も設定されない (代入は inline テストのみ)
- `Connection::feed_stream` のエラー後ガード (エラー設定済みなら全入力を拒否する設計) は実質無効で、接続エラーを返した後の `feed_stream` / `send_*` / `poll_event` はデータを処理し続けイベントを積み続ける
- `init_h3_streams` の doc は「呼び出し側がエラー後の Connection を破棄する」前提を書いており、実装と設計が乖離している
- エラー後の接続に対する `stream_reset` / `stop_sending` / `send_request` / `send_response` / `send_body` / `send_goaway` も通常どおり処理される

## 設計方針

- 接続エラーを返す経路で `self.error` を設定する
- エラー設定後の API 呼び出しを拒否する (または動作を明確に定義して doc 化する)

## 完了条件

- 接続エラー後の `feed_stream` / 送信 API が拒否される (または定義された動作になる)
- テストが追加される
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/connection/mod.rs` (`Connection.error` / `feed_stream` / `send_*` / `poll_event`)

### 修正内容

- `Connection` に、結果が `Error::ConnectionError` であれば `self.error` に記録し、以後の呼び出しを同じエラーで拒否するヘルパー (`check_error_state` / `remember_connection_error`) を追加した (RFC 9114 Section 8.1: 接続エラー後の接続は回復不能)
- `feed_stream` / `poll_event` / `drain_events` / `send_body` / `send_goaway` / `stream_reset` / `stop_sending` の各公開 API で、接続エラーを返した経路で `self.error` に記録し、エラー設定後は `check_error_state` で拒否する
- `send_request` / `send_response` は内部関数 (`*_inner`) に分離し、公開側で `remember_connection_error` を通すようにした
- ストリーム単位のエラー (`StreamError` / `StreamNotFound` 等) は接続エラーではないため記録しない
- inline テストに `test_connection_error_state_is_recorded_on_feed_stream` (接続エラーが記録され各 API が拒否される) と `test_stream_error_does_not_set_connection_error_state` (ストリームエラーは記録されない) を追加した
