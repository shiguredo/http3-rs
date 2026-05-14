# 0077: src/connection/mod.rs が約 4500 行で過大 — モジュール分割が必要

Created: 2026-05-14
Model: deepseek-v4-pro

## 概要

`src/connection/mod.rs` が約 4500 行に肥大化しており、以下の責務が単一ファイルに混在している:

- HTTP/3 接続ステートマシン
- QPACK エンコーダー/デコーダーストリームの処理
- WebTransport セッション (`WtSession`) のライフサイクル管理 (約 400 行)
- WebTransport ストリーム/データグラムのバッファリングと配送
- Capsule Protocol デコードと処理
- WebTransport ドラフトバージョンネゴシエーション
- WebTransport フロー制御
- GOAWAY 処理と WT セッション伝播

CLAUDE.md L87: 「テストが長くなるのはモジュール自体が大きすぎるサインなので
`src/<module>.rs` 側の分割を検討すること」

private 関数が 40 個を超え、`emit_header_events` と `process_stream_frames`
が相互に参照し合っており変更の影響範囲把握が困難。

## 修正方針

以下のように分割を検討する:
1. `src/connection/wt_session.rs` — `WtSession` 定義と全メソッド
2. `src/connection/wt_receive.rs` — WT 受信パス
3. `src/connection/wt_dispatch.rs` — WT ストリーム dispatch と配送

## 影響範囲

- `src/connection/mod.rs` (4500 行 → 縮小)
- 新規: `src/connection/wt_session.rs`
- 新規: `src/connection/wt_receive.rs`
- 新規: `src/connection/wt_dispatch.rs`
