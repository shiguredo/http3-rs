# 0060: Huffman デコードで EOS シンボルをエラーではなく Ok で返している

Created: 2026-05-14
Model: deepseek-v4-pro

## 概要

`src/qpack/huffman.rs:1129-1131` において、Huffman デコード中に EOS シンボル (256) にマッチした場合、
誤って `Ok(result)` を返している。RFC 7541 Section 5.2 は「デコーダーは EOS シンボルをエラーとして扱わなければならない (MUST)」と規定している。

## 再現手順

1. EOS シンボルを含む不正な Huffman 符号化データをデコードする
2. `Ok(result)` が返され、不正データが受け入れられる

## 修正方針

```rust
// 修正前
if sym_idx == 256 {
    // EOS symbol - should not appear in valid data
    return Ok(result);
}

// 修正後
if sym_idx == 256 {
    // EOS シンボルは正当なデータに出現してはならない (RFC 7541 Section 5.2)
    return Err(QpackError::InvalidHuffman);
}
```

## 影響範囲

- `src/qpack/huffman.rs:1129-1131`
- セキュリティ: 悪意ある入力による不正データ受け入れ
- RFC 7541 Section 5.2 違反
