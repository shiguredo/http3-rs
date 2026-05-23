//! Property-Based Testing for shiguredo_http3
//!
//! 各構築時検査型の `valid_*` 戦略を集約する。
//! 個別の `pbt/tests/prop_*.rs` から `use pbt::strategies::*;` で再利用する。

pub mod strategies {
    use proptest::prelude::*;
    use shiguredo_http3::VarInt;

    /// RFC 9000 Section 16: VarInt の値域 (0..=2^62 - 1)
    pub fn valid_varint() -> impl Strategy<Value = VarInt> {
        (0u64..=VarInt::MAX.get()).prop_map(|v| VarInt::new(v).unwrap())
    }

    /// `VarInt::new` / `VarInt::try_from` が必ず `Err` を返す入力 (PBT の
    /// negative path 用)
    pub fn invalid_varint_u64() -> impl Strategy<Value = u64> {
        (VarInt::MAX.get() + 1)..=u64::MAX
    }

    /// RFC 9114 Section 4.2 / RFC 9110 Section 5.1: 小文字 token-char のみで
    /// 構成された valid な field name (1..64 byte)
    pub fn valid_header_name() -> impl Strategy<Value = Vec<u8>> {
        (1usize..64)
            .prop_flat_map(|len| prop::collection::vec(prop::char::range('a', 'z'), len))
            .prop_map(|chars| chars.into_iter().map(|c| c as u8).collect())
    }

    /// RFC 9110 Section 5.5 の field-content に従い、両端は field-vchar
    /// (0x21-0x7E)、間には SP (0x20) も許す。空文字列も valid。
    /// (obs-text 0x80-0xFF は QPACK Huffman デコードとの整合性を保つため除外)
    pub fn valid_header_value() -> impl Strategy<Value = Vec<u8>> {
        (0usize..256)
            .prop_flat_map(|len| prop::collection::vec(0x20u8..=0x7e, len))
            .prop_map(|middle| {
                if middle.is_empty() {
                    return Vec::new();
                }
                let mut v = middle;
                if v[0] == 0x20 {
                    v[0] = 0x21;
                }
                let last = v.len() - 1;
                if v[last] == 0x20 {
                    v[last] = 0x21;
                }
                v
            })
    }
}
