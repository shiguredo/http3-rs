# transport parameter を Connection に伝達する API を設計する

Created: 2026-04-05
Model: Opus 4.6

## 優先度

別設計課題

## 概要

WebTransport の前提条件チェック (draft-ietf-webtrans-http3-15 Section 3.1) には、HTTP/3 SETTINGS だけでなく QUIC transport parameter の確認も必要。具体的には `max_datagram_frame_size` と `reset_stream_at` の 2 つ。

現在の `Connection` は QUIC 層の transport parameter を参照する手段を持たない。QUIC 層から HTTP/3 層に transport parameter を渡す API の設計が必要。

## 根拠

- draft-ietf-webtrans-http3-15 Section 3.1 (L381-400)
  - `max_datagram_frame_size` transport parameter (値 > 0) を双方が送信する要件
  - `reset_stream_at` transport parameter (空) を双方が送信する要件
  - L434-438: サーバーが transport parameter の値が正しくない場合、全 WebTransport セッションを malformed として扱う MUST 要件
  - L440-443: クライアントも transport parameter が正しくなければセッション確立禁止の MUST NOT 要件
- `src/webtransport/connect.rs` L248-266 の `PeerCapabilities::validate()` には `max_datagram_frame_size` と `reset_stream_at` のチェックが既に定義されているが、Connection の実運用経路で呼ばれていない

## pending の理由

QUIC 層と HTTP/3 層の境界設計に関わる課題。`Connection::new()` の引数に transport parameter を追加するか、別途 setter を設けるか、あるいはコールバック経由にするかなど、設計の選択肢が複数ある。外部依存の追加はないが、API 設計の判断が必要。
