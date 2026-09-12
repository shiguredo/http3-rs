# ローカル開始 uni ストリームへの STREAM / FIN が WebTransport データとして誤処理される

- Created: 2026-09-12
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-wt-local-uni-stream-data-fin-guard
- Polished: {YYYY-MM-DD}

## 目的

WT データストリームとして登録済みのローカル開始 uni ストリーム (ピアは受信専用) に STREAM (受信データ) / FIN が到達した場合の防御を追加し、クレジット回復・データ FC 計上・登録除去・イベント発火を防ぐ。

## 現状

- `src/connection/wt_stream.rs` の `handle_wt_uni_stream_data` は `wt_uni_streams` に登録済みのストリームを WT データとして処理し、データ FC を計上して `Event::WebTransport(UniStreamData)` を発火する
- `src/connection/wt_stream.rs` の `handle_wt_uni_stream_fin` は登録を除去し、`on_remote_stream_closed(false)` による WT_MAX_STREAMS クレジット回復と `Event::WebTransport(UniStreamEnd)` の発火を行う
- ローカル開始 uni はローカルにとって送信専用であり、ピアからの STREAM / FIN は RFC 9000 Section 19.8 の STREAM_STATE_ERROR に相当する。通常は QUIC 層で拒否されるため sans-I/O には到達しない
- 0197 で RESET_STREAM 経路 (`handle_wt_stream_reset`) には同じ趣旨の防御を追加したが、STREAM / FIN 経路には未対応で、統合層の不具合や将来の QUIC 実装変更で到達した場合に不正処理が起きる (0197 のレビューで検出)
- `register_local_wt_stream` の doc は RESET_STREAM のみを防御対象として記載しており、STREAM / FIN との非対称が残る

## 設計方針

- `handle_wt_uni_stream_data` / `handle_wt_uni_stream_fin` で、`wt_uni_streams` に登録済みのローカル開始 uni (`is_local_initiated_uni(kind)` が true) は早期 return して静かに吸収する
- `handle_wt_uni_stream_data` は `Ok(true)`、`handle_wt_uni_stream_fin` は `true` を返し、呼び出し元が汎用イベントを発火しないようにする
- データ FC 計上・WT_MAX_STREAMS クレジット回復・`remove_stream_received_data`・登録除去・WT イベント発火を行わない
- `register_local_wt_stream` と該当関数の doc を防御範囲 (RESET_STREAM / STREAM / FIN) に合わせて更新する

## 完了条件

- ローカル開始 uni を登録した状態で STREAM / FIN を feed しても、クレジット回復・データ FC 計上・登録除去・WT イベント発火が起きない
- テストを追加する (サーバー / クライアント両ロール。モック・スタブ不使用)
- ピア開始 uni の STREAM / FIN は従来どおり処理される
- `cargo test --workspace --tests` / `cargo fmt --all -- --check` / `cargo clippy --workspace --all-targets -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/connection/wt_stream.rs` (`handle_wt_uni_stream_data` / `handle_wt_uni_stream_fin` / `register_local_wt_stream` の doc)
- `src/connection/mod.rs` (テスト追加)

### 一次資料

- `refs/quic/rfc9000.txt` Section 19.8 (STREAM フレームと STREAM_STATE_ERROR)
- `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 4.2 / Section 5.3

### 関連 issue

- 0197 (RESET_STREAM 経路の同種の防御。本 issue はその未対応の STREAM / FIN 経路)
