# サーバー側の WebTransport SETTINGS 前提条件チェックが不完全

Created: 2026-04-05
Completed: 2026-04-05
Model: Opus 4.6

## 優先度

P1-P2 境界

## 概要

サーバーが WebTransport CONNECT リクエストを受け入れる際の前提条件チェックが不完全。現状は `local_settings.is_webtransport_enabled()` と peer の `H3_DATAGRAM` のみを確認しているが、peer (クライアント) の `SETTINGS_WT_ENABLED` と `ENABLE_CONNECT_PROTOCOL` を確認していない。

## 根拠

- draft-ietf-webtrans-http3-15 Section 3.1 (L402-462)
- draft-ietf-webtrans-http3-15 Section 7.1 (L1548-1564)

## 解決方法

サーバー側の `emit_header_events()` 内の WebTransport CONNECT 前提条件チェックを強化した:

1. `peer_settings` が `None` (クライアント SETTINGS 未受信) の場合は `MessageError` を返すようにした
2. peer の `SETTINGS_WT_ENABLED` が有効でない場合は `MessageError` を返すようにした
3. peer の `ENABLE_CONNECT_PROTOCOL` が有効でない場合は `MessageError` を返すようにした
4. 既存の peer の `H3_DATAGRAM` チェックは維持

これによりクライアント送信側 (3 条件チェック) とサーバー受信側の前提条件チェックが対称になった。
