# 0069: QPACK エンコーダーの複数の問題 (ダブルアック/Post-Base/RIC)

Created: 2026-05-14
Model: deepseek-v4-pro

## 概要

`src/qpack/encoder.rs` に 3 つの問題がある。

### 問題 1: ack_section がダブルアック時にエラーを出せない

- `encoder.rs:415-432`

RFC 9204 Section 4.4.1: 重複 Section Acknowledgment は
`QPACK_DECODER_STREAM_ERROR` で接続エラーとして扱わなければならない (MUST)。
現在の実装は `false` を返すのみで、呼び出し元が MUST レベルのエラーを検出することは
設計上保証されていない。

### 問題 2: Post-Base 参照がエンコードできず単に None を返している

- `encoder.rs:658-659`

`encode_indexed_field_dynamic` / `encode_literal_with_name_ref_dynamic` の両方で
`absolute_index >= base` の場合に `None` を返しているが、
Post-Base Indexing (`0001` prefix) を使えばエンコード可能 (RFC 9204 Section 4.5.3 / 4.5.5)。

### 問題 3: encode_required_insert_count が max_entries=0 時に固定値 1 を返す

- `encoder.rs:584-585`

容量 0 のテーブルで Required Insert Count を 1 としてエンコードしている。
デコーダー側 (`decoder.rs:436-438`) は `max_entries == 0` を `DecodeFailed` とするため、
エンコーダー/デコーダー間で不整合が生じる (RFC 9204 Section 4.5.1.1)。

## 修正方針

1. `ack_section` の戻り値を `Result<bool, QpackError>` に変更し、
   ダブルアック時は `Err` を返す
2. Post-Base エンコード (`0001` prefix) を実装する
3. `max_entries == 0` の場合は RIC を 0 としてエンコードする

## 影響範囲

- `src/qpack/encoder.rs:415-432,584-585,658-659`
- RFC 9204 Section 4.4.1, 4.5.1.1, 4.5.3, 4.5.5
