# 0074: validation.rs のインラインテストを tests/test_validation.rs に分割する

- Priority: Low
- Created: 2026-05-14
- Model: deepseek-v4-pro
- Branch: feature/refactor-move-validation-tests

## 目的

`src/validation.rs` に約 1100 行のインラインテスト (`#[cfg(test)] mod tests`) が含まれている。AGENTS.md の規約では単体テストファイルは `tests/test_<module>.rs` に配置すべきであり、`tests/test_validation.rs` が存在していない。

## 優先度根拠

Low: テストの動作自体に問題はない。AGENTS.md のファイル配置規約への準拠が目的。

## 現状

- `src/validation.rs:769-1845`: 約 1100 行のインラインテスト
- `tests/test_validation.rs`: 存在しない

## 設計方針

1. `src/validation.rs` の `#[cfg(test)] mod tests` ブロック内のテストを抽出
2. `tests/test_validation.rs` を新規作成して移動
3. `internal-test` フィーチャーで必要な pub(crate) 関数がある場合は適切にアクセス手段を確保

## 完了条件

- `src/validation.rs` からインラインテストが削除されていること
- `tests/test_validation.rs` が新規作成され全テストが pass すること
- `cargo test` が全て pass すること

## 影響範囲

- `src/validation.rs`: `#[cfg(test)] mod tests` ブロックの削除
- `tests/test_validation.rs`: 新規作成

## CHANGES.md エントリ案

```
### misc

- [UPDATE] validation.rs のインラインテストを tests/test_validation.rs に分割する
  - @担当者
```
