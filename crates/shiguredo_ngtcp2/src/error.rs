use std::ffi::CStr;
use std::fmt;

/// ngtcp2/nghttp3 のエラー型
#[derive(Debug)]
pub enum Error {
    /// ngtcp2 エラー
    Ngtcp2(String, i32),

    /// nghttp3 エラー
    Nghttp3(String, i32),

    /// 無効な引数
    InvalidArgument(String),

    /// バッファ不足
    BufferTooSmall,

    /// ストリームが見つからない
    StreamNotFound(i64),

    /// 接続が閉じている
    ConnectionClosing,

    /// ストリームがフロー制御でブロックされている
    StreamDataBlocked(i64),

    /// ストリームの書き込みがシャットダウンされている
    StreamShutWr(i64),

    /// 内部エラー
    Internal(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Ngtcp2(msg, code) => write!(f, "ngtcp2 error: {} ({})", msg, code),
            Error::Nghttp3(msg, code) => write!(f, "nghttp3 error: {} ({})", msg, code),
            Error::InvalidArgument(msg) => write!(f, "invalid argument: {}", msg),
            Error::BufferTooSmall => write!(f, "buffer too small"),
            Error::StreamNotFound(id) => write!(f, "stream not found: {}", id),
            Error::ConnectionClosing => write!(f, "connection is closing"),
            Error::StreamDataBlocked(id) => write!(f, "stream data blocked: {}", id),
            Error::StreamShutWr(id) => write!(f, "stream shut wr: {}", id),
            Error::Internal(msg) => write!(f, "internal error: {}", msg),
        }
    }
}

impl std::error::Error for Error {}

/// Result 型エイリアス
pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// ngtcp2 エラーコードからエラーを生成
    pub fn from_ngtcp2(code: libc::c_int) -> Self {
        let msg = unsafe {
            let ptr = ngtcp2_sys::ngtcp2_strerror(code);
            if ptr.is_null() {
                "unknown error".to_string()
            } else {
                CStr::from_ptr(ptr).to_string_lossy().into_owned()
            }
        };
        Error::Ngtcp2(msg, code)
    }

    /// nghttp3 エラーコードからエラーを生成
    pub fn from_nghttp3(code: libc::c_int) -> Self {
        let msg = unsafe {
            let ptr = nghttp3_sys::nghttp3_strerror(code);
            if ptr.is_null() {
                "unknown error".to_string()
            } else {
                CStr::from_ptr(ptr).to_string_lossy().into_owned()
            }
        };
        Error::Nghttp3(msg, code)
    }
}

/// ngtcp2 の結果をチェック
pub fn check_ngtcp2(code: libc::c_int) -> Result<()> {
    if code < 0 {
        Err(Error::from_ngtcp2(code))
    } else {
        Ok(())
    }
}

/// nghttp3 の結果をチェック
pub fn check_nghttp3(code: libc::c_int) -> Result<()> {
    if code < 0 {
        Err(Error::from_nghttp3(code))
    } else {
        Ok(())
    }
}
