# GOAWAY 受信後も新規 WebTransport セッションの Pending 作成が継続する

Created: 2026-04-06
Model: Opus 4.6

## 優先度

P2

## 概要

GOAWAY 受信後も `associate_or_buffer_stream()` と `feed_datagram()` で未知の session_id に対する Pending セッション作成が継続する。また、既存 WebTransport セッションへの draining 伝播がない。

## 根拠

draft-ietf-webtrans-http3-15 Section 4.7 は GOAWAY を全 WebTransport セッションの shutdown 開始シグナルとして扱っている。nghttp3 は GOAWAY 後の未知 session を reject している (`nghttp3_conn.c` L3654 付近)。

現在の実装では:

- GOAWAY 受信処理 (`src/connection/mod.rs` L1744 付近) は `goaway_received` フラグと `goaway_id` を記録し、`Event::GoawayReceived` を発火するのみ
- `send_request()` (`src/connection/mod.rs` L2347 付近) では GOAWAY ID 以上のストリーム作成を禁止しているが、これは新規リクエスト送信の制限のみ
- 既存 WebTransport セッションへの draining 伝播がない
- GOAWAY 後も `associate_or_buffer_stream()` / `feed_datagram()` で未知 session_id の Pending セッション作成が継続する

## 影響

GOAWAY 後にサーバーが送信した WT traffic が新規 Pending セッションとしてバッファリングされ、graceful shutdown が妨げられる。

## 対応方針

1. GOAWAY 受信後、`associate_or_buffer_stream()` / `feed_datagram()` で未知 session_id の新規 Pending セッション作成を拒否する
2. GOAWAY 受信時に既存の Pending / Established セッションに対して draining を伝播する (GOAWAY ID と session_id の関係に基づく)
3. `Event::WebTransportSessionDraining` を適切に発火する

## 解決方法

Completed: 2026-04-06

1. `associate_or_buffer_stream()` と `feed_datagram()` の `else` 分岐に `self.goaway_received` チェックを追加し、GOAWAY 受信後は新規 Pending セッション作成を拒否するようにした。
2. GOAWAY 受信処理に draining 伝播を追加した。GOAWAY ID 以上の session_id を持つ既存の Pending/Established セッションに対して `Event::WebTransportSessionDraining` を発火する。

## 参照

- draft-ietf-webtrans-http3-15 Section 4.7
- nghttp3 `nghttp3_conn.c` L3654 付近
- `src/connection/mod.rs` L504, L1437, L1744, L2347
