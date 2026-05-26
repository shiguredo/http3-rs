# 0079: QPACK 整数エンコード/デコードの重複実装を一本化する

- Priority: Low
- Created: 2026-05-14
- Completed: 2026-05-26
- Polished: 2026-05-26
- Model: deepseek-v4-pro
- Branch: feature/refactor-unify-qpack-integer-codec

## 目的

RFC 7541 Section 5.1 (Integer Representation) のプレフィックス整数エンコード/デコードが 4 ファイルで計 8 実装存在する。同一ロジックの重複であり、バグ修正時に全箇所を修正し忘れるリスクがある。

## 優先度根拠

Low: 現状で動作に問題はないが、DRY 原則違反であり保守性を損なっている。

## 現状

### encode 系 (4 実装、2 種類のシグネチャ)

| ファイル | 関数 | シグネチャ |
|----------|------|------------|
| `src/qpack/encoder.rs:216` | `Encoder::encode_integer` | `(&self, &mut [u8], u64, u8, u8) -> Option<usize>` (スライス版、`self` 未使用) |
| `src/qpack/encoder.rs:849` | `encode_integer_to_buf` | `(&mut [u8], u64, u8, u8) -> Option<usize>` (スライス版) |
| `src/qpack/encoder_stream.rs:355` | `encode_integer` | `(&mut Vec<u8>, u64, u8, u8)` (Vec push 版) |
| `src/qpack/decoder_stream.rs:230` | `encode_integer` | `(&mut Vec<u8>, u64, u8, u8)` (Vec push 版) |

### decode 系 (4 実装、同一シグネチャ)

| ファイル | 関数 | シグネチャ |
|----------|------|------------|
| `src/qpack/decoder.rs:253` | `Decoder::decode_integer` | `(&self, &[u8], u8) -> Result<(u64, usize), QpackError>` (`self` 未使用) |
| `src/qpack/decoder.rs:679` | `decode_integer` | `(&[u8], u8) -> Result<(u64, usize), QpackError>` |
| `src/qpack/encoder_stream.rs:401` | `decode_integer` | `(&[u8], u8) -> Result<(u64, usize), QpackError>` |
| `src/qpack/decoder_stream.rs:248` | `decode_integer` | `(&[u8], u8) -> Result<(u64, usize), QpackError>` |

### 備考

- `Encoder::encode_integer` と `Decoder::decode_integer` はメソッドだが `self` を一切使用していない。統合時にフリー関数化して問題ない
- 全 decode 実装でオーバーフロー保護閾値 `shift > 56` が統一されていることを確認済み (RFC 9204 Section 4.1.1 の「62 ビットまでデコード可能」要件を満たす)
- `encode_string` / `encode_string_with_prefix` / `decode_string` にも同様の重複があるが、本 issue のスコープは整数コーデックのみに限定する。文字列コーデックの統合は別 issue として扱う

## 設計方針

`src/qpack/integer.rs` を新設し、以下の 2 つのインターフェースを提供する:

```rust
// src/qpack/integer.rs

/// スライスにエンコード (RFC 7541 Section 5.1)
///
/// encoder.rs で使用。事前確保バッファに書き込み、バッファ不足時は None を返す。
pub(crate) fn encode_integer(
    buf: &mut [u8],
    value: u64,
    prefix_bits: u8,
    prefix: u8,
) -> Option<usize>;

/// Vec にエンコード (RFC 7541 Section 5.1)
///
/// encoder_stream.rs / decoder_stream.rs で使用。Vec に push で追記する。
pub(crate) fn encode_integer_to_vec(
    buf: &mut Vec<u8>,
    value: u64,
    prefix_bits: u8,
    prefix: u8,
);

/// デコード (RFC 7541 Section 5.1)
///
/// 全ファイルで共通。shift > 56 でオーバーフロー保護
/// (RFC 9204 Section 4.1.1: 62 ビットまでデコード可能であること)。
pub(crate) fn decode_integer(
    data: &[u8],
    prefix_bits: u8,
) -> Result<(u64, usize), QpackError>;
```

encode はスライス版と Vec 版でセマンティクスが異なる (バッファ不足の扱い) ため、2 関数に分ける。ロジック本体は共通の内部関数で実装し、出力先の差異だけをラッパーで吸収する方式でもよい。

