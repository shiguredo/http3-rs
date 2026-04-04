# WebTransport CONNECT ストリームでトレーラーを拒否する

Created: 2026-04-05
Completed: 2026-04-05
Model: Opus 4.6

## 優先度

P2

## 概要

WebTransport CONNECT ストリーム (Extended CONNECT) で 2 回目の HEADERS フレームをトレーラーとして受理している。plain CONNECT では `is_connect` フラグにより `H3_FRAME_UNEXPECTED` で拒否されるが、Extended CONNECT / WebTransport CONNECT ではこのフラグが設定されないためトレーラーが通る。

## 根拠

`set_connect()` (`src/stream/request.rs:437`) は plain CONNECT にのみ設定される (L436 コメント: "Extended CONNECT (`:protocol` 付き) はこのフラグを設定しない")。

受信側 (`src/stream/request.rs:320-323`) と送信側 (`src/stream/request.rs:148`) の両方で `is_connect` チェックにより plain CONNECT のみトレーラーを拒否している。

nghttp3 は CONNECT ストリーム上の trailers を明示的に `H3_FRAME_UNEXPECTED` にしている (`nghttp3_conn_test.c:3087`, `nghttp3_conn_test.c:3245`)。

WebTransport CONNECT では DATA フレームが Capsule Protocol を運ぶため、トレーラーに意味がない。RFC 9114 Section 4.4 は CONNECT メソッドに対して DATA フレームのみを定義しており、Extended CONNECT (RFC 9220) もトレーラーを明示的に許可していない。

## 設計判断

WebTransport CONNECT に限定するか、Extended CONNECT 全体に適用するかは設計判断が必要:

- **WebTransport CONNECT のみ**: `emit_header_events()` で WT セッション上のストリームにフラグを設定する
- **Extended CONNECT 全体**: `validate_request_headers()` の結果から `:protocol` 付き CONNECT を検出し、`set_connect()` 相当を設定する

nghttp3 の挙動に合わせるなら Extended CONNECT 全体が妥当だが、RFC 9220 に明示的な禁止文言がないため、WebTransport CONNECT に限定するのが安全。

## 対応方針

少なくとも WebTransport CONNECT ストリームで `is_connect` フラグ、または同等のトレーラー拒否メカニズムを設定する。トレーラー受信時に `H3_FRAME_UNEXPECTED` を返す。

## 解決方法

WebTransport CONNECT に限定して `set_connect()` を適用した。サーバー側は `emit_header_events()` の WT セッション登録時、クライアント側はセッション Established 遷移時にそれぞれ `set_connect()` を呼ぶ。`set_connect()` のコメントを更新し、plain CONNECT と WebTransport CONNECT の両方で使用されることを明記した。Extended CONNECT 全体への適用は見送り、WebTransport CONNECT のみに限定した。

## 参照

- RFC 9114 Section 4.4
- RFC 9220 Section 3
- `src/stream/request.rs:148` (送信側チェック)
- `src/stream/request.rs:320-323` (受信側チェック)
- `src/stream/request.rs:436-438` (set_connect)
