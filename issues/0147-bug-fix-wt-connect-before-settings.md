# サーバーがクライアント SETTINGS 受信前の WT CONNECT リクエストを即拒否する

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-wt-connect-before-settings
- Polished: 2026-08-15

## 目的

draft-ietf-webtrans-http3-16 が想定する「SETTINGS より先に CONNECT が届き得る」順序入れ替えで正当なクライアントのセッションが失敗する問題を修正する。

## 現状

- `src/connection/wt_session.rs` の `Connection::validate_wt_connect_request_server` は `peer_settings` が None の時点で `Error::StreamError(ErrorCode::MessageError)` を返す
- draft-16 Section 3.1「Servers should note that CONNECT requests to establish new WebTransport sessions, in addition to other messages, can arrive before the client's SETTINGS are received (see Section 4.6)」。同一フライトで SETTINGS と CONNECT を並送したクライアント (到着順序は保証されない) は、CONNECT が先に届いた場合に失敗する
- draft-16 Section 7.1「the server MUST NOT process any incoming WebTransport requests until the client's SETTINGS have been received」の正しい満たし方は「処理しない」ことであって「拒否」ではない

## 設計方針

- `peer_settings` が None かつ WebTransport CONNECT の場合、`src/connection/mod.rs` の `emit_header_events` は検証・セッション登録・イベント発行を行わず、CONNECT リクエストヘッダーを `Connection` に新設する保留バッファへ格納する (非 WT CONNECT は従来どおり即時処理。一般ヘッダー検証 `validate_headers` は保留時にも実施する)
- SETTINGS 受信時 (`process_control_stream`) に保留した CONNECT リクエストを `validate_wt_connect_request_server` で再検証し、受理なら登録・イベント発行、拒否なら現行どおり拒否する
- 保留期間中に同じ CONNECT ストリームへ届いた DATA は `Event::Data` として配送せず、保留中の CONNECT と併せて保持する。SETTINGS 受信後にセッションが登録されたら、保持した DATA は Pending セッションへの受信として処理する (0137 の楽観的カプセルバッファリングと連携。0137 実装前は保持 DATA が現行の Pending 挙動に当たる点に留意する)
- 保留数には既存の `WT_MAX_PENDING_SESSIONS` (16) とは別枠の上限を設け、超過時は `H3_MESSAGE_ERROR` で拒否する。保留上限は `WT_MAX_PENDING_SESSIONS` 以下にする (登録後に Pending セッション上限を超えないように)。保持する DATA にもバイト数の上限を設け、超過時は `H3_MESSAGE_ERROR` でストリームをリセットする (0137 と同様の DoS 対策)
- 保留期間中に同じ CONNECT ストリームの FIN / RESET_STREAM / STOP_SENDING を受信した場合は保留エントリを破棄する
- 既存テスト `test_server_wt_connect_rejected_without_peer_settings` は挙動変更に合わせて更新する

## 完了条件

- SETTINGS より先に WT CONNECT が届いても、SETTINGS 受信後にセッションが確立される
- 保留の上限を超えた場合は拒否される
- テストが追加される
- CHANGES.md の `## develop` セクションに変更履歴が記載される
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/connection/wt_session.rs` (`Connection::validate_wt_connect_request_server`)
- `src/connection/mod.rs` (`emit_header_events` / `process_control_stream`)
- `src/connection/wt_capsule.rs` (`handle_wt_data_frame` の DATA 保留)
- `src/connection/wt_types.rs` (保留バッファの定義)
- 一次資料: `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 3.1 / 4.6 / 7.1
