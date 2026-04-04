# datagram 受信時の SETTINGS ネゴシエーション確認とエラーコードの修正

Created: 2026-04-05
Completed: 2026-04-05
Model: Opus 4.6

## 優先度

P1

## 概要

`feed_datagram()` が SETTINGS_H3_DATAGRAM のネゴシエーション完了を確認せずに datagram を受理している。また、session ID の型違反時のエラーコードが仕様と異なる。

## 根拠

### SETTINGS_H3_DATAGRAM ネゴシエーション未確認

RFC 9297 は以下を要求している:

> QUIC DATAGRAM frames MUST NOT be sent until the SETTINGS_H3_DATAGRAM setting has been both sent and received with a value of 1.

`src/connection/mod.rs` L478 の `feed_datagram()` は接続エラー状態のみ確認し、SETTINGS_H3_DATAGRAM=1 の双方向ネゴシエーション完了を検証していない。ネゴシエーション前に送られた datagram を受理してしまう。

### エラーコードの不一致

`src/connection/mod.rs` L494-497 で session ID がクライアント開始双方向ストリーム ID でない場合に `GeneralProtocolError` を返している。

draft-ietf-webtrans-http3-15 Section 4.5 は以下を要求している:

> If the Session ID does not correspond to a client-initiated bidirectional stream, the receiver MUST close the connection with an H3_ID_ERROR error code.

正しくは `H3_ID_ERROR` (`ErrorCode::IdError`) を使うべき。

## 対応方針

- `feed_datagram()` の冒頭で SETTINGS_H3_DATAGRAM=1 が双方向にネゴシエーション済みか確認し、未完了なら接続エラーを返す
- session ID の型違反時のエラーコードを `ErrorCode::IdError` に変更する

## 解決方法

- `feed_datagram()` の冒頭で `local_settings.h3_datagram` と `peer_settings.h3_datagram` の両方が `Some(true)` であることを確認するガードを追加した。未完了の場合は `GeneralProtocolError` を返す
- session ID の型違反時のエラーコードを `ErrorCode::IdError` に変更した

## 参照

- RFC 9297 Section 3
- draft-ietf-webtrans-http3-15 Section 4.5
- `src/connection/mod.rs` L478-497
