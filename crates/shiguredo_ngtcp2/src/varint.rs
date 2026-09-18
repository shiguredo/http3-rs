//! QUIC 可変長整数 (RFC 9000 Section 16)
//!
//! nghttp3 の公開 API (nghttp3_put_uvarint / nghttp3_get_uvarint) を使った
//! varint エンコード・デコード。

use nghttp3_sys;

/// 値をエンコードするのに必要なバイト数を返す
pub fn encoded_len(n: u64) -> usize {
    unsafe { nghttp3_sys::nghttp3_put_uvarintlen(n) }
}

/// 可変長整数を `buf` にエンコードする
///
/// `buf` は `encoded_len(n)` バイト以上必要。
/// 書き込んだバイト数を返す。
///
/// # Panics
///
/// `buf` が不足している場合はパニックする。
pub fn encode(buf: &mut [u8], n: u64) -> usize {
    let len = encoded_len(n);
    assert!(buf.len() >= len, "varint encode: buffer too short");
    unsafe {
        nghttp3_sys::nghttp3_put_uvarint(buf.as_mut_ptr(), n);
    }
    len
}

/// 可変長整数を `Vec<u8>` に追記する
pub fn encode_to_vec(n: u64, buf: &mut Vec<u8>) {
    let len = encoded_len(n);
    let start = buf.len();
    buf.resize(start + len, 0);
    encode(&mut buf[start..], n);
}

/// 可変長整数をデコードする
///
/// 成功時は `(値, 消費バイト数)` を返す。
/// `buf` が空または短すぎる場合は `None` を返す。
pub fn decode(buf: &[u8]) -> Option<(u64, usize)> {
    if buf.is_empty() {
        return None;
    }
    let len = unsafe { nghttp3_sys::nghttp3_get_uvarintlen(buf.as_ptr()) };
    if buf.len() < len {
        return None;
    }
    let mut dest = 0u64;
    unsafe {
        nghttp3_sys::nghttp3_get_uvarint(&mut dest, buf.as_ptr());
    }
    Some((dest, len))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_varint_roundtrip() {
        for value in [0u64, 1, 63, 64, 16383, 16384, 1073741823, 1073741824] {
            let mut buf = Vec::new();
            encode_to_vec(value, &mut buf);
            let (decoded, consumed) =
                decode(&buf).expect("infallible: implementation bug if this panics");
            assert_eq!(decoded, value);
            assert_eq!(consumed, buf.len());
        }
    }

    #[test]
    fn test_varint_encoded_len() {
        assert_eq!(encoded_len(0), 1);
        assert_eq!(encoded_len(63), 1);
        assert_eq!(encoded_len(64), 2);
        assert_eq!(encoded_len(16383), 2);
        assert_eq!(encoded_len(16384), 4);
        assert_eq!(encoded_len(1073741823), 4);
        assert_eq!(encoded_len(1073741824), 8);
    }

    #[test]
    fn test_varint_decode_empty() {
        assert!(decode(&[]).is_none());
    }

    #[test]
    fn test_varint_decode_truncated() {
        // 2 バイト必要なのに 1 バイトしかない
        assert!(decode(&[0x40]).is_none());
    }
}
