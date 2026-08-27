# Role::Client 側での 0x41 bidi 受信時の挙動固定テストを追加する

- Created: 2026-08-28
- Completed: {YYYY-MM-DD}
- Branch: feature/add-client-role-wt-bidi-signal-behavior-test

## 目的

Client 側 (`role == Role::Client`) で server-initiated bidi ストリームとして 0x41 (WT_STREAM signal value) を受信した場合の挙動を固定するテストを追加し、0178 で `feed_stream` から `is_wt_fully_negotiated()` ゲートを除去したことによるリグレッションを防ぐ。

## 現状

- `src/connection/mod.rs` の `Connection::feed_stream` は、Server 側限定分岐 (`self.role == Role::Server`) の中で `dispatch_client_bidi_stream` を呼ぶ
- Client 側では server-initiated bidi ストリームは既存経路 (`handle_bidirectional_stream` 等) に落ちる。SETTINGS 未受信時は `Error::ConnectionError(ErrorCode::StreamCreationError)` になる既存挙動があるはず
- 0178 で追加された `pending_wt_bidi_pre_negotiation` と `ignored_pre_negotiation_wt_bidi` は Client 側では絶対に使われない
- 将来、`Role::Server` 判定を削除するようなリファクタが発生した場合、Client 側でも保留マップに入り込んでリグレッションになる可能性がある。それを検出するテストがない

## 設計方針

- Client 側の Connection を構築し、SETTINGS 未受信の状態で server-initiated bidi (`stream_id % 4 == 0x01`) に 0x41 を feed するテストを追加する
- 期待挙動:
  - `pending_wt_bidi_pre_negotiation` にエントリが入らない
  - `ignored_pre_negotiation_wt_bidi` にエントリが入らない
  - 既存の Client 側挙動 (接続エラー or 保留) が維持される
- 既存挙動の詳細を実装時に確認し、テストで固定する

## 完了条件

- Client 側での 0x41 bidi 受信時の挙動を固定するテストが追加される
- テストが通る (既存挙動を変えない)
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/connection/mod.rs` (`#[cfg(test)]` モジュールにテスト追加)

### 関連 issue

- 0178 (本 issue の起源。ゲート除去のリグレッション防止)
