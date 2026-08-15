# GOAWAY 送信後に新規リクエスト・WT CONNECT が拒否されず処理され続ける

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-goaway-new-request-rejection
- Polished: 2026-08-15

## 目的

GOAWAY 送信後の新規リクエスト拒否 (RFC 9114 Section 5.2 の SHOULD) を経路間で統一する。

## 現状

- WT データストリーム経路 (`src/connection/wt_session.rs` の `associate_or_buffer_stream`) では `last_sent_goaway_id` 境界以上で拒否するが、新規リクエスト経路では GOAWAY チェックがない
  - `src/connection/mod.rs` の `Connection::handle_bidirectional_stream` (新規リクエスト。新規 WT CONNECT もこの経路を通る)
  - `src/connection/wt_session.rs` の `Connection::validate_wt_connect_request_server` (新規 WT CONNECT の前提条件検証)
- RFC 9114 Section 5.2「Upon sending a GOAWAY frame, the endpoint SHOULD explicitly cancel any requests ... The endpoint SHOULD continue to do so as more requests or pushes arrive」
- 経路間で挙動が非対称で、GOAWAY 後の新規セッション確立が混在する

## 設計方針

- `handle_bidirectional_stream` の新規ストリーム処理 (ストリームが `self.streams` に未登録の初回処理) の冒頭に、`role == Role::Server` かつ `last_sent_goaway_id` があり `stream_id >= last_sent_goaway_id` の場合に `H3_REQUEST_REJECTED` (`Error::StreamError(ErrorCode::RequestRejected)`) で拒否するチェックを追加する
- 新規 WT CONNECT も `handle_bidirectional_stream` を経由するため、`validate_wt_connect_request_server` への個別追加は不要
- チェックは新規ストリームに限定する。GOAWAY 送信前に処理開始済み (ストリーム登録済み) のストリームには適用しない (RFC 9114 Section 4.1.1 の H3_REQUEST_REJECTED 使用制約に従う)
- ロールはサーバーのみ。クライアントの `last_sent_goaway_id` は push ID (実装上 0 のみ) のため、チェックをクライアントに適用すると全レスポンスを誤拒否する
- 境界は「以上」(`>=`) として扱う (RFC 9114 Section 5.2「with the indicated identifier or greater」、既存の `associate_or_buffer_stream` と同じ判定)

## 完了条件

- GOAWAY 送信後に境界以上の新規リクエスト / WT CONNECT が拒否される
- テストが追加される
- CHANGES.md の `## develop` セクションに変更履歴が記載される
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/connection/mod.rs` (`Connection::handle_bidirectional_stream`)
- 一次資料: `refs/h3/rfc9114.txt` Section 4.1.1 / 5.2
