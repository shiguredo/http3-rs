# 0074: validation.rs のインラインテストを tests/test_validation.rs に分割する

Created: 2026-05-14
Model: deepseek-v4-pro

## 概要

`src/validation.rs:769-1845` に約 1100 行のインラインテストが `#[cfg(test)] mod tests`
に含まれている。CLAUDE.md L82 に従い、単体テストは `tests/test_<module>.rs` に
配置すべきであり、現在 `tests/test_validation.rs` が存在していない。

## 修正方針

1. `src/validation.rs` の `#[cfg(test)] mod tests` ブロックからテストを抽出
2. `tests/test_validation.rs` に移動する

## 影響範囲

- `src/validation.rs:769-1845` (削除)
- `tests/test_validation.rs` (新規)
