# バッファリング中 WT ストリームの stale エントリが残存し誤配送される

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-wt-buffered-stream-stale-entries
- Polished: {YYYY-MM-DD}

## 目的

Pending セッションのバッファリング中に RESET_STREAM / STOP_SENDING されたストリームのエントリが残り、セッション確立後に誤配送される問題を修正する。

## 現状

- `src/connection/wt_session.rs` の `Connection::handle_wt_stream_reset` / `handle_wt_stop_sending` は `wt_uni_streams` / `wt_bidi_streams` のマッピングと `WtSession.associated_streams` のみを除去し、`WtSession.buffered_stream_entries` を掃除しない
- セッション確立時に `Connection::deliver_buffered_streams` がリセット済みストリームの Open / Data / End イベントを発火し、アプリは「StreamReset 通知後に Data が届く」という矛盾したイベント列を受ける
- `deliver_buffered_streams` はフロー制御違反で `break` した場合、`take_buffered_streams()` が vec を全取り出しするため未配送ストリームが喪失する (バッファに残骸が残り、終了時の RESET_STREAM も送られない)
- `Connection::terminate_wt_session_with` も `buffered_stream_ids` を `WtStreamReset` に追加するだけでマップから除去しないため、終了後も stale マッピングが残る

## 設計方針

- RESET / STOP_SENDING 時に `buffered_stream_entries` から該当エントリを除去する
- `deliver_buffered_streams` の中断時に未配送ストリームを保持する (再配送 or 終了時の RESET 対象として残す)
- セッション終了時にバッファリング中ストリームのマッピングを除去する

## 完了条件

- リセットされたバッファリング中ストリームがセッション確立後に配送されない
- FC 違反で中断された残りストリームが喪失しない
- テストが追加される
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/connection/wt_session.rs` (`Connection::deliver_buffered_streams` / `handle_wt_stream_reset` / `handle_wt_stop_sending` / `terminate_wt_session_with`)
- 一次資料: `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 4.6
