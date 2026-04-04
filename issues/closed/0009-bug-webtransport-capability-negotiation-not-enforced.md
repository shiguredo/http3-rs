# WebTransport 能力ネゴシエーションがコア接続層に結び付いていない

Created: 2026-04-05
Model: Opus 4.6

## 概要

`TransportCapabilities::validate()` (`src/webtransport/connect.rs:249`) は存在するが、`Connection` の `send_request()` や受信側ヘッダー処理から一切呼ばれていない。仕様上拒否すべき WebTransport セッション確立が、普通の Extended CONNECT と同様に通ってしまう。

## 根拠

- draft-ietf-webtrans-http3-15 Section 3.1: 「Clients MUST NOT attempt to establish WebTransport sessions until they have received the setting indicating WebTransport support from the server.」
- draft-ietf-webtrans-http3-15 Section 4.6: クライアントはサーバーの SETTINGS フレーム受信前に WebTransport セッションを開始してはならない
- draft-ietf-webtrans-http3-14 Section 3.1: サーバー側でも WebTransport 前提条件のチェックが必要

## 問題

- クライアント側: peer の `SETTINGS_WT_ENABLED` を確認せずに WebTransport CONNECT を送信できる
- サーバー側: peer の SETTINGS 未受信でも WebTransport リクエストを処理できる
- `validate_request_headers()` は Extended CONNECT の文法チェックのみで、WebTransport 固有の前提条件を検証しない

## 対応方針

- `send_request()` で WebTransport CONNECT (`:protocol` が `webtransport` 系) を検出した場合、peer SETTINGS の `SETTINGS_WT_ENABLED` / `SETTINGS_ENABLE_CONNECT_PROTOCOL` / `SETTINGS_H3_DATAGRAM` を検証する
- サーバー側の受信ヘッダー処理でも同様に peer SETTINGS を確認する
- 前提条件未充足の場合はエラーを返す

Completed: 2026-04-05

## 解決方法

- クライアント側 `send_request()`: `:protocol` が `webtransport-h3` / `webtransport` の CONNECT リクエスト送信時に、peer SETTINGS の `is_webtransport_enabled()` / `enable_connect_protocol` / `h3_datagram` を検証するようにした。peer SETTINGS 未受信の場合もエラーを返す。
- サーバー側 `emit_header_events()`: WebTransport CONNECT リクエスト受信時に、ローカル設定の `is_webtransport_enabled()` と peer の `h3_datagram` を検証するようにした。
- 通常の Extended CONNECT (websocket 等) は WebTransport チェックの対象外。
- `is_webtransport_connect()` / `is_webtransport_connect_decoded()` ヘルパー関数を追加した。
