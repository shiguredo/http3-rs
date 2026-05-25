# 0070: Connection::new で QPACK max_table_capacity 未設定時に 0 になる

- Priority: Medium
- Created: 2026-05-14
- Model: deepseek-v4-pro
- Branch: feature/fix-connection-max-table-capacity

## 目的

`src/connection/mod.rs` の `Connection::new` において、`local_settings.qpack_max_table_capacity` が `None` の場合に `.unwrap_or(0)` で 0 にフォールバックしている (672行)。一方で `Limits::default()` は `DEFAULT_QPACK_MAX_TABLE_CAPACITY = 4096` を使用しており、`limits.qpack_max_table_capacity` は 4096 のまま。

この不整合により、ユーザーが `Settings::new()` (全フィールド `None`) を渡した場合に QPACK 動的テーブルが無効化される。`Limits::default()` の値にフォールバックすべき。

## 優先度根拠

Medium: QPACK 動的テーブルが無効化されるとヘッダー圧縮効率が低下する。デフォルト設定で期待通りに動作しないのは API のバグだが、データ破損や接続エラーにはつながらない。

## 現状

```rust
// connection/mod.rs:670-672
let max_table_capacity = local_settings
    .qpack_max_table_capacity
    .map(VarInt::get)
    .unwrap_or(0);
```

`Limits::default()` は `qpack_max_table_capacity: 4096` だが、QPACK セットアップでは `0` が使われる。

## 設計方針

```rust
// 修正後: Limits のデフォルト値にフォールバック
let max_table_capacity = local_settings
    .qpack_max_table_capacity
    .map(VarInt::get)
    .unwrap_or(limits.qpack_max_table_capacity);
```

`limits` は同関数内で既に構築済みであり、ユーザー指定値または `DEFAULT_QPACK_MAX_TABLE_CAPACITY` が入っている。

## テスト戦略

単体テスト: `Settings::new()` (全フィールド None) で `Connection::new` を呼び、QPACK 動的テーブルが有効（容量 4096）になっていることを確認する。

## 完了条件

- `unwrap_or(0)` が `unwrap_or(limits.qpack_max_table_capacity)` に変更されていること
- `Settings::new()` で作成した接続の QPACK 動的テーブル容量が 4096 であること
- 既存テスト (`cargo test`) が全て pass すること
- CHANGES.md にエントリが追記されていること

## 後方互換性

デフォルト動作が変更される（動的テーブル無効 → 有効）。明示的に `qpack_max_table_capacity = Some(VarInt::new(0))` を設定しているコードには影響しない。

## 影響範囲

- `src/connection/mod.rs`: 670-672行（`Connection::new` 内の QPACK セットアップ）

## RFC 根拠

- RFC 9204 Section 3.2.3: QPACK_MAX_TABLE_CAPACITY SETTINGS パラメータ — デフォルト値は 0 だが、これはピアからの通知がない場合の受信側デフォルト。ローカル設定としてはライブラリのデフォルト値（4096）を使用すべき。

## CHANGES.md エントリ案

```
- [FIX] Connection::new で QPACK max_table_capacity が未設定時に Limits のデフォルト値 (4096) にフォールバックするよう修正する
  - @担当者
```
