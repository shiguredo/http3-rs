# Huffman デコードで EOS シンボルをエラーではなく Ok で返している

Created: 2026-05-14
Model: deepseek-v4-pro

## 概要

`src/qpack/huffman.rs:1129-1131` において、Huffman デコード中に EOS シンボル (256) にマッチした場合、誤って `Ok(result)` を返している。

EOS シンボルが正当な Huffman 符号化データに含まれることは許容されない (RFC 9204 Section 4.1.2 が参照する RFC 7541 Section 5.2)。この規定により、EOS を含む文字列全体が無効であり、検出時点でエラーを返さなければならない (MUST)。

## 再現手順

EOS シンボル (30-bit code `0xfffffffc`) を含むバイト列をデコードする。例: `[0xff, 0xff, 0xff, 0xc0]` (EOS プレフィックスを含む 4 バイト)。

期待される動作: `Err(QpackError::InvalidHuffman)` が返ること。
実際の動作: `Ok(result)` が返され不正データが受け入れられる。

## 修正内容

```rust
// 修正前 (src/qpack/huffman.rs:1129-1131)
if sym_idx == 256 {
    // EOS symbol - should not appear in valid data
    return Ok(result);
}

// 修正後
if sym_idx == 256 {
    // EOS シンボルは正当なデータに出現してはならない
    // (RFC 9204 Section 4.1.2, RFC 7541 Section 5.2)
    return Err(QpackError::InvalidHuffman);
}
```

EOS 検出時に `result` を破棄するのは RFC 7541 Section 5.2 の「文字列全体が無効」という規定に従っている。

## テスト

単体テストを `src/qpack/huffman.rs` に追加する:

```rust
#[test]
fn decode_eos_returns_error() {
    // EOS シンボル (index 256, 30-bit code 0xfffffffc) を埋め込んだバイト列
    let eos_data = [0xff, 0xff, 0xff, 0xfc];
    assert!(decode(&eos_data).is_err());
}
```

既存のインラインテスト (L1169-1218) は正常データのみで構成されているため、この修正で壊れない。

## 影響範囲

- `src/qpack/huffman.rs:1129-1131`
- セキュリティ: 悪意ある入力による不正データ受け入れを防止
- RFC 7541 Section 5.2 違反を修正
