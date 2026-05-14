# 0075: CapsuleValidationError::MaxDataExceedsLimit の発生経路が未実装

Created: 2026-05-14
Model: deepseek-v4-pro

## 概要

`src/webtransport/capsule.rs:53-54` に `CapsuleValidationError::MaxDataExceedsLimit` が
定義されているが、上限チェックの実装が欠落している。

- `capsule.rs:442-447` `validate_max_data()`: 減少チェックのみで上限チェックがない
- `session.rs:919-924` `process_capsule` の `MaxData` 分岐: 減少チェックのみ
- `DirectionalStreamFlowControl` の `MAX_STREAMS_LIMIT` キャップとは非対称

コードコメントには「H3_DATAGRAM_ERROR として扱う」と明記されているが未実装。

## 修正方針

`validate_max_data` に上限チェック (`2^62-1`) を追加し、
`process_capsule` からの呼び出しで適用する。

```rust
// validate_max_data に上限チェック追加
if maximum > (1u64 << 62) - 1 {
    return Err(CapsuleValidationError::MaxDataExceedsLimit);
}
```

## 影響範囲

- `src/webtransport/capsule.rs:442-447`
- `src/webtransport/session.rs:919-924`
- draft-ietf-webtrans-http3-15 Section 5.6
