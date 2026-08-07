# ローカル側の CONNECT ストリーム FIN で WT セッションが終了しない

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-wt-connect-fin-session-state
- Polished: {YYYY-MM-DD}

## 目的

draft-ietf-webtrans-http3-16 Section 6 の「CONNECT ストリームのクローズ (どちら側でも) = セッション終了」がローカル側 FIN で実装されていない問題を修正する。

## 現状

- `src/connection/mod.rs` の `Connection::send_body` で CONNECT ストリームに `fin=true` を渡しても、送信バッファに FIN を立てるだけで `WtSession` の状態は Established のまま
- `SessionClosed` イベントが発火せず、`send_datagram` も通る
- 受信側 FIN (`src/connection/wt_capsule.rs` の CONNECT ストリーム終了処理) のみ終了処理がある
- draft-16 Section 6「A WebTransport session ... is terminated when ... the CONNECT stream is closed, either cleanly or abruptly, on either side」

## 設計方針

- CONNECT ストリームの送信側 FIN 設定時にセッション終了処理 (Draining / Closed 遷移、`SessionClosed` イベント、関連ストリームの RESET) を実行する

## 完了条件

- ローカル側が CONNECT ストリームに FIN を送ったらセッションが終了し `SessionClosed` イベントが発火する
- テストが追加される
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/connection/mod.rs` (`Connection::send_body` / `Connection::send_request` の fin 処理)
- `src/connection/wt_session.rs` (セッション終了処理)
- 一次資料: `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 6
