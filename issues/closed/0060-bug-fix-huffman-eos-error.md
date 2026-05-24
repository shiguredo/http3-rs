# Huffman デコードで EOS シンボルをエラーではなく Ok で返している

Created: 2026-05-14
Completed: 2026-05-24
Model: deepseek-v4-pro

## 概要

`src/qpack/huffman.rs:1129-1131` において、Huffman デコード中に EOS シンボル (index 256) にマッチした場合、`return Err(QpackError::InvalidHuffman)` ではなく `return Ok(result)` を返している。

RFC 7541 Section 5.2 は以下の MUST 規定を定めている:

> A Huffman-encoded string literal containing the EOS symbol MUST be treated as a decoding error.

同セクションにはパディングに関する 2 つの MUST 規定も存在するが、それらは既に正しく実装されている (`src/qpack/huffman.rs:1151-1164`)。EOS シンボル検出のみが未対応である。

QPACK (RFC 9204 Section 4.1.2) は RFC 7541 Section 5.2 の文字列リテラル定義を使用し、Huffman 符号化に関する規定を変更していないため、この MUST 規定は HTTP/3 の QPACK にも適用される。

## 再現手順

EOS シンボルの Huffman 符号 (RFC 7541 Appendix B: 30 ビット, 値 `0x3fffffff`) を含むバイト列をデコードする。

例: `[0xff, 0xff, 0xff, 0xff]`
- 上位 30 ビットが EOS 符号 (`0x3fffffff` を左詰め 32 ビットにすると `0xfffffffc`)
- 下位 2 ビット `11` は正当なパディング (EOS の最上位ビット)

期待される動作: `Err(QpackError::InvalidHuffman)` が返ること。
実際の動作: `Ok(vec![])` が返され不正データが受け入れられる。

## 修正内容

`src/qpack/huffman.rs:1131` の `return Ok(result)` を `return Err(QpackError::InvalidHuffman)` に変更する。

既存の英語コメント (`// EOS symbol - should not appear in valid data`) も日本語に変更する。

## 影響範囲

`huffman::decode` の呼び出し元は以下の 5 箇所であり、いずれも `?` 演算子で `QpackError` を伝播している。戻り値の型 `Result<Vec<u8>, QpackError>` は変わらないため API 互換性に影響はない。

- `src/qpack/decoder.rs:222, 244, 734, 755`
- `src/qpack/encoder_stream.rs:468`

## テスト

意図的なエラーパスの検証であり、単体テストで対応する。`tests/test_qpack.rs` を新規作成して配置する。

検証すべきケース:

- EOS のみを含むバイト列 (`[0xff, 0xff, 0xff, 0xff]`) がエラーを返すこと
- 有効なシンボルの後に EOS が出現するバイト列がエラーを返すこと
  - 例: `[0x1f, 0xff, 0xff, 0xff, 0xff]` (`'a'` (5 ビット) + EOS (30 ビット) + パディング (5 ビット, 全て 1 で正当))
  - 修正前は `Ok(vec![b'a'])` を返す最も危険なパターン

## CHANGES.md

`## develop` セクションに以下を追記する:

- [FIX] Huffman デコードで EOS シンボル検出時に `Ok` を返していた RFC 7541 Section 5.2 違反を修正し `Err(QpackError::InvalidHuffman)` を返すようにする
  - @voluntas

## 解決方法

`src/qpack/huffman.rs:1131` の `return Ok(result)` を `return Err(QpackError::InvalidHuffman)` に変更した。英語コメントも日本語に変更した。`tests/test_qpack.rs` を新規作成し、EOS のみ・有効シンボル+EOS の 2 ケースの単体テストを追加した。
