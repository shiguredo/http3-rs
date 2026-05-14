# 0079: QPACK 整数エンコード/デコードの重複実装を一本化する

Created: 2026-05-14
Model: deepseek-v4-pro

## 概要

RFC 7541 Section 5.1 のプレフィックス整数エンコード/デコードが 4 ファイルで計 6 実装存在する。

- `src/qpack/encoder.rs:232-270` — `Encoder::encode_integer` (メソッド)
- `src/qpack/encoder.rs:826-858` — `encode_integer_to_buf` (フリー関数、同一ロジック)
- `src/qpack/encoder_stream.rs:355-370` — `encode_integer` (フリー関数、同一ロジック)
- `src/qpack/decoder.rs:263-299` — `Decoder::decode_integer` (メソッド)
- `src/qpack/decoder.rs:703-739` — `decode_integer` (フリー関数、同一ロジック)
- `src/qpack/decoder_stream.rs:248-284` — `decode_integer` (フリー関数、同一ロジック)

## 修正方針

`src/qpack/integer.rs` を新設し、全エンコード/デコード実装を集約する。

```rust
// src/qpack/integer.rs
pub fn encode_integer(buf: &mut [u8], value: u64, prefix_bits: u8) -> Option<usize>;
pub fn decode_integer(buf: &[u8], prefix_bits: u8) -> Option<(u64, usize)>;
```

## 影響範囲

- 新規: `src/qpack/integer.rs`
- `src/qpack/encoder.rs`: 重複実装削除、`integer::*` を使用
- `src/qpack/decoder.rs`: 同上
- `src/qpack/encoder_stream.rs`: 同上
- `src/qpack/decoder_stream.rs`: 同上
