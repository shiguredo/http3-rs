# ローカル開始の WT uni ストリームでピアの RESET_STREAM / STOP_SENDING / FIN が通知されない

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-local-initiated-wt-uni-stream-events
- Polished: {YYYY-MM-DD}

## 目的

アプリが自分で開いた WT uni ストリームに対するピアの RESET_STREAM / STOP_SENDING / FIN が WebTransport イベントとして通知されない問題を修正する。

## 現状

- `src/connection/wt_session.rs` の `Connection::handle_wt_stream_reset` / `handle_wt_stop_sending` は `wt_uni_streams` / `wt_bidi_streams` に登録されたストリームのみを処理し、未登録のストリーム (ローカル開始ストリーム) は `false` を返して汎用 `Event::StreamReset` にフォールスルーする
- `src/connection/wt_stream.rs` の `Connection::handle_wt_uni_stream_fin` も `wt_uni_streams` に登録されたストリームのみを処理する
- ローカル開始の WT uni ストリームを登録する API は 0144 (ローカル開始の WT ストリーム受信データがリクエストストリームとして誤処理される) で bidi のみを対象としており、uni はスコープ外とされた
- ローカル開始 uni ストリームには受信データは存在しないが (RFC 9000 Section 2.1)、ピアからの RESET_STREAM / STOP_SENDING / FIN は受信し得る

## 設計方針

- ローカル開始 WT uni ストリームの登録 API を追加し (0144 の bidi 登録 API と同様の形)、`handle_wt_stream_reset` / `handle_wt_stop_sending` / `handle_wt_uni_stream_fin` が登録済みのローカル開始 uni ストリームを処理できるようにする
- ピアの RESET_STREAM / STOP_SENDING / FIN を `StreamReset` / `UniStreamEnd` 等の WebTransport イベントとして通知する

## 完了条件

- ローカル開始 WT uni ストリームに対するピアの RESET_STREAM / STOP_SENDING / FIN が WebTransport イベントとして通知される
- テストが追加される
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/connection/wt_session.rs` (`Connection::handle_wt_stream_reset` / `handle_wt_stop_sending`)
- `src/connection/wt_stream.rs` (`Connection::handle_wt_uni_stream_fin` / 登録 API)
- `src/connection/mod.rs` (`wt_uni_streams` / 登録 API)
- 関連 issue: 0144 (ローカル開始 WT bidi ストリームの登録 API。本 issue は uni 側の拡張)
- 一次資料: `refs/quic/rfc9000.txt` Section 2.1、`refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 4.2
