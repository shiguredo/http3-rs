//! Property-Based Testing for shiguredo_http3
//!
//! 各構築時検査型の `valid_*` 戦略を集約する。
//! 個別の `pbt/tests/prop_*.rs` から `use pbt::strategies::*;` で再利用する。

use shiguredo_http3::qpack::{Decoder, Header};

/// 検査なしの name/value を QPACK Literal Field Line with Literal Name
/// (RFC 9204 Section 4.5.6) として符号化し、Decoder でデコードして
/// Header を返す (wire 模擬)。
pub fn wire_header(name: &[u8], value: &[u8]) -> Header {
    let mut wire = Vec::new();
    // Required Insert Count = 0
    wire.push(0x00);
    // Delta Base = 0
    wire.push(0x00);
    // Literal Field Line with Literal Name: 001N=0, H=0
    encode_qpack_literal(&mut wire, name, 3, 0x20);
    // Value: H=0, 7-bit prefix
    encode_qpack_string(&mut wire, value);

    let decoder = Decoder::new();
    let headers = decoder
        .decode(&wire)
        .expect("infallible: wire_header produced invalid QPACK");
    headers
        .into_iter()
        .next()
        .expect("infallible: wire encoding produces exactly one header")
}

/// QPACK 整数 (RFC 7541 Section 5.1) を指定 prefix bits で符号化する。
fn encode_qpack_integer(buf: &mut Vec<u8>, value: u64, prefix_bits: u8, prefix: u8) {
    let max_prefix = (1u64 << prefix_bits) - 1;
    if value < max_prefix {
        buf.push(prefix | value as u8);
    } else {
        buf.push(prefix | max_prefix as u8);
        let mut remaining = value - max_prefix;
        while remaining >= 128 {
            buf.push(0x80 | (remaining & 0x7f) as u8);
            remaining >>= 7;
        }
        buf.push(remaining as u8);
    }
}

/// QPACK string literal (H=0) を指定 prefix/prefix_bits で符号化する。
fn encode_qpack_literal(buf: &mut Vec<u8>, data: &[u8], prefix_bits: u8, prefix: u8) {
    encode_qpack_integer(buf, data.len() as u64, prefix_bits, prefix);
    buf.extend_from_slice(data);
}

/// QPACK string literal (H=0, 7-bit prefix) を符号化する。
fn encode_qpack_string(buf: &mut Vec<u8>, data: &[u8]) {
    encode_qpack_literal(buf, data, 7, 0x00);
}

pub mod strategies {
    use noprop::TestCaseContext;
    use shiguredo_http3::VarInt;

    /// RFC 9000 Section 16: VarInt の値域 (0..=2^62 - 1)
    pub fn valid_varint(ctx: &mut TestCaseContext) -> VarInt {
        VarInt::new(noprop::sample_u64_in(ctx, 0..=VarInt::MAX.get()))
            .expect("value is within VarInt::MAX")
    }

    /// `VarInt::new` / `VarInt::try_from` が必ず `Err` を返す入力 (PBT の
    /// negative path 用)
    pub fn invalid_varint_u64(ctx: &mut TestCaseContext) -> u64 {
        noprop::sample_u64_in(ctx, (VarInt::MAX.get() + 1)..=u64::MAX)
    }

    /// RFC 9114 Section 4.2 / RFC 9110 Section 5.1: 小文字 token-char のみで
    /// 構成された valid な field name (1..64 byte)
    pub fn valid_header_name(ctx: &mut TestCaseContext) -> Vec<u8> {
        let len = noprop::sample_usize_in(ctx, 1..64);
        let mut name = Vec::new();
        for _ in 0..len {
            // 小文字英字 (a-z) から 1 文字ずつサンプリングする
            name.push(b'a' + noprop::sample_usize_in(ctx, 0..26) as u8);
        }
        name
    }

    /// RFC 9110 Section 5.5 の field-content に従い、両端は field-vchar
    /// (0x21-0x7E)、間には SP (0x20) も許す。空文字列も valid。
    /// (obs-text 0x80-0xFF は QPACK Huffman デコードとの整合性を保つため除外)
    pub fn valid_header_value(ctx: &mut TestCaseContext) -> Vec<u8> {
        let len = noprop::sample_usize_in(ctx, 0..256);
        let mut middle = Vec::new();
        for _ in 0..len {
            // field-content: 0x20-0x7E からサンプリングする
            middle.push(0x20 + noprop::sample_usize_in(ctx, 0..=0x5e) as u8);
        }
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
    }
}
