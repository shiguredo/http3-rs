# WebTransport 単方向ストリームの session_id が検証されていない

Created: 2026-04-05
Completed: 2026-04-05
Model: Opus 4.6

## 優先度

P1

## 概要

`Connection::resolve_wt_uni_stream_session_id()` で、varint デコードした `session_id` が client-initiated bidirectional stream ID であるかを検証せずに、そのまま `Event::WebTransportUniStreamOpen` を発火している。

draft-ietf-webtrans-http3-15 Section 4.2 は、`session_id` が client-initiated bidirectional stream ID に対応しない場合、`H3_ID_ERROR` で接続を閉じることを MUST としている。

## 根拠

- draft-ietf-webtrans-http3-15 Section 4.2 (L653-662)
  - Session ID は CONNECT ストリームの stream ID から派生し、常に client-initiated bidirectional stream に対応する MUST 要件
  - 単方向ストリーム、双方向ストリーム、データグラムで受信した `session_id` が client-initiated bidirectional stream ID に対応しない場合、`H3_ID_ERROR` で接続を閉じる MUST 要件
  - 閉じられたセッションに対応する `session_id` はこのチェックでは無効とみなさない

## 解決方法

`resolve_wt_uni_stream_session_id()` の varint デコード直後に `session_id & 0x03 != 0x00` の検証を追加した。RFC 9000 Section 2.1 により、client-initiated bidirectional stream ID は `stream_id % 4 == 0` で判定できる。検証失敗時は `Error::ConnectionError(ErrorCode::IdError)` を返す。

不正な session_id (server-initiated bidi, client-initiated uni, server-initiated uni) に対するテストを 3 件追加した。
