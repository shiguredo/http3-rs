# サーバーがクライアント開始の WebTransport 双方向ストリームを受理できない

Created: 2026-04-05
Completed: 2026-04-05
Model: Opus 4.6

## 優先度

P0

## 概要

`feed_stream_data` において、サーバーがクライアントから開かれた WebTransport 双方向ストリーム (シグナル値 `0x41`) を認識する経路が存在しない。すべてのクライアント開始双方向ストリームが HTTP/3 リクエストストリームとして処理される。

## 根拠

draft-ietf-webtrans-http3-15 Section 4.3 は以下のように明記している:

> Clients and servers use the signal value 0x41 to open a bidirectional WebTransport stream.

つまり、クライアントもサーバーも `0x41` で WT 双方向ストリームを開く前提。

## 現状の問題

`src/connection/mod.rs` L760-768 のフロー:

1. `wt_bidi_streams` / `pending_wt_bidi_streams` に既登録 → WT bidi として処理
2. それ以外 → `handle_bidirectional_stream` (リクエストストリーム扱い)

`wt_bidi_streams` への登録は `resolve_wt_bidi_stream_header()` で行われるが、この関数は server-initiated bidi stream 専用パスからしか呼ばれない。新規のクライアント開始双方向ストリームで `0x41` シグナルを検出するコードが存在しない。

## 影響

WebTransport のクライアント側が双方向ストリームを開いてもサーバーが受理できず、HTTP/3 フレームとしてパースを試みて失敗する。WebTransport の基本機能が部分的に欠落している。

## 対応方針

`feed_stream_data` でサーバーがクライアント開始双方向ストリームを受信した際に、先頭バイトが `0x41` かどうかを判定し、WT bidi ストリームとして `resolve_wt_bidi_stream_header` に流す経路を追加する。設計レベルの変更が必要。

## 解決方法

`feed_stream` にクライアント開始 bidi ストリームのディスパッチロジックを追加した。

- `dispatch_client_bidi_stream` メソッドを新設。先頭 varint をデコードし、値が `0x41` (WT_STREAM) なら `handle_wt_bidi_stream` へ、それ以外なら `handle_bidirectional_stream` へ振り分ける
- varint が不完全な場合は `pending_bidi_dispatch` にバッファリングし、後続データで判定を完了する
- `pending_bidi_dispatch` フィールドを `Connection` に追加。`pending_wt_bidi_streams` (WT_STREAM 確定後の session_id 解決用) とは分離した

## 参照

- draft-ietf-webtrans-http3-15 Section 4.3
- `src/connection/mod.rs` L746-768
