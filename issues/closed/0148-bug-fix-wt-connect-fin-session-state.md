# ローカル側の CONNECT ストリーム FIN で WT セッションが終了しない

- Created: 2026-08-08
- Completed: 2026-08-16
- Branch: feature/fix-wt-connect-fin-session-state
- Polished: 2026-08-15

## 目的

draft-ietf-webtrans-http3-16 Section 6 の「CONNECT ストリームのクローズ (どちら側でも) = セッション終了」がローカル側 FIN で実装されていない問題を修正する。

## 現状

- `src/connection/mod.rs` の `Connection::send_body` で CONNECT ストリームに `fin=true` を渡しても、送信バッファに FIN を立てるだけで `WtSession` の状態は Established のまま
- `SessionClosed` イベントが発火せず、`send_datagram` も通る
- 受信側 FIN (`src/connection/wt_capsule.rs` の `handle_wt_stream_end` による CONNECT ストリーム終了処理) のみ終了処理がある
- draft-16 Section 6「A WebTransport session ... is terminated when ... the CONNECT stream is closed, either cleanly or abruptly, on either side」

## 設計方針

- 対象は WebTransport セッションの CONNECT ストリーム (plain CONNECT は対象外) への送信 FIN のみ
- 終了処理のフックは FIN 交付時 (`get_stream_data` で交付される時点。`take_stream_data` は内部で `get_stream_data` を呼ぶため `get_stream_data` 単独で双方をカバーできる) に行う。FIN 設定時 (`send_body` 呼び出し時) にセッションを終了すると、`remove_stream_if_done` が `closed_wt_sessions` 判定でストリームを除去し、未交付の FIN が失われる
- 終了処理は既存の `terminate_wt_session` を使用する (WT_SESSION_GONE、close_error_code=0。clean FIN は error code 0 の WT_CLOSE_SESSION と等価。draft-16 Section 6)。`WtSession` を Closed へ遷移させ、`SessionClosed` イベントを発火し、関連ストリームを RESET する (Draining 遷移は行わない)
- 対象経路は `send_body` / `send_response` を問わず、CONNECT ストリームの送信 FIN 全般とする。終了対象は `wt_sessions` に存在する CONNECT ストリーム (Pending 含む) で、サーバーが非 2xx + fin=true で拒否した場合も Pending セッションが終了し `SessionClosed` が発火する
- セッション終了後は `send_datagram` が拒否される

## 完了条件

- ローカル側が CONNECT ストリームに FIN を送ったらセッションが終了し `SessionClosed` イベントが発火する
- CONNECT ストリームの FIN が送信バッファから失われず交付される
- セッション終了後に `send_datagram` が拒否される
- テストが追加される
- CHANGES.md の `## develop` セクションに変更履歴が記載される
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/connection/mod.rs` (`get_stream_data` での FIN 交付時フック。`send_request` は CONNECT の fin=true を既に拒否しているため対象外)
- `src/connection/wt_session.rs` (セッション終了処理 `terminate_wt_session`)
- 一次資料: `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 6
