# 0061: エンコーダーストリームでバッファ消費後にテーブル操作 — 処理順序の誤り

Created: 2026-05-14
Model: deepseek-v4-pro

## 概要

`src/qpack/encoder_stream.rs` の 3 メソッド (`decode_insert_with_name_ref`、`decode_insert_with_literal_name`、`decode_duplicate`) において、`self.recv_buffer.drain(..consumed)` を実行した**後**にテーブル操作 (`table.insert` / `table.duplicate`) を行っている。

テーブル操作が失敗した場合、既にバッファからデータが削除されているため、エラー原因の解析に必要な受信データが失われる（フェイルセーフ原則違反）。

現在の実装では `connection/mod.rs:process_encoder_stream` がエラーを `QpackEncoderStreamError` に変換して接続を閉じるため、実際の HTTP データ損失には至らない。しかし処理順序として誤っており、将来のコード変更で問題が顕在化するリスクがある。

なお `decode_set_capacity` にも同様の drain → table.set_capacity パターンがあるが、`set_capacity` は infallible であるため修正対象外。

## 再現手順

### ケース 1: 不正な relative_index で Duplicate

```rust
let mut receiver = EncoderStreamReceiver::new();
receiver.set_max_table_capacity(4096);
let mut table = DynamicTable::with_capacity(4096);
table.insert(b"name".to_vec(), b"value".to_vec()); // abs=0

// Duplicate with relative_index=5 (存在しないインデックス)
// 命令: 00000101 (5-bit prefix, value=5)
receiver.receive(&[0x05]);

// process は Err(QpackError::InvalidIndex(5)) を返すが、
// 現在の実装では drain 後に duplicate が呼ばれるため、
// recv_buffer.data は既に空。エラー発生時のバッファ状態を確認できない。
let result = receiver.process(&mut table);
assert_eq!(result, Err(QpackError::InvalidIndex(5)));
assert!(receiver.buffer().is_empty()); // ← バッファが空
```

### ケース 2: 容量オーバーで Insert 失敗

```rust
let mut receiver = EncoderStreamReceiver::new();
receiver.set_max_table_capacity(4096);
let mut table = DynamicTable::with_capacity(40); // 小容量

// Insert with Literal Name: 01 prefix + name + value
// エントリサイズが capacity を超える場合、insert は None を返す
// エンコード列 (容量計算は省略)
receiver.receive(&encoded_data);
let result = receiver.process(&mut table);
// Err(QpackError::DecodeFailed) が返るが、バッファは既に drain 済み
```

## 対象

| メソッド | 行 | 現象 |
|----------|-----|------|
| `decode_insert_with_name_ref` | 276 | `drain` → `table.get_by_relative_index_encoder` → `table.insert` |
| `decode_insert_with_literal_name` | 316 | `drain` → `table.insert` |
| `decode_duplicate` | 336 | `drain` → `table.duplicate` |

`decode_set_capacity` (256) は `set_capacity` が infallible のため修正対象外。

## 修正方針

テーブル操作を成功させてから `drain` を実行するように順序を入れ替える。

### decode_insert_with_name_ref の修正

```rust
// 修正前:
self.recv_buffer.drain(..consumed);

let name = if is_static {
    STATIC_TABLE
        .get(name_index as usize)
        .ok_or(QpackError::InvalidIndex(name_index))?
        .name
        .to_vec()
} else {
    table
        .get_by_relative_index_encoder(name_index)
        .ok_or(QpackError::InvalidIndex(name_index))?
        .name
        .clone()
};

table
    .insert(name, value.clone())
    .ok_or(QpackError::DecodeFailed)?;

// 修正後: テーブル操作を先に行い、成功後に drain
let name = if is_static {
    STATIC_TABLE
        .get(name_index as usize)
        .ok_or(QpackError::InvalidIndex(name_index))?
        .name
        .to_vec()
} else {
    table
        .get_by_relative_index_encoder(name_index)
        .ok_or(QpackError::InvalidIndex(name_index))?
        .name
        .clone()
};

table
    .insert(name, value.clone())
    .ok_or(QpackError::DecodeFailed)?;

self.recv_buffer.drain(..consumed);
```

### decode_insert_with_literal_name の修正

```rust
// 修正前:
self.recv_buffer.drain(..consumed);
table
    .insert(name.clone(), value.clone())
    .ok_or(QpackError::DecodeFailed)?;

// 修正後:
table
    .insert(name.clone(), value.clone())
    .ok_or(QpackError::DecodeFailed)?;
self.recv_buffer.drain(..consumed);
```

### decode_duplicate の修正

```rust
// 修正前:
self.recv_buffer.drain(..consumed);
table
    .duplicate(relative_index)
    .ok_or(QpackError::InvalidIndex(relative_index))?;

// 修正後:
table
    .duplicate(relative_index)
    .ok_or(QpackError::InvalidIndex(relative_index))?;
self.recv_buffer.drain(..consumed);
```

## テスト戦略

- **単体テスト**: 再現手順の 2 ケースを含むエラーパスを `tests/test_encoder_stream.rs` に追加する。エラー発生時に `receiver.buffer()` が空になっていないことを検証する。
- **PBT**: 既存の `pbt/tests/prop_qpack.rs` にエラーパスプロパティを追加し、不正入力に対してエラーが返るときバッファが消費されていないことを検証する。
- **Fuzzing**: 不要（エラーパスは単体テストでカバー）。

## 後方互換性

外部 API に変更なし。`process()` の戻り値型 (`Result<Option<EncoderInstruction>, QpackError>`) は変更されない。エラー発生時の `recv_buffer` 状態が変わるが、エラー時は接続が閉じられるため互換性に影響しない。

## 影響範囲

- `src/qpack/encoder_stream.rs:276,316,336`
- エラー発生時の受信バッファ状態が変化（空 → 未消費）
- 接続エラー処理 (`connection/mod.rs:2333-2342`) への影響なし

## CHANGES.md エントリ案

```
- [FIX] エンコーダーストリームレシーバーでバッファ消費後にテーブル操作が失敗した場合、
  バッファデータが失われる問題を修正する。テーブル操作成功後に drain するよう順序を入れ替え。
  - @担当者
```
