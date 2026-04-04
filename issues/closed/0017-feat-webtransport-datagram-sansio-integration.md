# WebTransport Datagram の Sans I/O 接続層統合

Created: 2026-04-05
Model: Opus 4.6

## 概要

WebTransport datagram の送受信が Sans I/O の `Connection` 状態機械に統合されていない。
現状は standalone な `Datagram` codec (`src/webtransport/mod.rs`) のみが公開されており、
`Connection` 内の `WtSession::buffered_datagrams` は `#[allow(dead_code)]` で未使用。

## 根拠

- `draft-ietf-webtrans-http3-15 Section 4.5`: datagram はセッション ID に基づいてルーティングされる
- `draft-ietf-webtrans-http3-15 Section 4.6`: セッション未確立時の datagram は buffering / drop を制御する必要がある
- 現状の設計ではこれらの仕様責務が上位ラッパーに漏れており、Sans I/O 層で担保できない

## 必要な変更

1. `Connection` に datagram 受信パス (`feed_datagram`) を追加し、session_id でルーティングする
2. セッション未確立時は `WtSession::buffer_datagram()` でバッファリングする
3. セッション確立時にバッファリング済み datagram を配送する (`WebTransportDatagram` イベント)
4. バッファ上限超過時は datagram を破棄する
5. `Connection` に datagram 送信パス (`send_datagram`) を追加する

## 補足

- セッション終了後の datagram 送信は draft-15 Section 6 で禁止されており、接続層で拒否する必要がある
- `Section 4.5` の検証 (session_id の妥当性) も接続層で行うべき
- ライブラリは WebTransport サポートを謳っているが、datagram 経路が存在しないのは機能的な欠落

## 優先度

P0 — ストリーム側の先行到着処理 (Section 4.6) と同じ設計パターンを適用する。
WebTransport の基本機能として Capsule 統合 (issue 0018) と並行して対応する。

## 解決方法

Completed: 2026-04-05

以下の変更で WebTransport Datagram を Sans I/O 接続層に統合した:

1. `Event::WebTransportDatagram { session_id, payload }` バリアントを追加
2. `Connection::feed_datagram(data)` を追加 — QUIC DATAGRAM ペイロードを受信し、session_id でルーティング:
   - Established: `WebTransportDatagram` イベントを生成
   - Pending: `WtSession::buffer_datagram()` でバッファリング
   - Closed: 破棄 (Section 6)
   - 未登録: 新規 Pending セッションを作成してバッファリング
3. `Connection::send_datagram(session_id, payload)` を追加 — Established セッションのみ送信許可
4. セッション確立時 (クライアント/サーバー両方) にバッファ済みデータグラムを配送
5. `WtSession` の `#[allow(dead_code)]` を除去 (`buffered_datagrams`, `buffer_datagram`, `take_buffered_datagrams`)
6. `ClientConnection` / `ServerConnection` にラッパーメソッドを追加

## 参考

- `src/connection/mod.rs`: `WtSession::buffered_datagrams` (dead code)
- `src/webtransport/mod.rs`: `Datagram` codec
- nghttp3: `nghttp3_conn` が datagram の session_id routing を担当
