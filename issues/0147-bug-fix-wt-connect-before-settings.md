# サーバーがクライアント SETTINGS 受信前の WT CONNECT リクエストを即拒否する

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-wt-connect-before-settings
- Polished: {YYYY-MM-DD}

## 目的

draft-ietf-webtrans-http3-16 が想定する「SETTINGS より先に CONNECT が届き得る」順序入れ替えで正当なクライアントのセッションが失敗する問題を修正する。

## 現状

- `src/connection/wt_session.rs` の `Connection::validate_wt_connect_request_server` は `peer_settings` が None の時点で `Error::StreamError(ErrorCode::MessageError)` を返す
- draft-16 Section 3.1「Servers should note that CONNECT requests to establish new WebTransport sessions, in addition to other messages, can arrive before the client's SETTINGS are received (see Section 4.6)」。同一フライトで SETTINGS と CONNECT を並送したクライアント (到着順序は保証されない) は必ず失敗する
- draft-16 Section 7.1「the server MUST NOT process any incoming WebTransport requests until the client's SETTINGS have been received」の正しい満たし方は「処理しない」ことであって「拒否」ではない

## 設計方針

- `validate_wt_connect_request_server` で `peer_settings` が None の場合はリクエストを保留 (バッファリング) し、SETTINGS 受信後に再検証して受理または拒否する
- 保留中の上限 (pending 上限 16 と同様の枠) を設ける

## 完了条件

- SETTINGS より先に WT CONNECT が届いても、SETTINGS 受信後にセッションが確立される
- 保留の上限を超えた場合は拒否される
- テストが追加される
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/connection/wt_session.rs` (`Connection::validate_wt_connect_request_server`)
- 一次資料: `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 3.1 / 4.6 / 7.1
