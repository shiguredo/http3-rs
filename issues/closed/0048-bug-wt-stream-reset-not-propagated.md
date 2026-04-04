# WebTransport データストリームの RESET_STREAM / STOP_SENDING がセッションへ伝播されない

Created: 2026-04-07
Completed: 2026-04-07
Model: Opus 4.6

## 概要

`Connection::stream_reset` および `Connection::stop_sending` は、汎用 `Event::StreamReset` / `Event::StopSending` を発行し、`terminate_wt_session(stream_id)` を CONNECT stream 前提で呼ぶだけになっている。WebTransport データストリーム (uni / bidi) の reset / stop_sending を、当該ストリームが属する WebTransport セッションへ通知していない。

draft-ietf-webtrans-http3-15 Section 4.4 / 5.4 では、既知 WebTransport セッションに属するデータストリームの reset error はアプリへ転送する必要があり、また将来の `WT_MAX_DATA` 実装には reset 済みストリームの final size をセッション側で集計する必要がある。

## 該当箇所

- `src/connection/mod.rs` `Connection::stream_reset` (現在 L3470 付近)
- `src/connection/mod.rs` `Connection::stop_sending` (現在 L3516 付近)
- `src/connection/mod.rs` `terminate_wt_session` (現在 L1944 付近)

## 根拠

- draft-ietf-webtrans-http3-15 Section 4.4: WebTransport ストリームへのリセットは WebTransport application error code に変換される
- draft-ietf-webtrans-http3-15 Section 5.4: フロー制御のため reset/stop_sending された WT データストリームの状態をセッション側が把握する必要がある
- `refs/webtrans/draft-ietf-webtrans-http3-15.txt` L767, L1104

## 修正方針

1. `stream_reset` / `stop_sending` で `stream_id` を WebTransport ストリームとして判定する。
2. WebTransport データストリームの場合:
   - 紐付いている `session_id` を解決する
   - WT application error code に変換した上で `Event::WebTransportStreamReset { session_id, stream_id, application_error_code }` を発行する
   - セッション内のストリーム集合から該当 stream_id を除去する
3. CONNECT stream (= session_id 自身) の場合のみ `terminate_wt_session` を呼ぶように分岐する。
4. 既存の `terminate_wt_session` の呼び出し位置 (汎用 reset での無条件呼び出し) を是正する。
5. 単体テストで以下を追加:
   - 既知セッションの WT bidi/uni ストリームが reset された場合、セッションは終了せず該当ストリームのみ通知される
   - CONNECT stream の reset でセッション全体が終了する

## 解決方法

- `src/event.rs` に `Event::WebTransportStreamReset` および `Event::WebTransportStreamStopSending` を追加し、`stream_id()` ヘルパでも参照可能にした。
- `src/connection/mod.rs` `Connection::stream_reset` を以下のように分岐させた:
  1. `stream_id` が `wt_sessions` のキー (= CONNECT stream) の場合 → `terminate_wt_session` を呼びセッションを終了する。
  2. `wt_uni_streams` / `wt_bidi_streams` に登録されているデータストリームの場合 → 紐づくセッションを解決し、`WtSession::associated_streams` から外した上で `WebTransportStreamReset` を発行する。
  3. それ以外 → 従来通り汎用の `Event::StreamReset` を発行する。
- `Connection::stop_sending` も同じ三分岐に書き換え、データストリームの場合は `WebTransportStreamStopSending` を発行するようにした (こちらは送信側の停止要求なので `associated_streams` は維持する)。
- 単体テストで以下を追加:
  - `test_stream_reset_propagates_to_wt_uni_data_stream`
  - `test_stream_reset_on_connect_stream_terminates_wt_session`
  - `test_stop_sending_propagates_to_wt_bidi_data_stream`

## 残課題

- WT データストリームの reset で得られる final size を WT_MAX_DATA のフロー制御に反映する処理は本 issue のスコープ外とし、フロー制御実装と合わせて別 issue で扱う。
