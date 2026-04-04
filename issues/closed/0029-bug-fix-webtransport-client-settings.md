# WebTransport クライアント設定で ENABLE_CONNECT_PROTOCOL を送らない

Created: 2026-04-06
Completed: 2026-04-06
Model: Opus 4.6

## 優先度

P1

## 概要

`Settings::enable_webtransport()` がロール非依存で `enable_connect_protocol = Some(true)` を立てており、クライアント時にも `SETTINGS_ENABLE_CONNECT_PROTOCOL` が送出される。draft-ietf-webtrans-http3-15 および RFC 9220 / RFC 8441 の仕様上、この設定はサーバーが CONNECT 拡張の受諾を広告するためのものであり、クライアントが送信する項目ではない。

## 根拠

- draft-ietf-webtrans-http3-15 Section の「サーバー送信項目」「クライアント送信項目」の非対称定義:
  - サーバー: `SETTINGS_WT_ENABLED`, `SETTINGS_ENABLE_CONNECT_PROTOCOL`, `SETTINGS_H3_DATAGRAM`, `max_datagram_frame_size`, `reset_stream_at`
  - クライアント: `SETTINGS_H3_DATAGRAM`, `max_datagram_frame_size`, `reset_stream_at`（および draft 版のみ `SETTINGS_WT_ENABLED`）
- RFC 9220 / RFC 8441 Section 3: `SETTINGS_ENABLE_CONNECT_PROTOCOL` はサーバー広告用
- nghttp3 もこの非対称性を前提に実装されている

## 修正方針

`Settings::enable_webtransport()` を削除し、以下に分離する（破壊的変更）:

- `enable_webtransport_client(wt: webtransport::Settings) -> Self`
  - `h3_datagram = Some(true)`
  - `wt_settings = Some(wt)`
  - `enable_connect_protocol` には触れない
- `enable_webtransport_server(wt: webtransport::Settings) -> Self`
  - `enable_connect_protocol = Some(true)`
  - `h3_datagram = Some(true)`
  - `wt_settings = Some(wt)`

呼び出し側 (wrapper / tests / examples) をすべて追従させる。

## 影響

- 破壊的変更: `CHANGES.md` に `[CHANGE]` で記録
- テスト: クライアント SETTINGS の wire encoding に `SETTINGS_ENABLE_CONNECT_PROTOCOL` が含まれないこと、サーバー側は従来通り含まれることを検証

## 解決方法

`src/settings.rs` で `enable_webtransport()` を以下 2 つに分離した (破壊的変更):

- `enable_webtransport_server(wt)`: `enable_connect_protocol = Some(true)` + `h3_datagram = Some(true)` + `wt_settings`
- `enable_webtransport_client(wt)`: `h3_datagram = Some(true)` + `wt_settings` のみ
  (`SETTINGS_ENABLE_CONNECT_PROTOCOL` は送信しない)

呼び出し側の移行:

- `crates/tokio-s2n-quic/src/config.rs`: `ServerConfig::enable_webtransport` は
  `enable_webtransport_server`、`ClientConfig::enable_webtransport` は
  `enable_webtransport_client` を内部で呼ぶように変更
- `pbt/tests/prop_settings.rs`, `src/settings.rs` (tests), `src/connection/mod.rs` (tests),
  `tests/test_webtransport_draft_connect.rs`: 既存の `.enable_webtransport(` 呼び出しを
  `.enable_webtransport_server(` に一括置換 (振る舞いは旧メソッドと同一)

Completed: 2026-04-06
