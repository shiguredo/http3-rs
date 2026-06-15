# QPACK 整数デコードの 62-bit 上限を厳密化する

- Priority: Medium
- Created: 2026-06-15
- Completed:
- Model: Opus 4.7
- Branch: feature/fix-qpack-integer-62bit-limit
- Polished:

## 目的

`src/qpack/integer.rs:78-122` の `decode_integer` は `shift > 56` でオーバーフロー保護をするが、prefix_bits=8 のとき最大 2^63 程度まで受理する。RFC 9204 Section 4.1.1 は「62 ビットまでデコードできる MUST」を要求しており、その上限を超える値を厳密に弾くべき。実装の上限を明確化する。

## 優先度根拠

Medium。仕様 (62 bit を最低保証) は満たすが、それ以上の値も無検査で受理してしまうため、上位の QPACK プロトコル不変条件 (`VarInt` 範囲 = 2^62 - 1) と整合させたい。

## 現状

`src/qpack/integer.rs:78-122`:

```rust
pub fn decode_integer(data: &[u8], prefix_bits: u8) -> Result<(u64, usize), QpackError> {
    // ...
    let mut value = prefix_value as u64;
    let mut shift = 0u32;
    // ...
    loop {
        // ...
        value += ((byte & 0x7f) as u64) << shift;
        // ...
        if shift > 56 {
            return Err(QpackError::DecodeFailed);
        }
    }
    Ok((value, offset))
}
```

`shift > 56` ガードでは prefix_bits=8 のとき (max_prefix=255、shift=63 まで進める) 最大 `2^63 - 1` 付近まで受理する。

RFC 9204 Section 4.1.1:

> QPACK implementations MUST be able to decode integers up to and including 62 bits long.

## 設計方針

- デコード結果が `(1u64 << 62) - 1` (= VarInt 上限) を超えたら `QpackError::DecodeFailed` を返す
- これは MUST より厳しい (厳格化) 制限になるが、QPACK / HTTP/3 の他箇所で `VarInt` 範囲を前提にしているため整合する
- PBT で「62 bit を超える VarInt は必ず DecodeFailed」を検証
- fuzz_target がこの境界をカバーすることを確認

## 完了条件

- 62 bit 超の整数が `DecodeFailed` で弾かれる
- PBT で境界が検証される
- 既存テスト・PBT・fuzz がパスする
- `make fmt && make clippy && make check` が通る

## 解決方法

```rust
const QPACK_MAX_INTEGER: u64 = (1u64 << 62) - 1;

if value > QPACK_MAX_INTEGER {
    return Err(QpackError::DecodeFailed);
}
```

### 関連ファイル

- 修正対象: `src/qpack/integer.rs:78-122`
- PBT 追加: `pbt/tests/prop_qpack/integer.rs`
- 一次資料: `refs/h3/rfc9204.txt` Section 4.1.1
