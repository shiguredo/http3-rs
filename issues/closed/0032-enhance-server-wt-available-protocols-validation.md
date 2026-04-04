# サーバー受信時に WT-Available-Protocols を保持し send_response() で WT-Protocol を検証する

Created: 2026-04-05
Completed: 2026-04-05
Model: Opus 4.6

## 優先度

P2

## 概要

サーバーが WebTransport CONNECT リクエストを受信した際、クライアントが送信した `WT-Available-Protocols` ヘッダーを `WtSession` に保存していない。そのため `send_response()` で `WT-Protocol` ヘッダーの整合性を検証できず、仕様違反のレスポンスを返せる。

## 根拠

draft-ietf-webtrans-http3-15 Section 3.3:

> If the server receives a request with a WT-Available-Protocols header field, it MUST NOT reply with a protocol that is not listed in the client's request.

クライアント側の `send_request()` (`src/connection/mod.rs:2310-2323`) では `WT-Available-Protocols` をパースして `session.available_protocols` に保存し、レスポンス受信時 (`src/connection/mod.rs:1966`) に `WT-Protocol` との整合性を検証している。

一方、サーバー側の `emit_header_events()` (`src/connection/mod.rs:1930-1933`) では `WtSession::new()` を挿入するだけで `WT-Available-Protocols` のパース・保存を行っていない。

結果として `send_response()` (`src/connection/mod.rs:2329`) に WebTransport 固有の検証がなく、サーバーはクライアントが提示していないプロトコルを `WT-Protocol` で返せる。

## 対応方針

1. `emit_header_events()` の WebTransport CONNECT 受信分岐 (`src/connection/mod.rs:1930`) で、デコード済みヘッダーから `wt-available-protocols` をパースし `WtSession.available_protocols` に保存する
2. `send_response()` で WebTransport セッションへの 2xx レスポンス送信時、`WT-Protocol` ヘッダーが `available_protocols` に含まれるプロトコルであることを検証する。`available_protocols` が空でない (クライアントがヘッダーを送信した) 場合に不一致なら `InternalError` を返す

## 解決方法

1. `emit_header_events()` の WebTransport CONNECT 受信分岐で、デコード済みヘッダーから `wt-available-protocols` をパースし `WtSession.available_protocols` に保存するようにした
2. `send_response()` で 2xx レスポンス送信時、WT セッションの `available_protocols` が空でない場合に `wt-protocol` ヘッダーの値が `available_protocols` に含まれることを検証するガードを追加した。不一致または `wt-protocol` 未指定の場合は `InternalError` を返す

## 参照

- draft-ietf-webtrans-http3-15 Section 3.3
- `src/connection/mod.rs:1930-1933` (サーバー側セッション登録)
- `src/connection/mod.rs:2310-2323` (クライアント側 available_protocols 保存)
- `src/connection/mod.rs:1966` (クライアント側 WT-Protocol 検証)
- `src/connection/mod.rs:2329` (send_response)
