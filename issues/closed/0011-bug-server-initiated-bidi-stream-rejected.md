# server-initiated 双方向 WebTransport ストリームがクライアント側で無条件拒否されている

Created: 2026-04-05
Completed: 2026-04-05
Model: Opus 4.6

## 優先度

P1

## 概要

`Connection::receive_data()` で、クライアントが server-initiated bidirectional stream を受信した場合、WebTransport の有効/無効にかかわらず無条件に `H3_STREAM_CREATION_ERROR` を返している。

draft-ietf-webtrans-http3-15 Section 4.3 は server-initiated bidirectional stream を WebTransport 用に拡張しており、先頭の signal value `0x41` と `session_id` を読んでセッションに関連付けることを要求している。

## 根拠

- draft-ietf-webtrans-http3-15 Section 4.3 (L733-763)

## 解決方法

1. `Event` に `WebTransportBidiStreamOpen` / `WebTransportBidiStreamData` / `WebTransportBidiStreamEnd` を追加した
2. `Connection` に `wt_bidi_streams` (確定済み) と `pending_wt_bidi_streams` (ヘッダー未確定バッファ) を追加した
3. `feed_stream` で WebTransport 有効 + クライアント + server-initiated bidi の場合に `handle_wt_bidi_stream` に分岐するようにした
4. `handle_wt_bidi_stream` で signal value (0x41) と session_id の varint パース (チャンク分割対応) を実装した
5. signal value が 0x41 でない場合は `H3_FRAME_ERROR`、session_id が不正な場合は `H3_ID_ERROR` を返すようにした
6. WebTransport 無効時は従来通り `H3_STREAM_CREATION_ERROR` を返す
