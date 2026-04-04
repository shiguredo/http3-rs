# WebTransport 単方向ストリーム 0x54 がコア接続層で破棄される

Created: 2026-04-05
Model: Opus 4.6

## 概要

`UniStreamType::WebTransport = 0x54` が定義されているにもかかわらず、`Connection` の単方向ストリーム受信分岐 (`src/connection/mod.rs:549`) では `0x00 / 0x01 / 0x02 / 0x03` のみを処理し、`0x54` は `_ =>` で `ignored_uni_streams` に入り黙って破棄される。

## 根拠

- draft-ietf-webtrans-http3-15 Section 4.2: HTTP/3 単方向ストリームタイプ `0x54` を WebTransport ストリームとして明示的に規定
- nghttp3 は `NGHTTP3_STREAM_TYPE_WT_STREAM` として専用分岐 (`nghttp3_conn.c:708`) と専用読み取り処理 (`nghttp3_conn.c:803`) を持つ

## 問題

コア層が `0x54` を未知ストリームとして無視するため、WebTransport 単方向ストリームのデータが全て失われる。上位ラッパーで補っていたとしても、Sans I/O コアとしては不足。

## 対応方針

- `Connection` の単方向ストリーム受信分岐に `0x54` の専用処理を追加する
- ストリームタイプ後の session ID (varint) をパースし、イベントとして上位に通知する
- WebTransport が無効な接続で `0x54` を受信した場合は `H3_STREAM_CREATION_ERROR` を返す (nghttp3 と同様)

Completed: 2026-04-05

## 解決方法

- `Connection` に `wt_uni_streams` (確定済み WT ストリーム → セッション ID) と `pending_wt_uni_streams` (セッション ID 未確定バッファ) を追加した
- `handle_new_unidirectional_stream()` に `0x54` の専用分岐を追加し、WebTransport 無効時は `StreamCreationError` を返すようにした
- `handle_unidirectional_stream()` で既知の WT ストリームへのデータ受信を処理するようにした
- セッション ID の varint が複数チャンクにまたがる場合のバッファリングを実装した
- `Event::WebTransportUniStreamOpen` / `WebTransportUniStreamData` / `WebTransportUniStreamEnd` イベントを追加した
- FIN 受信時に適切にストリームをクリーンアップしイベントを生成するようにした
