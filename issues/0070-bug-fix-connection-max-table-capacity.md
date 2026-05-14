# 0070: Connection::new で max_table_capacity 未設定時に 0 になる

Created: 2026-05-14
Model: deepseek-v4-pro

## 概要

`src/connection/mod.rs:666` において:

```rust
let max_table_capacity = local_settings.qpack_max_table_capacity.unwrap_or(0);
```

ユーザーが `Settings::new()` (全フィールド `None`) を渡した場合、
`qpack_max_table_capacity` が `None` のままになり動的テーブルが無効化される。
`Limits::default()` (`DEFAULT_QPACK_MAX_TABLE_CAPACITY = 4096`) にフォールバックすべき。

## 修正方針

```rust
// 修正前
let max_table_capacity = local_settings.qpack_max_table_capacity.unwrap_or(0);

// 修正後
let max_table_capacity = local_settings
    .qpack_max_table_capacity
    .unwrap_or(crate::limits::DEFAULT_QPACK_MAX_TABLE_CAPACITY);
```

## 影響範囲

- `src/connection/mod.rs:666`
- RFC 9204 Section 3.2.3
