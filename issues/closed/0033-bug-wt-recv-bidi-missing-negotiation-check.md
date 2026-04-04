# WebTransport 受信側 bidi stream 判定で交渉条件が不足している

Created: 2026-04-05
Completed: 2026-04-06
Model: Opus 4.6

## 優先度

P1

## 概要

クライアントが server-initiated bidi stream を受信した際、`local_settings.is_webtransport_enabled()` のみで WebTransport 判定している。送信側 `send_request()` では peer の `WT_ENABLED` / `ENABLE_CONNECT_PROTOCOL` / `H3_DATAGRAM` / `wt_transport_verified` を全て確認しているが、受信側はこれらを一切見ていない。

## 根拠

送信側 (`connection/mod.rs` L2282-2307) の交渉条件:

1. `peer.is_webtransport_enabled()` — peer が WT を有効にしているか
2. `peer.enable_connect_protocol` — Extended CONNECT をサポートしているか (draft-02 以外)
3. `peer.h3_datagram` — H3_DATAGRAM が有効か
4. `self.wt_transport_verified` — QUIC transport parameter の前提条件が満たされているか

受信側 (`connection/mod.rs` L784-793) の交渉条件:

1. `self.local_settings.is_webtransport_enabled()` のみ

この不整合により、peer が WT 非対応でも local 設定だけで WT 有効なら server-initiated bidi stream を WebTransport として処理してしまう。さらに `associate_or_buffer_stream()` がセッション未登録でも新規 Pending セッションを作成するため、不正なストリームがバッファリングされる。

draft-ietf-webtrans-http3-15 Section 4.3 では、server は CONNECT request が成立した後にセッション用 bidi stream を開く前提であり、交渉未完了の状態で受け入れるのは仕様違反。

## 対応方針

`feed_stream()` の server-initiated bidi stream 判定を送信側と同じ粒度にそろえる:

1. `peer_settings` が受信済みかつ `is_webtransport_enabled()` であること
2. draft-02 でなければ `enable_connect_protocol == Some(true)` であること
3. `h3_datagram == Some(true)` であること
4. `wt_transport_verified` が true であること

全条件を満たさない場合は `StreamCreationError` で拒否する（現行の WT 無効時と同じ）。

共通の判定ロジックを抽出するかは実装時に判断する。

## 参照

- draft-ietf-webtrans-http3-15 Section 3.1, 4.3
- `src/connection/mod.rs` L784-793 (受信側)
- `src/connection/mod.rs` L2282-2307 (送信側)

## 解決方法

`Connection` に `is_wt_fully_negotiated()` ヘルパーメソッドを追加し、送信側 (`send_request`) と同じ全条件（`peer.is_webtransport_enabled()` / `enable_connect_protocol` / `h3_datagram` / `wt_transport_verified` / draft-15 の `wt_reset_stream_at_supported`）を検証するようにした。

受信側の 2 箇所を修正:
1. クライアント: server-initiated bidi stream 受信パス (`feed_stream` L796)
2. サーバー: client-initiated bidi stream の WT dispatch パス (`feed_stream` L823)

両パスとも `local_settings.is_webtransport_enabled()` → `is_wt_fully_negotiated()` に置き換え。ネゴシエーション未完了時は `StreamCreationError` で拒否する（従来と同じエラーコード）。
