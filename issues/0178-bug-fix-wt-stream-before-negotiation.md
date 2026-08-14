# WT ネゴシエーション完了前に到着した先頭 0x41 の bidi ストリームがリクエストストリームとして誤処理される

- Created: 2026-08-14
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-wt-stream-before-negotiation
- Polished: {YYYY-MM-DD}

## 目的

WT ネゴシエーション未完了時に到着した正当な WT bidi ストリームがリクエストストリームとして誤処理され、接続が壊れる問題を修正する。

## 現状

- サーバー側の新規クライアント開始 bidi ストリームの振り分けは `src/connection/mod.rs` の `Connection::dispatch_client_bidi_stream` が行い、`is_wt_fully_negotiated()` が true のときのみ先頭 varint を捕捉して WT 経路 (`handle_wt_bidi_stream`) に回す
- ネゴシエーション未完了 (クライアントの SETTINGS 未受信) の間に到着した先頭 0x41 のストリームは `handle_bidirectional_stream` 経由で `RequestStream::process_raw` に落ちる
- QUIC はストリーム間の到着順を保証しないため、ネゴシエーション未完了時の bidi ストリーム到着は起こり得る
- `RequestStream::process_raw` の `Frame::Unknown` 分岐は 0x41 を未知フレームとして無視する (サーバー先頭位置のみ) が、WT_STREAM は length を持たない (draft-ietf-webtrans-http3-16 Section 4.3: "WT_STREAM lacks length and is not a proper HTTP/3 frame") ため、ワイヤ上の 2 番目の varint (session_id) が HTTP/3 フレームの length として解釈される
- session_id が 0 でなければ実ペイロードの先頭が length 分巻き込まれ、以降の解析がずれて接続エラー (H3_FRAME_ERROR 等) に至り得る

## 設計方針

- ネゴシエーション未完了時に先頭 0x41 の bidi ストリームを受信した場合の扱いを確定する (例: ストリームをバッファリングしてネゴシエーション完了後に WT 経路に回す、または 0x41 をフレームとして解釈せず正しく破棄する)
- 0x41 を「length 前置の HTTP/3 フレーム」として解釈しないこと (WT_STREAM は length を持たず、2 番目の varint は session_id)
- サーバー側先頭位置の 0x41 無視 (0142 で実装済み) との整合を保つ

## 完了条件

- ネゴシエーション未完了時に先頭 0x41 の bidi ストリームが到着しても、ボディが誤解析されず接続が壊れない
- ネゴシエーション完了後に到着した同ストリームは WT ストリームとして処理される
- テストが追加される (`src/connection/mod.rs` の `#[cfg(test)]` モジュールでネゴシエーション未完了時の到着順を再現する統合テスト)
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

- (未定)