## テスト戦略

- 既存の PBT (`pbt/tests/prop_qpack.rs` およびサブモジュール) がラウンドトリップを検証しているため、統合後もこれが pass すれば正しさは担保される
- `integer.rs` 専用の PBT を `pbt/tests/prop_qpack/` 配下にサブモジュールとして追加する (CLAUDE.md のテスト配置規約に準拠)
  - strategy: `prefix_bits` を 1..=8 の範囲で生成、`value` を 0..=`u64::MAX` の範囲で生成、`prefix` を `(0xFF >> prefix_bits) << prefix_bits` の範囲でマスク生成
  - プロパティ: encode → decode のラウンドトリップで値が一致すること
- 境界値テスト (PBT で到達しにくいケース): 空バッファ、`prefix_bits` の上下限、`shift > 56` のオーバーフロー
- `fuzz/fuzz_targets/` に decode_integer のパニック安全性テストが既にあるか確認し、なければ追加する

## 完了条件

- 8 箇所の重複実装が全て削除され `integer.rs` に一本化されていること
- 統合後の実装が RFC 7541 Section 5.1 の MUST 要件 (実装制限を超えるエンコードはデコードエラー) を満たすこと
- 統合後の実装が RFC 9204 Section 4.1.1 の MUST 要件 (62 ビットまでデコード可能) を満たすこと
- `cargo test` が全て pass すること
- 相互運用テストが pass すること

## 影響範囲

- 新規: `src/qpack/integer.rs`
- 修正: `src/qpack/encoder.rs`, `src/qpack/decoder.rs`, `src/qpack/encoder_stream.rs`, `src/qpack/decoder_stream.rs`
- テスト追加: `pbt/tests/prop_qpack/` 配下に整数コーデック PBT サブモジュール

## RFC 根拠

- refs/rfc7541.txt Section 5.1 "Integer Representation": プレフィックス整数表現のエンコード/デコードアルゴリズム
- refs/h3/rfc9204.txt Section 4.1.1 "Prefixed Integers": QPACK が RFC 7541 Section 5.1 を unmodified で参照。 62 ビットまでのデコード MUST 要件

## 解決方法

`src/qpack/integer.rs` を新設し、4 ファイル (encoder.rs, decoder.rs, encoder_stream.rs, decoder_stream.rs) に散在していた 8 箇所の整数エンコード/デコード実装を 3 関数 (`encode_integer`, `encode_integer_to_vec`, `decode_integer`) に統合した。

### 変更内容

- `src/qpack/integer.rs` を新設し、スライス版エンコード・Vec 版エンコード・デコードの 3 関数を配置
- `src/qpack/mod.rs` にモジュール宣言を追加
- `src/qpack/encoder.rs` から `Encoder::encode_integer` メソッドと `encode_integer_to_buf` フリー関数を削除し、`integer::encode_integer` に置換
- `src/qpack/decoder.rs` から `Decoder::decode_integer` メソッドと `decode_integer` フリー関数を削除し、`integer::decode_integer` に置換
- `src/qpack/encoder_stream.rs` から `encode_integer` と `decode_integer` フリー関数を削除し、`integer::encode_integer_to_vec` / `integer::decode_integer` に置換
- `src/qpack/decoder_stream.rs` から `encode_integer` と `decode_integer` フリー関数を削除し、`integer::encode_integer_to_vec` / `integer::decode_integer` に置換
- `pbt/tests/prop_qpack.rs` を `pbt/tests/prop_qpack/main.rs` に変換し、`integer.rs` サブモジュールとして整数コーデック専用の PBT と境界値テストを追加

### テスト

- PBT: encode -> decode ラウンドトリップ (スライス版・Vec 版)、スライス版と Vec 版の一致、1 バイトエンコーディング
- 境界値テスト: 空バッファ、0 値、max_prefix 境界、バッファ不足、オーバーフロー保護、不完全多バイト、最大デコード可能値

## CHANGES.md エントリ案

```markdown
### misc

- [UPDATE] QPACK 整数エンコード/デコードの重複実装を src/qpack/integer.rs に一本化する
  - @voluntas
```
