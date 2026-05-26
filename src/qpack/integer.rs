//! RFC 7541 Section 5.1 のプレフィックス整数エンコード/デコード
//!
//! QPACK (RFC 9204 Section 4.1.1) は RFC 7541 Section 5.1 を unmodified で参照する。
//! RFC 9204 Section 4.1.1 では 62 ビットまでデコードできること (MUST) を要求している。
//! この要件は shift > 56 でのオーバーフロー保護によって満たされる。
//!
//! 仕様は将来変更される可能性がある。

use crate::error::QpackError;

/// スライスにエンコード (RFC 7541 Section 5.1)
///
/// バッファ不足時は None を返す。
pub fn encode_integer(buf: &mut [u8], value: u64, prefix_bits: u8, prefix: u8) -> Option<usize> {
    let max_prefix = (1u64 << prefix_bits) - 1;

    if value < max_prefix {
        if buf.is_empty() {
            return None;
        }
        buf[0] = prefix | (value as u8);
        Some(1)
    } else {
        if buf.is_empty() {
            return None;
        }
        buf[0] = prefix | (max_prefix as u8);
        let mut offset = 1;
        let mut remaining = value - max_prefix;

        while remaining >= 128 {
            if offset >= buf.len() {
                return None;
            }
            buf[offset] = 0x80 | ((remaining & 0x7f) as u8);
            remaining >>= 7;
            offset += 1;
        }

        if offset >= buf.len() {
            return None;
        }
        buf[offset] = remaining as u8;
        Some(offset + 1)
    }
}

/// Vec にエンコード (RFC 7541 Section 5.1)
///
/// Vec に push で追記する。バッファ不足は発生しない。
pub fn encode_integer_to_vec(buf: &mut Vec<u8>, value: u64, prefix_bits: u8, prefix: u8) {
    let max_prefix = (1u64 << prefix_bits) - 1;

    if value < max_prefix {
        buf.push(prefix | (value as u8));
    } else {
        buf.push(prefix | (max_prefix as u8));
        let mut remaining = value - max_prefix;

        while remaining >= 128 {
            buf.push(0x80 | ((remaining & 0x7f) as u8));
            remaining >>= 7;
        }
        buf.push(remaining as u8);
    }
}

/// デコード (RFC 7541 Section 5.1)
///
/// shift > 56 でオーバーフロー保護 (RFC 9204 Section 4.1.1: 62 ビットまでデコード可能)。
pub fn decode_integer(data: &[u8], prefix_bits: u8) -> Result<(u64, usize), QpackError> {
    if data.is_empty() {
        return Err(QpackError::BufferTooShort);
    }

    let mask = ((1u16 << prefix_bits) - 1) as u8;
    let prefix_value = data[0] & mask;

    if prefix_value < mask {
        return Ok((prefix_value as u64, 1));
    }

    let mut value = prefix_value as u64;
    let mut shift = 0u32;
    let mut offset = 1;

    loop {
        if offset >= data.len() {
            return Err(QpackError::BufferTooShort);
        }

        let byte = data[offset];
        value += ((byte & 0x7f) as u64) << shift;
        offset += 1;

        if byte & 0x80 == 0 {
            break;
        }

        shift += 7;
        if shift > 56 {
            return Err(QpackError::DecodeFailed);
        }
    }

    Ok((value, offset))
}
