# 0075: CapsuleValidationError::MaxDataExceedsLimit の発生経路が未実装

- Priority: Medium
- Created: 2026-05-14
- Model: deepseek-v4-pro
- Branch: feature/fix-max-data-exceeds-limit

## 目的

`src/webtransport/capsule.rs:63` に `CapsuleValidationError::MaxDataExceedsLimit` が定義されているが、`validate_max_data` 関数 (462行) は減少チェックのみで上限チェック (`2^62-1`) が未実装。エラーバリアントが定義されているのに発生経路がない状態は、フロー制御の安全性を損なう。

## 優先度根拠

Medium: VarInt の値域上限 (`2^62-1`) を超える MAX_DATA 値が受信された場合にバリデーションが効かない。実運用で `2^62-1` を超える値が送信される可能性は低いが、防御的検証として実装すべき。

## 現状

```rust
// capsule.rs:462-467
pub fn validate_max_data(maximum: u64, current_max: u64) -> Result<(), CapsuleValidationError> {
    if maximum < current_max {
        return Err(CapsuleValidationError::MaxDataDecreased);
    }
    Ok(())
}
```

上限チェックがなく、`MaxDataExceedsLimit` は到達不能コード。

## 設計方針

`validate_max_data` に上限チェックを追加する。VarInt の最大値 (`2^62-1`) を超える場合は `MaxDataExceedsLimit` を返す。

```rust
// 修正後
pub fn validate_max_data(maximum: u64, current_max: u64) -> Result<(), CapsuleValidationError> {
    if maximum > (1u64 << 62) - 1 {
        return Err(CapsuleValidationError::MaxDataExceedsLimit);
    }
    if maximum < current_max {
        return Err(CapsuleValidationError::MaxDataDecreased);
    }
    Ok(())
}
```

## テスト戦略

単体テスト: `validate_max_data` に `2^62` 以上の値を渡して `MaxDataExceedsLimit` が返ることを確認。既存テスト (`capsule.rs:572-579`) に追加する。

## 完了条件

- `validate_max_data` に上限チェックが実装されていること
- `MaxDataExceedsLimit` のテストが pass すること
- 既存テスト (`cargo test`) が全て pass すること

## 影響範囲

- `src/webtransport/capsule.rs`: `validate_max_data` 関数

## RFC 根拠

- draft-ietf-webtrans-http3-15 Section 5.6: フロー制御の MAX_DATA / MAX_STREAMS — VarInt 値域内であるべき

## CHANGES.md エントリ案

```
- [FIX] validate_max_data に VarInt 上限チェックを追加し MaxDataExceedsLimit エラーの発生経路を実装する
  - @担当者
```
