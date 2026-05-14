# 0061: エンコーダーストリームでバッファ消費後にテーブル操作失敗 — データ損失

Created: 2026-05-14
Model: deepseek-v4-pro

## 概要

`src/qpack/encoder_stream.rs` の 3 メソッドにおいて、`self.recv_buffer.drain(..consumed)` を実行した**後**に
テーブル操作 (`table.insert` / `table.duplicate`) を行っている。
テーブル操作が失敗した場合、既にバッファからデータが削除されており回復不能となる。

## 対象

- `decode_insert_with_name_ref` (line 276): `drain` → `table.get_by_relative_index_encoder` → `table.insert`
- `decode_insert_with_literal_name` (line 316): `drain` → `table.insert`
- `decode_duplicate` (line 336): `drain` → `table.duplicate`

## 修正方針

テーブル操作を成功させてから `drain` を実行するように順序を入れ替える。

```rust
// decode_insert_with_name_ref の修正例
// 修正前: drain 後に name 解決 + insert
self.recv_buffer.drain(..consumed);
let name = table.get_by_relative_index_encoder(...)?.name.clone();
table.insert(name, value.clone())?;

// 修正後: name 解決 + insert 成功後に drain
let name = table.get_by_relative_index_encoder(...)?.name.clone();
table.insert(name, value.clone())?;
self.recv_buffer.drain(..consumed);
```

## 影響範囲

- `src/qpack/encoder_stream.rs:276,316,336`
- データ損失の可能性
- RFC 9204 違反 (不正な命令を検出した場合に正しくエラーを返せない)
