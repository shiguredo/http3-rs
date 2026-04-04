# WebTransport CONNECT で :scheme = https を強制していない

Created: 2026-04-05
Completed: 2026-04-05
Model: Opus 4.6

## 優先度

P1

## 概要

サーバー側で WebTransport CONNECT リクエストを受信した際、`:scheme` が `https` であることを検証していない。draft-ietf-webtrans-http3-15 Section 3.2 の MUST 違反。

## 根拠

draft-ietf-webtrans-http3-15 Section 3.2:

> The :scheme pseudo-header field MUST be "https".

`ConnectRequest::validate()` (`src/webtransport/connect.rs:478`) に `:scheme == https` の検証が実装済みだが、サーバー側の受信パス `emit_header_events()` (`src/connection/mod.rs:1888`) では呼び出されていない。

`validate_request_headers()` (`src/validation.rs:428`) は Extended CONNECT の汎用検証 (`:scheme` の存在確認、URI scheme としての妥当性) のみ行い、`https` に限定する検証は行わない。

結果として、`:scheme = http` や任意のスキームで WebTransport CONNECT が受理される。

## 再現手順

1. WebTransport CONNECT リクエストを `:scheme = http` で送信する
2. サーバー側でエラーなく受理される

## 対応方針

`emit_header_events()` の WebTransport CONNECT 分岐 (`src/connection/mod.rs:1888`) で、デコード済みヘッダーから `:scheme` を取得し `https` であることを検証する。`https` でない場合は `H3_MESSAGE_ERROR` で拒否する。

## 解決方法

`emit_header_events()` の WebTransport CONNECT 分岐で、デコード済みヘッダーから `:scheme` が `https` であることを確認するガードを追加した。`https` でない場合は `H3_MESSAGE_ERROR` でストリームエラーを返す。

## 参照

- draft-ietf-webtrans-http3-15 Section 3.2
- `src/connection/mod.rs:1888`
- `src/webtransport/connect.rs:478`
- `src/validation.rs:428`
