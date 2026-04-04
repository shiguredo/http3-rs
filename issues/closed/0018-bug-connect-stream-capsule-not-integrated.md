# CONNECT ストリーム上の Capsule Protocol が接続層に統合されていない

Created: 2026-04-05
Model: Opus 4.6

## 概要

CONNECT ストリーム上で受信したデータが `RawReceivedData::Data` → `Event::Data` に無条件変換されており、
Capsule Protocol のデコードと処理が接続層で行われていない。

`capsule.rs` の `Capsule::decode` と `session.rs` の `process_capsule` は実装済みだが、
`src/connection/` から一切呼び出されていない。

## 根拠

- `draft-ietf-webtrans-http3-15 Section 5.6`: フロー制御カプセル (WT_MAX_STREAMS, WT_MAX_DATA, WT_STREAMS_BLOCKED, WT_DATA_BLOCKED) は CONNECT ストリーム上で送受信される
- `draft-ietf-webtrans-http3-15 Section 6`: WT_CLOSE_SESSION / WT_DRAIN_SESSION も CONNECT ストリーム上のカプセル
- nghttp3 は `nghttp3_wt.c` で CONNECT ストリーム上のデータを逐次 Capsule としてパースしている
- 現状では上位アプリケーションが自力で `Event::Data` から Capsule をデコードし処理する必要があり、Sans I/O ライブラリとしての責務を果たせていない

## 影響範囲

- WT_CLOSE_SESSION によるセッション終了が接続層で検知できない
- WT_DRAIN_SESSION による draining 状態遷移が機能しない
- フロー制御カプセルが全て無視される
- 他の全ての WebTransport 接続層統合 issue の前提条件

## 必要な変更

1. CONNECT ストリーム (WebTransport セッション) 上の `RawReceivedData::Data` を検知するパスを追加する
2. 該当ストリームのデータに対して `Capsule::decode` を適用する
3. デコードされたカプセルを `Session::process_capsule` に渡す
4. カプセルの処理結果に応じたイベント生成 (セッション終了、draining 等) を行う
5. Capsule 境界をまたぐ部分受信に対応するバッファリングを実装する

## 優先度

P0 — 他の WebTransport 接続層統合 issue の前提条件であり、最優先で対応する。

## 解決方法

Completed: 2026-04-05

以下の変更で CONNECT ストリーム上の Capsule Protocol を接続層に統合した:

1. `WtSession` に `capsule_buf: Vec<u8>` フィールドを追加 — Capsule の部分受信バッファ
2. `RawReceivedData::Data` の処理で `wt_sessions` に stream_id が存在する場合は Capsule デコードパスに分岐
3. `Connection::process_wt_capsule_data()` を追加 — データをバッファに蓄積し `Capsule::decode()` で逐次デコード
4. `Connection::handle_wt_capsule()` を追加 — デコードされた Capsule をイベントに変換:
   - `WT_CLOSE_SESSION`: `terminate_wt_session()` を呼び出してセッション終了
   - `WT_DRAIN_SESSION`: `WebTransportSessionDraining` イベントを生成
   - フロー制御カプセル: 接続層で保持 (issue 0021 で結線予定)
   - 不明カプセル: 無視
5. `Event::WebTransportSessionDraining` バリアントを追加

## 参考

- `src/connection/mod.rs:1460-1462`: `RawReceivedData::Data` の現在の処理
- `src/webtransport/capsule.rs:137`: `Capsule::decode`
- `src/webtransport/session.rs:890`: `Session::process_capsule`
