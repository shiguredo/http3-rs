# GOAWAY 送信後に新規リクエスト・WT CONNECT が拒否されず処理され続ける

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-goaway-new-request-rejection
- Polished: {YYYY-MM-DD}

## 目的

GOAWAY 送信後の新規リクエスト拒否 (RFC 9114 Section 5.2 の SHOULD) を経路間で統一する。

## 現状

- WT データストリーム経路 (`src/connection/wt_session.rs` の `associate_or_buffer_stream` 相当) では `last_sent_goaway_id` 境界で拒否するが、以下の経路では GOAWAY チェックがない
  - `src/connection/mod.rs` の `Connection::handle_bidirectional_stream` (新規リクエスト)
  - `src/connection/mod.rs` の `Connection::dispatch_client_bidi_stream` (WT bidi)
  - `src/connection/wt_session.rs` の `Connection::validate_wt_connect_request_server` (新規 WT CONNECT)
- RFC 9114 Section 5.2「Upon sending a GOAWAY frame, the endpoint SHOULD explicitly cancel any requests ... The endpoint SHOULD continue to do so as more requests or pushes arrive」
- 経路間で挙動が非対称で、GOAWAY 後の新規セッション確立が混在する

## 設計方針

- 上記 3 経路に `last_sent_goaway_id` 境界チェックを追加し、境界を超える新規リクエスト / WT CONNECT を `H3_REQUEST_REJECTED` で拒否する

## 完了条件

- GOAWAY 送信後に境界以上の新規リクエスト / WT CONNECT が拒否される
- テストが追加される
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/connection/mod.rs` (`Connection::handle_bidirectional_stream` / `Connection::dispatch_client_bidi_stream`)
- `src/connection/wt_session.rs` (`Connection::validate_wt_connect_request_server`)
- 一次資料: `refs/h3/rfc9114.txt` Section 5.2
