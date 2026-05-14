# send_datagram() で SETTINGS_H3_DATAGRAM のネゴシエーションを確認していない

Created: 2026-04-05
Completed: 2026-04-05
Model: Opus 4.6

## 優先度

P1

## 概要

`send_datagram()` が SETTINGS_H3_DATAGRAM=1 の双方向ネゴシエーション完了を確認せずにデータグラムをエンコードしている。`feed_datagram()` (受信側) では issue #0025 で修正済みだが、送信側に同じ確認が漏れている。

## 根拠

draft-ietf-webtrans-http3-15 Section 2.1.1:

> QUIC DATAGRAM frames MUST NOT be sent until the SETTINGS_H3_DATAGRAM setting has been both sent and received with a value of 1.

`feed_datagram()` (`src/connection/mod.rs:498-507`) では `local_settings.h3_datagram` と `peer_settings.h3_datagram` の両方が `Some(true)` であることを確認している。

一方 `send_datagram()` (`src/connection/mod.rs:562-581`) では以下のみ確認している:

- session_id の形式
- セッションの存在
- セッション状態が Established

SETTINGS のネゴシエーション完了は確認していない。Sans I/O ライブラリとして、送信 API 側でもネゴシエーション完了を前提条件として検証すべき。

## 再現手順

1. ピアの SETTINGS 受信前に `send_datagram()` を呼ぶ
2. エラーなくエンコード済みデータグラムが返される

## 対応方針

`send_datagram()` の冒頭で `feed_datagram()` と同じネゴシエーション確認を追加する。未完了の場合は `GeneralProtocolError` を返す。

## 解決方法

`send_datagram()` の冒頭で `feed_datagram()` と同じネゴシエーション確認を追加した。`local_settings.h3_datagram` と `peer_settings.h3_datagram` の両方が `Some(true)` でない場合は `GeneralProtocolError` を返す。

## 参照

- draft-ietf-webtrans-http3-15 Section 2.1.1
- `src/connection/mod.rs:498-507` (feed_datagram のネゴシエーション確認)
- `src/connection/mod.rs:562-581` (send_datagram)
- `issues/closed/0025-bug-datagram-missing-settings-negotiation.md`
