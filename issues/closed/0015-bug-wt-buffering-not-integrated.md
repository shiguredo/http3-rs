# 未確立セッション向け stream/datagram の buffering が Connection に統合されていない

Created: 2026-04-05
Completed: 2026-04-05
Model: Opus 4.6

## 優先度

P2

## 概要

`Session` 側に Section 4.6 用の buffering API が実装済みだが、`Connection` 側にはセッション表がなく、受信時のバッファリングとリジェクトが live path に存在しなかった。

## 根拠

- draft-ietf-webtrans-http3-15 Section 4.6 (L862-870)

## 解決方法

1. `WtSession` にバッファリング機能 (`buffer_stream`, `take_buffered_streams`, `buffer_datagram`, `take_buffered_datagrams`) を実装した
2. `WT_MAX_BUFFERED_STREAMS` (100) と `WT_MAX_BUFFERED_DATAGRAMS` (100) の上限定数を定義した
3. `associate_or_buffer_stream` メソッドを追加:
   - Established セッション: ストリームを関連付ける
   - Pending セッション: バッファリングする (上限超過時は `false` を返す)
   - 未登録セッション: 新規 Pending セッションを作成してバッファリング
4. バッファ上限超過時は `WebTransportBufferedStreamRejected` イベントを発火し、呼び出し側が `WT_BUFFERED_STREAM_REJECTED` で RESET_STREAM / STOP_SENDING を送信する
5. セッション確立時にバッファされたストリームを関連付ける
6. datagram のバッファリングは構造のみ実装済み (datagram 受信 API が未実装のため `#[allow(dead_code)]`)
