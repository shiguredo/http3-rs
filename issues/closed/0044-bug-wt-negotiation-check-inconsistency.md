# WebTransport 受信判定が経路ごとに不整合 (uni stream / datagram のガードが甘い)

Created: 2026-04-06
Completed: 2026-04-07
Model: Opus 4.6

## 解決方法

`feed_datagram()` と単方向ストリーム 0x54 の経路で `is_wt_fully_negotiated()` を呼ぶように変更し、双方向 stream 経路と判定基準を一本化した。datagram は未確立時に静かに破棄 (RFC 9297 と整合)、uni stream は `H3_STREAM_CREATION_ERROR` 相当でストリームエラーにする。テストヘルパ `make_negotiated_wt_server()` を追加し、既存のサーバー単体 WT uni stream テストを peer SETTINGS 受信済み状態に揃えた。

## 優先度

P2

## 概要

サーバー側で WebTransport トラフィックを受信する経路ごとに、ネゴシエーション完了の判定基準が不整合になっている。

- 双方向 stream 受信: `is_wt_fully_negotiated()` で厳格に判定 (`src/connection/mod.rs` L1617 付近)
- 単方向 stream 受信: `local_settings.is_webtransport_enabled()` のみ (`src/connection/mod.rs` L1180 付近)
- Datagram 受信: `H3_DATAGRAM` のみ (`src/connection/mod.rs` L661 付近)

draft-ietf-webtrans-http3-15 Section 7.1 では、サーバーはクライアントの SETTINGS を受信する前に WebTransport request を処理してはならない、と明示している。現状では、クライアント SETTINGS 未受信、`SETTINGS_WT_MAX_SESSIONS` 未成立、`reset_stream_at` transport parameter 未確認といった状態でも、uni stream / datagram 経由なら処理に入ってしまう。

## 根拠

- draft-ietf-webtrans-http3-15 Section 4 (L653-662)
- draft-ietf-webtrans-http3-15 Section 7.1 (L1560 付近)
- nghttp3 `lib/nghttp3_conn.c` L54
- `src/connection/mod.rs` L661 (`feed_datagram`)
- `src/connection/mod.rs` L959, L1180 (`resolve_wt_uni_stream_header`)
- `src/connection/mod.rs` L1617 (双方向 stream 経路)

## 影響

- クライアント SETTINGS 受信前にサーバーが WebTransport トラフィックを処理し始める可能性
- ネゴシエーションが完了していない状態でセッション関連状態が生成され、その後のハンドシェイクと矛盾する
- 双方向 stream 経路と挙動が食い違うため、PBT / fuzzing が経路依存の異常状態を引きやすい

## 対応方針

1. `is_wt_fully_negotiated()` を WT 受信判定の一次関数として確立する (peer SETTINGS 受信 + `SETTINGS_WT_MAX_SESSIONS` + `H3_DATAGRAM` + 必要なら `reset_stream_at` を含む)
2. `feed_datagram()` 冒頭、`resolve_wt_uni_stream_header()` 冒頭、双方向 stream 経路の三箇所で同じ関数を呼ぶ
3. 未確立で受信した場合の扱いをそれぞれ定義する:
   - uni stream: `H3_STREAM_CREATION_ERROR` 相当でストリームエラー
   - datagram: 静かに破棄 (RFC 9297 と整合)
4. テスト: SETTINGS 未受信状態で uni stream / datagram を受信した場合の挙動を経路ごとに検証する

## 参照

- draft-ietf-webtrans-http3-15 Section 4, 7.1
- nghttp3 `lib/nghttp3_conn.c` L54
- `src/connection/mod.rs` L661, L959, L1180, L1617
