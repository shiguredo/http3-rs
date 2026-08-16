# バッファリング中 WT ストリームの stale エントリが残存し誤配送される

- Created: 2026-08-08
- Completed: 2026-08-16
- Branch: feature/fix-wt-buffered-stream-stale-entries
- Polished: 2026-08-15

## 目的

Pending セッションのバッファリング中に RESET_STREAM されたストリームのエントリが残り、セッション確立後に誤配送される問題を修正する。STOP_SENDING はピアが送信した受信データを無効化しないため対象外 (RFC 9000 Section 3.5)。

## 現状

- `src/connection/wt_session.rs` の `Connection::handle_wt_stream_reset` は `wt_uni_streams` / `wt_bidi_streams` のマッピングと `WtSession.associated_streams` のみを除去し、`WtSession.buffered_stream_entries` を掃除しないため、リセット済みストリームのエントリが残る (`handle_wt_stop_sending` はマッピングもエントリも除去しない)
- セッション確立時に `Connection::deliver_buffered_streams` がリセット済みストリームの Open / Data / End イベントを発火し、アプリは「StreamReset 通知後に Data が届く」という矛盾したイベント列を受ける
- `deliver_buffered_streams` はフロー制御違反で `break` した場合、`take_buffered_streams()` が vec を全取り出しするため未配送ストリームが喪失する (バッファに残骸が残り、終了時の RESET_STREAM も送られない)
- `Connection::terminate_wt_session_with` も `buffered_stream_ids` を `WtStreamReset` に追加するだけでマップから除去しないため、終了後も stale マッピングが残る

## 設計方針

- RESET_STREAM 受信時に `buffered_stream_entries` と `buffered_streams` の両方から該当エントリを除去する。STOP_SENDING はピアが送信した受信データを無効化しないため除去しない (RFC 9000 Section 3.5)。現行の `handle_wt_stop_sending` の挙動を維持する
- `deliver_buffered_streams` の中断時に未配送ストリーム (フロー制御違反したストリーム自身を含む) を終了時の RESET 対象として残す
- セッション終了時にバッファリング中ストリームのマッピングを除去する

## 完了条件

- リセットされたバッファリング中ストリームがセッション確立後に配送されない
- FC 違反で中断された残りストリーム (フロー制御違反したストリーム自身を含む) が喪失しない
- セッション終了時にバッファリング中ストリームの `wt_uni_streams` / `wt_bidi_streams` マッピングが除去される
- テストが追加される
- CHANGES.md の `## develop` セクションに変更履歴が記載される
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

- `WtSession` に `remove_buffered_stream()` と `restore_buffered_stream()` を追加する
- `handle_wt_stream_reset` でバッファリング中ストリームのエントリを全セッションから検索して除去する
- `deliver_buffered_streams` で FC 違反中断時に違反ストリームと後続の未配送ストリームを `restore_buffered_stream` でバッファに戻す
- `terminate_wt_session_with` で `buffered_stream_ids` のエントリを `buffered_stream_entries` から除去する

### 関連ファイル

- `src/connection/wt_session.rs` (`Connection::deliver_buffered_streams` / `handle_wt_stream_reset` / `terminate_wt_session_with`。`handle_wt_stop_sending` は変更しない)
- `src/connection/wt_types.rs` (`WtSession.buffered_streams` / `buffered_stream_entries` の除去処理)
- 一次資料: `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 4.4 / 4.6 / 6、`refs/quic/rfc9000.txt` Section 3.5
