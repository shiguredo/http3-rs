# Connection のストリーム / WT セッションが無制限に蓄積する

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-connection-resource-leak
- Polished: {YYYY-MM-DD}

## 目的

長時間接続のサーバーでメモリがリクエスト数・転送量・セッション数に比例して無制限に増加する問題を修正する。

## 現状

- `src/connection/mod.rs` の `Connection.streams` (`HashMap<u64, RequestStream>`) は `or_insert_with` で追加されるのみで、完走 (StreamEnd) ・リセット (stream_reset) ・STOP_SENDING のいずれの経路でもエントリが除去されない
- `src/stream/request.rs` の `RequestStream::receive` は受信 DATA を `RecvBuffer` に全量 `extend_from_slice` で保持するため、完走ストリームがボディ全体を保持し続ける
- `src/connection/wt_session.rs` の `Connection::terminate_wt_session_with` は状態を Closed にするだけで `Connection.wt_sessions` からエントリを除去しない。`associated_streams` / `capsule_buf` / バッファも保持し続ける

## 設計方針

- ストリーム終了 (StreamEnd / StreamReset / StopSending) 時に `streams` からエントリを除去する
- セッション終了処理の最後に `wt_sessions.remove` する（再 RESET / FIN による二重イベント防止の設計意図がある場合は、除去後の扱いを明示する）

## 完了条件

- ストリーム完走・リセット後に `Connection.streams` からエントリが除去される
- セッション終了後に `wt_sessions` からエントリが除去される
- 長時間接続でメモリ使用量が一定に保たれるテストが追加される
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/connection/mod.rs` (`Connection::streams` / `stream_reset` / `stop_sending` / StreamEnd 処理)
- `src/connection/wt_session.rs` (`terminate_wt_session_with`)
- `src/stream/request.rs` (`RequestStream::receive` / `RecvBuffer`)
