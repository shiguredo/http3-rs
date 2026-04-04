# Closed セッションで新規 WebTransport ストリームを受け入れてしまう

Created: 2026-04-05
Model: Opus 4.6

## 概要

`associate_or_buffer_stream()` が `Closed` 状態のセッションに対して `true` を返すため、
セッション終了後に到着した新規ストリームが `WebTransportUniStreamOpen` / `WebTransportBidiStreamOpen`
イベントとして上位に通知される。

コメントには「ストリームは無視」とあるが、`true` が返った後の呼び出し元コードは
ストリームを `wt_uni_streams` / `wt_bidi_streams` に登録し、Open イベントを発火する。
コメントと実装が矛盾している。

## 根拠

- `draft-ietf-webtrans-http3-15 Section 6`: セッション終了を知った後は、関連する全ストリームを `WT_SESSION_GONE` でリセットし、新規ストリームやデータグラムを送受信してはならない
- 現状の実装では Closed セッションへの新規ストリームが正常パスで処理されてしまう

## 再現手順

1. WebTransport セッションを確立する
2. CONNECT ストリームの FIN を受信してセッションが Closed に遷移する
3. その後に同一 session_id を持つ新規単方向/双方向ストリームが到着する
4. `associate_or_buffer_stream` が `true` を返す
5. `WebTransportUniStreamOpen` / `WebTransportBidiStreamOpen` イベントが発火される

## 必要な変更

`associate_or_buffer_stream()` の `Closed` 分岐で:

1. ストリームを受け入れず、`WT_SESSION_GONE` でリセットすべきことを示すイベントを生成する
2. または `false` を返して `WebTransportBufferedStreamRejected` イベントを `WT_SESSION_GONE` エラーコードで発火する

## 優先度

P1 — セッション終了後のストリーム処理は仕様違反であり、相互運用性に影響する。

## 解決方法

Completed: 2026-04-05

`associate_or_buffer_stream()` の戻り値を `bool` から `Result<bool, ()>` に変更:

- `Ok(true)`: ストリーム受け入れ (Established/Pending)
- `Ok(false)`: バッファ上限超過 (WT_BUFFERED_STREAM_REJECTED)
- `Err(())`: セッション終了済み (WT_SESSION_GONE で拒否)

呼び出し元 (uni/bidi 両方) で `Err(())` を処理し、`WebTransportBufferedStreamRejected` イベントを `WT_SESSION_GONE` エラーコードで生成するようにした。

## 参考

- `src/connection/mod.rs:1044-1056`: `associate_or_buffer_stream` の Closed 分岐
- `src/connection/mod.rs:930`: `resolve_wt_uni_stream_session_id` の Open イベント発火
- `src/connection/mod.rs:1128`: `resolve_wt_bidi_stream_header` の Open イベント発火
