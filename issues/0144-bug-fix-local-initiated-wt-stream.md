# ローカル開始の WT ストリーム受信データがリクエストストリームとして誤処理される

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-local-initiated-wt-stream
- Polished: {YYYY-MM-DD}

## 目的

アプリが自分で開いた WT ストリーム (クライアント開始 bidi 等) への受信データが HTTP/3 リクエストストリームとして誤解析される問題を修正する。

## 現状

- `src/connection/mod.rs` の `Connection.wt_uni_streams` / `wt_bidi_streams` は受信経路 (ピアが開いたストリーム) でのみ登録される
- ローカル側が開いた WT ストリーム (クライアント開始 bidi の下り、サーバー開始 bidi / uni) を `Connection::feed_stream` に渡すと、`wt_bidi_streams` に無いため `Connection::handle_bidirectional_stream` でリクエストストリームとして処理され、WT ペイロードが HTTP/3 フレームとして誤解釈される
- ローカル開始ストリームを登録する API が存在しない (実装漏れ)
- WebTransport の基本利用 (アプリが開いたストリームへの応答データ受信) で必ず発生する経路

## 設計方針

- ローカル開始 WT ストリームの登録 API を追加する (ストリーム ID とセッション ID の関連付け)
- `feed_stream` で登録済みストリーム ID が来たら WT ストリームとして処理する

## 完了条件

- ローカル開始の WT bidi / uni ストリームに受信データを feed すると WT ストリームとして処理される
- テストが追加される
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/connection/mod.rs` (`Connection::feed_stream` / `Connection::handle_bidirectional_stream` / `wt_uni_streams` / `wt_bidi_streams`)
- 一次資料: `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 4.2 / 4.3
