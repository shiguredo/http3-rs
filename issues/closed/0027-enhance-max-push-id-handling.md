# control stream 上の MAX_PUSH_ID を最小限の検証付きで受理する

Created: 2026-04-05
Completed: 2026-04-07
Model: Opus 4.6

## 解決方法

`Frame` enum に `MaxPushId(u64)` バリアントを追加し、`decode_frame` で MAX_PUSH_ID を `decode_max_push_id_frame` で push ID の単一 varint としてパースするようにした。CANCEL_PUSH / PUSH_PROMISE は引き続き `ServerPushNotSupported` で拒否する。`Connection` に `max_push_id: Option<u64>` フィールドを追加し、`process_control_stream()` の `Frame::MaxPushId` 経路でロールに応じて以下のように扱う:

- サーバー: 単調増加制約 (前の値より小さい場合は `H3_ID_ERROR`) を検証してから値を保持する。サーバープッシュ非対応のため値そのものは利用しない。
- クライアント: control stream 上での MAX_PUSH_ID 受信は `H3_FRAME_UNEXPECTED`。

リクエストストリーム側 (`stream/request.rs`) でも `Frame::MaxPushId` を `H3_FRAME_UNEXPECTED` で拒否する。`encode_frame` / `encoded_frame_len` も `MaxPushId` を扱えるように更新した。デコードのラウンドトリップを確認する単体テスト `test_decode_max_push_id_frame` を追加し、既存の push 系拒否テストから MAX_PUSH_ID を取り除いた。

## 優先度

P2

## 概要

サーバーがクライアントから control stream 上で受信した `MAX_PUSH_ID` フレームを `H3_FRAME_UNEXPECTED` で拒否しているが、RFC 9114 上はサーバーが受信する正当なフレームである。単に無視するのではなく、最小限の状態検証を行った上で受理する必要がある。

## 根拠

RFC 9114 Section 7.2.7:

- `MAX_PUSH_ID` はクライアントからサーバーへ control stream 上で送信されるフレーム
- `H3_FRAME_UNEXPECTED` で拒否すべきなのはクライアントが受信した場合、または control stream 以外で受信した場合のみ
- 値は単調増加でなければならず、前の値より小さい場合は `H3_ID_ERROR` で接続を閉じる

現実装 (`src/frame/decoder.rs` L75-88) は stream 種別と role を区別せず一律に `ServerPushNotSupported` エラーを返す。

nghttp3 も push を実装していないが `MAX_PUSH_ID` は解析し、単調性検証を行っている (`nghttp3_conn.c` L916, L1180)。

## 対応方針

1. `decode_frame()` から `MAX_PUSH_ID` のエラー処理を除外し、`Frame::MaxPushId` バリアントとして正常にデコードする
2. control stream のフレーム処理で role を考慮する:
   - サーバー側: `MAX_PUSH_ID` を受理し、値の単調性を検証する（後退は `H3_ID_ERROR`）
   - クライアント側: `MAX_PUSH_ID` を受信したら `H3_FRAME_UNEXPECTED`
   - control stream 以外: `H3_FRAME_UNEXPECTED`
3. `Connection` に `max_push_id: Option<u64>` フィールドを追加し、受信した値を保持する（push 非実装のため参照はしないが、単調性検証に使用する）
4. `CANCEL_PUSH` / `PUSH_PROMISE` は現行通りエラーで拒否する（Server Push 非実装方針）

## 参照

- RFC 9114 Section 7.2.7
- `src/frame/decoder.rs` L75-88
- `src/frame/mod.rs` L28-38
- `src/stream/control.rs`
