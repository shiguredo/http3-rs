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

/// サーバー実装向けの接続エラー種別
///
/// ngtcp2 の API 契約では `ngtcp2_conn_read_pkt` や `ngtcp2_conn_handle_expiry` が
/// 返す一部の負エラーは接続単位の非致命的エラーであり、サーバー全体を停止させては
/// ならない (`ngtcp2_conn_read_pkt` のドキュメント参照)。サーバー実装はこの分類に
/// 従ってエラーを接続単位で処理する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionErrorKind {
    /// 無視してよいエラー (パケットの破棄指示やストリーム単位のシグナル)
    Ignore,

    /// 接続を黙って破棄する
    ///
    /// NGTCP2_ERR_DROP_CONN / NGTCP2_ERR_IDLE_CLOSE / NGTCP2_ERR_RETRY。
    /// CONNECTION_CLOSE は送らない。
    SilentDrop,

    /// 終了状態 (closing / draining) に移行済みの接続
    ///
    /// NGTCP2_ERR_DRAINING / NGTCP2_ERR_CLOSING。
    /// 接続は閉じられつつあるため、そのままにして除去処理に任せる。
    Terminal,

    /// トランスポートエラー。CONNECTION_CLOSE (0x1c) を送って closing 状態にする
    ///
    /// NGTCP2_ERR_CRYPTO などの致命的エラー (RFC 9000 Section 11.1)。
    TransportClose,

    /// アプリケーションエラー。CONNECTION_CLOSE (0x1d) を送って closing 状態にする
    ///
    /// nghttp3 (HTTP/3 層) のエラー (RFC 9000 Section 11.1)。
    ApplicationClose,

    /// 内部エラー。CONNECTION_CLOSE を送らずに接続を破棄する
    ///
    /// プロトコル違反ではなく実装側の問題であるため、ピアに通知する
    /// エラーコードが存在しない。
    Internal,
}

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

    /// サーバー実装向けにエラーを接続単位の種別へ分類する
    ///
    /// 分類は ngtcp2 の API 契約 (`ngtcp2_conn_read_pkt` /
    /// `ngtcp2_conn_handle_expiry` のドキュメント) に従う。各種別の詳細は
    /// `ConnectionErrorKind` の各 variant のドキュメント参照。
    ///
    /// NGTCP2_ERR_RETRY は Retry パケット送信の要求だが、本実装では
    /// Retry を送らないため SilentDrop に分類する (将来 Retry 送信を
    /// 実装する場合は分類を見直すこと)。
    pub fn classify_connection_error(&self) -> ConnectionErrorKind {
        match self {
            Error::Ngtcp2(_, code) => {
                let code = *code;
                match code {
                    ngtcp2_sys::NGTCP2_ERR_DISCARD_PKT => ConnectionErrorKind::Ignore,
                    ngtcp2_sys::NGTCP2_ERR_DROP_CONN
                    | ngtcp2_sys::NGTCP2_ERR_IDLE_CLOSE
                    | ngtcp2_sys::NGTCP2_ERR_RETRY => ConnectionErrorKind::SilentDrop,
                    ngtcp2_sys::NGTCP2_ERR_DRAINING | ngtcp2_sys::NGTCP2_ERR_CLOSING => {
                        ConnectionErrorKind::Terminal
                    }
                    _ => ConnectionErrorKind::TransportClose,
                }
            }
            Error::Nghttp3(_, _) => ConnectionErrorKind::ApplicationClose,
            // ストリーム単位のフロー制御シグナル。接続エラーとして扱わない
            Error::StreamDataBlocked(_) | Error::StreamShutWr(_) => ConnectionErrorKind::Ignore,
            Error::InvalidArgument(_)
            | Error::BufferTooSmall
            | Error::StreamNotFound(_)
            | Error::ConnectionClosing
            | Error::Internal(_) => ConnectionErrorKind::Internal,
        }
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
