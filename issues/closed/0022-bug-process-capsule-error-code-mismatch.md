# process_capsule のエラーコードが仕様と不整合

Created: 2026-04-05
Model: Opus 4.6

## 概要

`Session::process_capsule()` の以下の点で draft-15 のエラーコード指定と整合していない:

1. `WT_MAX_STREAMS > 2^60` 検出時に `FlowControlError` を返しているが、draft-15 では異なるエラーコードが要求される
2. `WT_MAX_DATA` に上限チェック (`> 2^62 - 1`) がなく、`capsule.rs` 側のコメントと整合していない

## 根拠

- `draft-ietf-webtrans-http3-15 Section 5.6`: WT_MAX_STREAMS の値が `2^60` を超える場合は接続エラーとして扱う。エラーコードは `H3_MESSAGE_ERROR` が適切
- `draft-ietf-webtrans-http3-15 Section 5.6`: WT_MAX_DATA にも同様の上限制約がある
- `src/webtransport/capsule.rs:391` 付近のコメントで上限について言及されているが、`session.rs` の `process_capsule` には `WT_MAX_DATA` の上限チェックが実装されていない

## 必要な変更

1. `WT_MAX_STREAMS > 2^60` のエラーコードを draft-15 準拠のものに修正する
2. `WT_MAX_DATA` に上限チェック (`> 2^62 - 1`) を追加する
3. draft-15 原文を精査し、各カプセルの上限違反に対する正確なエラーコードを確認する

## 優先度

P2 — エラーコードの不整合は相互運用性テストで検出されるが、正常パスには影響しない。

## 解決方法

Completed: 2026-04-05

1. `H3_DATAGRAM_ERROR` 定数 (`0x33`) を `capsule.rs` に追加 — RFC 9297 Section 5.2 で定義
2. `session.rs` の `process_capsule()` で `WT_MAX_STREAMS > 2^60` 時のエラーコードを `ErrorCode::FlowControlError` から `H3_DATAGRAM_ERROR (0x33)` に修正 — draft-15 Section 5.6.2 準拠
3. `WT_MAX_DATA` の上限チェック追加は不要と判断 — draft-15 Section 5.6.4 には `WT_MAX_STREAMS` のような明示的な `2^60` 上限要件がない (varint の最大値 `2^62-1` はデコード段階で自動的に保証される)

## 参考

- `src/webtransport/session.rs:934`: `WT_MAX_STREAMS` の上限チェックとエラーコード
- `src/webtransport/session.rs:919`: `WT_MAX_DATA` の処理（上限チェックなし）
- `src/webtransport/capsule.rs:391`: 上限に関するコメント
- `refs/webtrans/rfc9297.txt:555-556`: `H3_DATAGRAM_ERROR = 0x33` の定義
