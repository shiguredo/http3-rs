# RESET_STREAM 受信時にフロー制御クレジットが計上・回復されない

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-wt-reset-flow-control-credit
- Polished: {YYYY-MM-DD}

## 目的

ピアがデータストリームを RESET_STREAM で閉じた場合の WT フロー制御クレジット (データ量・ストリーム数) の計上漏れを修正する。

## 現状

- `src/connection/wt_session.rs` の `Connection::handle_wt_stream_reset` は `StreamReset` イベントを発火するだけで、以下の処理をしていない
  - RESET_STREAM の final_size から WT ストリームヘッダー長を引いた量を受信側データフロー制御に計上しない。受信側は実際に消費されたクレジットを過少計上し、広告した `WT_MAX_DATA` を超えるデータをピアに許す (draft-16 Section 5.4 の MUST)
  - `WtSession::on_remote_stream_closed` を呼ばず、`WT_MAX_STREAMS` のクレジットが返却されない。ピアがストリームを使い捨て (開いて即 RESET) すると受信側ウィンドウが枯渇し、以後の正常なストリームが `WT_FLOW_CONTROL_ERROR` でセッション終了する
- FIN 経路 (`BidiStreamEnd` / `UniStreamEnd`) では `on_remote_stream_closed` が呼ばれており、RESET 経路だけが非対称

## 設計方針

- `handle_wt_stream_reset` で final_size を受信側データ FC に計上する (`wt_stream_header_len` を差し引く)
- リセットされたストリームに対しても `on_remote_stream_closed` を呼びストリーム数クレジットを回復する

## 完了条件

- RESET_STREAM 受信時にデータ FC・ストリーム数 FC のクレジットが正しく計上・回復される
- テストが追加される
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/connection/wt_session.rs` (`Connection::handle_wt_stream_reset` / `WtSession::on_remote_stream_closed`)
- `src/connection/wt_stream.rs` (`Connection::wt_stream_header_len`)
- 一次資料: `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 5.3 / 5.4
