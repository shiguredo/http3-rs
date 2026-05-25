# 0079: QPACK 整数エンコード/デコードの重複実装を一本化する

- Priority: Low
- Created: 2026-05-14
- Model: deepseek-v4-pro
- Branch: feature/refactor-unify-qpack-integer-codec

## 目的

RFC 7541 Section 5.1 のプレフィックス整数エンコード/デコードが 4 ファイルで計 6 実装存在する。同一ロジックの重複であり、バグ修正時に全箇所を修正し忘れるリスクがある。

## 優先度根拠

Low: 現状で動作に問題はないが、DRY 原則違反であり保守性を損なっている。

## 現状

| ファイル | 関数 | 種別 |
|----------|------|------|
| `src/qpack/encoder.rs:232-270` | `Encoder::encode_integer` | メソッド |
| `src/qpack/encoder.rs:826-858` | `encode_integer_to_buf` | フリー関数 |
| `src/qpack/encoder_stream.rs:355-370` | `encode_integer` | フリー関数 |
| `src/qpack/decoder.rs:263-299` | `Decoder::decode_integer` | メソッド |
| `src/qpack/decoder.rs:703-739` | `decode_integer` | フリー関数 |
| `src/qpack/decoder_stream.rs:248-284` | `decode_integer` | フリー関数 |

## 設計方針

`src/qpack/integer.rs` を新設し、全エンコード/デコード実装を集約する。

```rust
// src/qpack/integer.rs
pub(crate) fn encode_integer(buf: &mut [u8], value: u64, prefix_bits: u8) -> Option<usize>;
pub(crate) fn decode_integer(buf: &[u8], prefix_bits: u8) -> Result<(u64, usize), QpackError>;
```

各ファイルの重複実装を削除し、`integer::encode_integer` / `integer::decode_integer` を使用する。

## テスト戦略

既存の PBT (`pbt/tests/prop_qpack.rs`) がラウンドトリップを検証しているため、統合後もこれが pass すれば正しさは担保される。追加で `integer.rs` 専用の境界値テストを作成する。

## 完了条件

- 重複実装が全て削除され `integer.rs` に一本化されていること
- `cargo test` が全て pass すること
- 相互運用テストが pass すること

## 影響範囲

- 新規: `src/qpack/integer.rs`
- 修正: `src/qpack/encoder.rs`, `src/qpack/decoder.rs`, `src/qpack/encoder_stream.rs`, `src/qpack/decoder_stream.rs`

## RFC 根拠

- RFC 7541 Section 5.1: プレフィックス整数表現
- RFC 9204 Section 4.1.1: QPACK が RFC 7541 Section 5.1 を参照

## CHANGES.md エントリ案

```
### misc

- [UPDATE] QPACK 整数エンコード/デコードの重複実装を src/qpack/integer.rs に一本化する
  - @担当者
```
