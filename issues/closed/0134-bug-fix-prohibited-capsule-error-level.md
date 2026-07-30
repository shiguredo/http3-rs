# 禁止 Capsule 受信時にセッションエラーを返すように修正する

- Created: 2026-07-31
- Completed: 2026-07-31
- Branch: feature/fix-prohibited-capsule-error-level
- Polished: 2026-07-31

## 目的

`WT_MAX_STREAM_DATA` / `WT_STREAM_DATA_BLOCKED` (HTTP/3 上で禁止された Capsule) を受信した際、WebTransport セッションエラーとして処理するようにする。draft-ietf-webtrans-http3-15 Section 5.4 (draft-16 でも同一) は "session error" を MUST で要求している。

## 現状

禁止 Capsule の処理が 2 層で仕様に違反している。

**Connection 層** (`src/connection/wt_capsule.rs` の `Connection::handle_wt_capsule`): 禁止 Capsule は `Capsule::Unknown` としてデコードされ、黙って無視される。`is_prohibited_in_http3` は `src/connection/` 内で一切呼ばれていない。仕様の MUST ("session error") に違反して何もしない。

**Session 層** (`src/webtransport/session/mod.rs` の `Session::process_capsule`): 禁止 Capsule 受信時に `CapsuleProcessError::Connection(0x105)` (H3_FRAME_UNEXPECTED) を返す。仕様の "session error" に対して接続レベルのエラーを返しており、エラーレベルが誤り。

draft-ietf-webtrans-http3-15 Section 5.4 (`refs/webtrans/draft-ietf-webtrans-http3-15.txt`):

> Endpoints MUST treat receipt of a WT_MAX_STREAM_DATA or a WT_STREAM_DATA_BLOCKED capsule as a session error.

0114 で Session 層のエラーコードを `FlowControlError` から `H3_FRAME_UNEXPECTED` に修正したが、エラーレベル (Connection vs Session) と Connection 層の無視は未修正のまま。

## 設計方針

- **Connection 層**: `Connection::handle_wt_capsule` の `Capsule::Unknown` 分岐で `is_prohibited_in_http3` を呼び出し、禁止 Capsule を検出した場合はセッション終了イベントを生成する
- **Session 層**: `Session::process_capsule` の `CapsuleProcessError::Connection(0x105)` を `CapsuleProcessError::Session(...)` に変更する
- **エラーコードの選択**: 仕様は具体的なエラーコードを規定していない。禁止 Capsule はフロー制御違反ではなく HTTP/3 に存在しない Capsule 種の受信であるため、`ErrorCode::FlowControlError` は意味的に不適合。アプリケーションエラーコード `Error::application(0, "prohibited capsule received")` でセッションを閉じる
- `process_capsule` 内のインラインコメントを session error に書き換える。`CapsuleProcessError::Connection` の doc コメントにある「など」を削除し、使用例が WT_MAX_STREAMS > 2^60 のみであることを明示する
- コメントに draft-15 Section 5.4 の引用を残す

## 完了条件

- Connection 層で禁止 Capsule が検出され、セッション終了として処理される
- `Session::process_capsule` が禁止 Capsule 受信時に `CapsuleProcessError::Session` を返す
- インラインテスト `test_session_process_prohibited_capsule_returns_error` の期待値が更新される
- 禁止 Capsule 2 種 (`WT_MAX_STREAM_DATA` = 0x190B4D3E、`WT_STREAM_DATA_BLOCKED` = 0x190B4D42) の両方に対するテストが存在する
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

1. `src/connection/wt_capsule.rs` の `Connection::handle_wt_capsule` の `Capsule::Unknown` 分岐に `is_prohibited_in_http3` チェックを追加し、禁止 Capsule 検出時に `terminate_wt_session_with` でセッションを終了するようにした
2. `src/webtransport/session/mod.rs` の `Session::process_capsule` 内の禁止 Capsule 分岐を `CapsuleProcessError::Session(Error::application(0, "prohibited capsule received"))` に変更した
3. `CapsuleProcessError::Connection` の doc コメントから「など」を削除し、使用例が WT_MAX_STREAMS > 2^60 のみであることを明示した
4. 既存テスト `test_session_process_prohibited_capsule_returns_error` の期待値を更新し、`WT_STREAM_DATA_BLOCKED` (0x190B4D42) のテスト `test_session_process_prohibited_capsule_stream_data_blocked` を追加した

### 関連ファイル

- 修正対象: `src/connection/wt_capsule.rs` の `Connection::handle_wt_capsule` (テストは同ファイル内の `#[cfg(test)]` モジュールに追加)
- 修正対象: `src/webtransport/session/mod.rs` の `Session::process_capsule`
- テスト: `src/webtransport/session/mod.rs` 内の `test_session_process_prohibited_capsule_returns_error`
- 一次資料: `refs/webtrans/draft-ietf-webtrans-http3-15.txt` Section 5.4、`refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 5.4
