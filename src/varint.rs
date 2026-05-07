//! QUIC 可変長整数 (RFC 9000 Section 16)
//!
//! QUIC の可変長整数は 1, 2, 4, 8 バイトでエンコードされ、最大 2^62-1 まで表現可能。

use bytes::BufMut;

/// 可変長整数の最大値 (2^62 - 1)
pub const MAX_VALUE: u64 = (1 << 62) - 1;

/// 可変長整数デコードエラー
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// バッファが不足している
    BufferTooShort,
}

/// 可変長整数エンコードエラー
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodeError {
    /// 値が大きすぎる (> 2^62 - 1)
    ValueTooLarge,
    /// バッファが不足している
    BufferTooShort,
}

/// 値をエンコードするのに必要なバイト数を返す
///
/// 呼び出し側が値 <= `MAX_VALUE` を保証している場面で使う内部向けヘルパー。
/// 外部からは `try_encoded_len` を使うこと。
///
/// # Panics
///
/// 値が `MAX_VALUE` を超える場合はパニックする
#[inline]
pub fn encoded_len(value: u64) -> usize {
    if value < 64 {
        1
    } else if value < 16384 {
        2
    } else if value < 1_073_741_824 {
        4
    } else if value <= MAX_VALUE {
        8
    } else {
        panic!("value exceeds maximum: {value}");
    }
}

/// 値をエンコードするのに必要なバイト数を返す (panic しない公開 API)
///
/// 値が `MAX_VALUE` を超える場合は `Err(EncodeError::ValueTooLarge)` を返す。
/// Sans I/O 境界ではこちらを使うこと。
#[inline]
pub fn try_encoded_len(value: u64) -> Result<usize, EncodeError> {
    if value > MAX_VALUE {
        return Err(EncodeError::ValueTooLarge);
    }
    Ok(encoded_len(value))
}

/// 可変長整数を `BufMut` に追記する
///
/// `Vec<u8>` も `BytesMut` も `BufMut` を実装するため、どちらの buffer にも追記できる。
/// 呼び出し側が値 <= `MAX_VALUE` を保証している場面で使う内部向けヘルパー。
/// 外部からは `try_encode_into` を使うこと。
///
/// # Panics
///
/// 値が `MAX_VALUE` を超える場合はパニックする
pub fn encode_into<B: BufMut>(buf: &mut B, value: u64) {
    let len = encoded_len(value);
    // BufMut::put_u{8,16,32,64} は big-endian で書き込む (RFC 9000 Section 16 と一致)
    match len {
        1 => buf.put_u8(value as u8),
        2 => buf.put_u16((value as u16) | 0x4000),
        4 => buf.put_u32((value as u32) | 0x8000_0000),
        8 => buf.put_u64(value | 0xc000_0000_0000_0000),
        _ => unreachable!(),
    }
}

/// 可変長整数を `BufMut` に追記する (panic しない公開 API)
///
/// 値が `MAX_VALUE` を超える場合は `Err(EncodeError::ValueTooLarge)` を返す。
/// Sans I/O 境界ではこちらを使うこと。
pub fn try_encode_into<B: BufMut>(buf: &mut B, value: u64) -> Result<(), EncodeError> {
    if value > MAX_VALUE {
        return Err(EncodeError::ValueTooLarge);
    }
    encode_into(buf, value);
    Ok(())
}

/// 可変長整数をエンコードする
///
/// 成功時はエンコードしたバイト数を返す
pub fn encode(buf: &mut [u8], value: u64) -> Result<usize, EncodeError> {
    if value > MAX_VALUE {
        return Err(EncodeError::ValueTooLarge);
    }

    let len = encoded_len(value);
    if buf.len() < len {
        return Err(EncodeError::BufferTooShort);
    }

    match len {
        1 => {
            buf[0] = value as u8;
        }
        2 => {
            let v = (value as u16) | 0x4000;
            buf[..2].copy_from_slice(&v.to_be_bytes());
        }
        4 => {
            let v = (value as u32) | 0x8000_0000;
            buf[..4].copy_from_slice(&v.to_be_bytes());
        }
        8 => {
            let v = value | 0xc000_0000_0000_0000;
            buf[..8].copy_from_slice(&v.to_be_bytes());
        }
        _ => unreachable!(),
    }

    Ok(len)
}

/// 可変長整数をデコードする
///
/// 成功時は (値, 消費バイト数) を返す
pub fn decode(buf: &[u8]) -> Result<(u64, usize), DecodeError> {
    if buf.is_empty() {
        return Err(DecodeError::BufferTooShort);
    }

    let prefix = buf[0] >> 6;
    let len = 1 << prefix;

    if buf.len() < len {
        return Err(DecodeError::BufferTooShort);
    }

    let value = match len {
        1 => u64::from(buf[0] & 0x3f),
        2 => {
            let mut bytes = [0u8; 2];
            bytes.copy_from_slice(&buf[..2]);
            u64::from(u16::from_be_bytes(bytes) & 0x3fff)
        }
        4 => {
            let mut bytes = [0u8; 4];
            bytes.copy_from_slice(&buf[..4]);
            u64::from(u32::from_be_bytes(bytes) & 0x3fff_ffff)
        }
        8 => {
            let mut bytes = [0u8; 8];
            bytes.copy_from_slice(&buf[..8]);
            u64::from_be_bytes(bytes) & 0x3fff_ffff_ffff_ffff
        }
        _ => unreachable!(),
    };

    Ok((value, len))
}

/// バッファの先頭バイトから可変長整数のバイト長を取得する
///
/// バッファが空の場合は `None` を返す
#[inline]
pub fn peek_len(buf: &[u8]) -> Option<usize> {
    buf.first().map(|&b| 1 << (b >> 6))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_buffer_too_short() {
        assert_eq!(decode(&[]), Err(DecodeError::BufferTooShort));
        assert_eq!(decode(&[0x40]), Err(DecodeError::BufferTooShort));
    }
}
