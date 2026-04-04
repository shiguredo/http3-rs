# 2xx 応答前の CONNECT ストリーム上 DATA を Capsule として処理してしまう

Created: 2026-04-06
Completed: 2026-04-07
Model: Opus 4.6

## 優先度

P1

## 概要

サーバー側で `RawReceivedData::Data` を受信した際、`self.wt_sessions.contains_key(&stream_id)` だけで `process_wt_capsule_data()` に分岐しており、セッション状態 (`Pending` / `Established`) を確認していない。

draft-ietf-webtrans-http3-15 では WebTransport セッションはサーバーが 2xx 応答を送信した時点で確立し、Capsule Protocol もその時点でネゴシエートされる。Pending 状態 (= サーバーがまだ 2xx を送っていない) の CONNECT ストリーム上で受信した DATA は HTTP リクエストボディとして扱うか、CONNECT 中の余剰データとしてエラーにすべきであり、Capsule デコードを適用してはならない。

## 根拠

- draft-ietf-webtrans-http3-15 Section 4.2, 5.6 (Capsule Protocol はセッション確立後)
- draft-ietf-webtrans-http3-15 L494 付近
- nghttp3 `lib/nghttp3_conn.c` L1942: established 前のセッションでは capsule 処理を block
- `src/connection/mod.rs` L2239-2247 (DATA 受信時の分岐)
- `src/connection/mod.rs` L2338-2346 (Pending セッション登録)

## 影響

- アプリが 2xx 応答を返す前に、クライアントから先行送信されたバイト列が誤って Capsule としてデコードされる
- 不正な Capsule とみなされて接続/ストリームが過剰にエラー終了する可能性
- 逆に、運悪く Capsule としてパースが通った場合に内部状態が破壊される

## 対応方針

1. `RawReceivedData::Data` の WebTransport 分岐で `wt_sessions[stream_id].state == Established` を確認する
2. Pending 状態の場合、CONNECT のリクエストボディとしての扱いを定義する。draft-15 上は CONNECT に request body は許容されないため、`H3_MESSAGE_ERROR` 相当でストリームエラーとする方針が妥当 (nghttp3 と整合)
3. `RawReceivedData::StreamEnd` 側でも同様にセッション状態を確認し、未確立セッションの FIN 取り扱いを定義する
4. テスト: Pending CONNECT ストリームに DATA を流したケース、Established 後に Capsule が正しく処理されるケース、両方をカバーする

## 参照

- draft-ietf-webtrans-http3-15 Section 4.2, 5.6
- nghttp3 `lib/nghttp3_conn.c` L1942
- `src/connection/mod.rs` L2239, L2338

## 解決方法

`src/connection/mod.rs` の `RawReceivedData::Data` 分岐で `wt_sessions.contains_key()` だけでなく `WtSession::state` を確認するように変更した。`WtSessionState::Established` の場合のみ `process_wt_capsule_data()` を呼び出し、`Pending` / `Closed` の場合は `H3_MESSAGE_ERROR` (`Error::StreamError(ErrorCode::MessageError)`) でストリームエラーにする。これにより 2xx 応答送出前の CONNECT ストリーム上の DATA が Capsule として誤デコードされる問題を防ぐ (nghttp3 と整合)。
