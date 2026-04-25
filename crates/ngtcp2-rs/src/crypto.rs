//! TLS コンテキスト管理
//!
//! ngtcp2_crypto_boringssl と aws-lc を統合して、QUIC 接続用の
//! TLS コンテキストを管理する。
//!
//! # 設計
//!
//! - `TlsContext`: SSL_CTX のラッパー。サーバー/クライアント共通の設定を保持
//! - `TlsSession`: SSL のラッパー。個々の接続で使用
//!
//! ngtcp2_crypto_boringssl_configure_*_context() を使用することで、
//! ngtcp2 が必要とする TLS コールバックが自動的に設定される。

use std::ffi::{CString, c_void};
use std::path::Path;

use aws_lc_sys::{
    SSL, SSL_CTX, SSL_CTX_free, SSL_CTX_new, SSL_CTX_set_alpn_protos, SSL_CTX_use_PrivateKey_file,
    SSL_CTX_use_certificate_chain_file, SSL_FILETYPE_PEM, SSL_free, SSL_new, SSL_set_accept_state,
    SSL_set_connect_state, SSL_set_tlsext_host_name, TLS_method,
};
use ngtcp2_sys::{
    ngtcp2_crypto_boringssl_configure_client_context,
    ngtcp2_crypto_boringssl_configure_server_context,
};

use crate::error::{Error, Result};

/// TLS コンテキスト
///
/// SSL_CTX をラップし、QUIC 接続に必要な設定を提供する。
/// サーバーまたはクライアント用に作成し、複数の TlsSession を生成できる。
pub struct TlsContext {
    ctx: *mut SSL_CTX,
    is_server: bool,
    // ALPN コールバックで使用するデータ (サーバー用)
    // コールバックに渡したポインタを保持し、Drop で解放する
    alpn_data: Option<*mut Vec<u8>>,
}

// SAFETY: SSL_CTX はスレッドセーフに使用される
unsafe impl Send for TlsContext {}
unsafe impl Sync for TlsContext {}

impl TlsContext {
    /// クライアント用 TLS コンテキストを作成
    ///
    /// # Arguments
    ///
    /// * `alpn` - ALPN プロトコルリスト (例: &[b"h3"])
    pub fn new_client(alpn: &[&[u8]]) -> Result<Self> {
        Self::new_client_with_options(alpn, true)
    }

    /// クライアント用 TLS コンテキストを作成 (オプション付き)
    ///
    /// # Arguments
    ///
    /// * `alpn` - ALPN プロトコルリスト (例: &[b"h3"])
    /// * `verify_peer` - サーバー証明書を検証するかどうか
    pub fn new_client_with_options(alpn: &[&[u8]], verify_peer: bool) -> Result<Self> {
        unsafe {
            let method = TLS_method();
            if method.is_null() {
                return Err(Error::Internal("TLS_method failed".to_string()));
            }

            let ctx = SSL_CTX_new(method);
            if ctx.is_null() {
                return Err(Error::Internal("SSL_CTX_new failed".to_string()));
            }

            // ngtcp2 用の設定を適用
            // SAFETY: aws_lc_sys::SSL_CTX と ngtcp2_sys::SSL_CTX は同じ構造体
            // 両方とも aws-lc/BoringSSL の SSL_CTX へのポインタ
            let rv =
                ngtcp2_crypto_boringssl_configure_client_context(ctx as *mut ngtcp2_sys::SSL_CTX);
            if rv != 0 {
                SSL_CTX_free(ctx);
                return Err(Error::Internal(
                    "ngtcp2_crypto_boringssl_configure_client_context failed".to_string(),
                ));
            }

            // 証明書検証の設定
            if !verify_peer {
                // 証明書検証を無効にする (テスト用の自己署名証明書で使用)
                aws_lc_sys::SSL_CTX_set_verify(ctx, aws_lc_sys::SSL_VERIFY_NONE, None);
            }

            // ALPN を設定
            let alpn_wire = Self::encode_alpn(alpn);
            let rv = SSL_CTX_set_alpn_protos(ctx, alpn_wire.as_ptr(), alpn_wire.len());
            if rv != 0 {
                SSL_CTX_free(ctx);
                return Err(Error::Internal(
                    "SSL_CTX_set_alpn_protos failed".to_string(),
                ));
            }

            Ok(Self {
                ctx,
                is_server: false,
                alpn_data: None,
            })
        }
    }

    /// サーバー用 TLS コンテキストを作成
    ///
    /// # Arguments
    ///
    /// * `cert_path` - 証明書ファイルのパス (PEM 形式)
    /// * `key_path` - 秘密鍵ファイルのパス (PEM 形式)
    /// * `alpn` - ALPN プロトコルリスト (例: &[b"h3"])
    pub fn new_server(cert_path: &Path, key_path: &Path, alpn: &[&[u8]]) -> Result<Self> {
        unsafe {
            let method = TLS_method();
            if method.is_null() {
                return Err(Error::Internal("TLS_method failed".to_string()));
            }

            let ctx = SSL_CTX_new(method);
            if ctx.is_null() {
                return Err(Error::Internal("SSL_CTX_new failed".to_string()));
            }

            // ngtcp2 用の設定を適用
            // SAFETY: aws_lc_sys::SSL_CTX と ngtcp2_sys::SSL_CTX は同じ構造体
            let rv =
                ngtcp2_crypto_boringssl_configure_server_context(ctx as *mut ngtcp2_sys::SSL_CTX);
            if rv != 0 {
                SSL_CTX_free(ctx);
                return Err(Error::Internal(
                    "ngtcp2_crypto_boringssl_configure_server_context failed".to_string(),
                ));
            }

            // 証明書を読み込み
            let cert_path_cstr = CString::new(cert_path.to_string_lossy().as_bytes())
                .map_err(|_| Error::InvalidArgument("invalid cert path".to_string()))?;
            let rv = SSL_CTX_use_certificate_chain_file(ctx, cert_path_cstr.as_ptr());
            if rv != 1 {
                SSL_CTX_free(ctx);
                return Err(Error::Internal(format!(
                    "SSL_CTX_use_certificate_chain_file failed: {}",
                    cert_path.display()
                )));
            }

            // 秘密鍵を読み込み
            let key_path_cstr = CString::new(key_path.to_string_lossy().as_bytes())
                .map_err(|_| Error::InvalidArgument("invalid key path".to_string()))?;
            let rv = SSL_CTX_use_PrivateKey_file(ctx, key_path_cstr.as_ptr(), SSL_FILETYPE_PEM);
            if rv != 1 {
                SSL_CTX_free(ctx);
                return Err(Error::Internal(format!(
                    "SSL_CTX_use_PrivateKey_file failed: {}",
                    key_path.display()
                )));
            }

            // ALPN コールバックを設定 (サーバー用)
            let alpn_wire = Self::encode_alpn(alpn);
            let alpn_data = Box::new(alpn_wire);
            let alpn_ptr = Box::into_raw(alpn_data);

            aws_lc_sys::SSL_CTX_set_alpn_select_cb(
                ctx,
                Some(alpn_select_callback),
                alpn_ptr as *mut c_void,
            );

            Ok(Self {
                ctx,
                is_server: true,
                alpn_data: Some(alpn_ptr),
            })
        }
    }

    /// TLS セッションを作成
    pub fn create_session(&self) -> Result<TlsSession> {
        unsafe {
            let ssl = SSL_new(self.ctx);
            if ssl.is_null() {
                return Err(Error::Internal("SSL_new failed".to_string()));
            }

            if self.is_server {
                SSL_set_accept_state(ssl);
            } else {
                SSL_set_connect_state(ssl);
            }

            Ok(TlsSession {
                ssl,
                is_server: self.is_server,
            })
        }
    }

    /// ALPN プロトコルリストをワイヤーフォーマットにエンコード
    ///
    /// ワイヤーフォーマット: [len1][proto1][len2][proto2]...
    fn encode_alpn(alpn: &[&[u8]]) -> Vec<u8> {
        let mut wire = Vec::new();
        for proto in alpn {
            wire.push(proto.len() as u8);
            wire.extend_from_slice(proto);
        }
        wire
    }

    /// 内部ポインタを取得
    pub fn as_ptr(&self) -> *mut SSL_CTX {
        self.ctx
    }
}

impl Drop for TlsContext {
    fn drop(&mut self) {
        if !self.ctx.is_null() {
            unsafe {
                // ALPN コールバックで設定したデータを解放
                if let Some(alpn_ptr) = self.alpn_data {
                    let _ = Box::from_raw(alpn_ptr);
                }
                SSL_CTX_free(self.ctx);
            }
        }
    }
}

/// TLS セッション
///
/// SSL をラップし、個々の QUIC 接続で使用する。
pub struct TlsSession {
    ssl: *mut SSL,
    is_server: bool,
}

// SAFETY: SSL は適切にロックされて使用される
unsafe impl Send for TlsSession {}
unsafe impl Sync for TlsSession {}

impl TlsSession {
    /// SNI (Server Name Indication) を設定
    ///
    /// クライアント接続で使用する。接続先のサーバー名を指定する。
    pub fn set_server_name(&mut self, server_name: &str) -> Result<()> {
        if self.is_server {
            return Err(Error::InvalidArgument(
                "cannot set server name on server session".to_string(),
            ));
        }

        let server_name_cstr = CString::new(server_name)
            .map_err(|_| Error::InvalidArgument("invalid server name".to_string()))?;

        unsafe {
            let rv = SSL_set_tlsext_host_name(self.ssl, server_name_cstr.as_ptr());
            if rv != 1 {
                return Err(Error::Internal(
                    "SSL_set_tlsext_host_name failed".to_string(),
                ));
            }
        }

        Ok(())
    }

    /// QUIC トランスポートパラメータを設定
    ///
    /// ngtcp2_conn に接続する前に呼び出す必要がある。
    pub fn set_quic_transport_params(&mut self, params: &[u8]) -> Result<()> {
        unsafe {
            let rv =
                aws_lc_sys::SSL_set_quic_transport_params(self.ssl, params.as_ptr(), params.len());
            if rv != 1 {
                return Err(Error::Internal(
                    "SSL_set_quic_transport_params failed".to_string(),
                ));
            }
        }
        Ok(())
    }

    /// 内部ポインタを取得
    pub fn as_ptr(&self) -> *mut SSL {
        self.ssl
    }

    /// 内部ポインタを c_void として取得 (ngtcp2 用)
    pub fn as_void_ptr(&self) -> *mut c_void {
        self.ssl as *mut c_void
    }

    /// サーバーかどうか
    pub fn is_server(&self) -> bool {
        self.is_server
    }
}

impl Drop for TlsSession {
    fn drop(&mut self) {
        if !self.ssl.is_null() {
            unsafe {
                SSL_free(self.ssl);
            }
        }
    }
}

/// ALPN 選択コールバック (サーバー用)
///
/// クライアントが提示した ALPN リストから、サーバーがサポートするプロトコルを選択する。
unsafe extern "C" fn alpn_select_callback(
    _ssl: *mut SSL,
    out: *mut *const u8,
    outlen: *mut u8,
    client_alpn: *const u8,
    client_alpn_len: u32,
    arg: *mut c_void,
) -> i32 {
    const SSL_TLSEXT_ERR_OK: i32 = 0;
    const SSL_TLSEXT_ERR_NOACK: i32 = 3;

    if arg.is_null() {
        return SSL_TLSEXT_ERR_NOACK;
    }

    // SAFETY: arg は TlsContext::new_server で作成した Vec<u8> へのポインタ
    let server_alpn = unsafe { &*(arg as *const Vec<u8>) };
    let client_alpn_slice =
        unsafe { std::slice::from_raw_parts(client_alpn, client_alpn_len as usize) };

    // サーバーの ALPN リストをパース
    let mut server_pos = 0;
    while server_pos < server_alpn.len() {
        let server_proto_len = server_alpn[server_pos] as usize;
        server_pos += 1;
        if server_pos + server_proto_len > server_alpn.len() {
            break;
        }
        let server_proto = &server_alpn[server_pos..server_pos + server_proto_len];
        server_pos += server_proto_len;

        // クライアントの ALPN リストをパース
        let mut client_pos = 0;
        while client_pos < client_alpn_slice.len() {
            let client_proto_len = client_alpn_slice[client_pos] as usize;
            client_pos += 1;
            if client_pos + client_proto_len > client_alpn_slice.len() {
                break;
            }
            let client_proto = &client_alpn_slice[client_pos..client_pos + client_proto_len];
            client_pos += client_proto_len;

            // マッチした場合
            if server_proto == client_proto {
                // SAFETY: out と outlen は呼び出し元から渡された有効なポインタ
                unsafe {
                    *out = client_alpn.add(client_pos - client_proto_len);
                    *outlen = client_proto_len as u8;
                }
                return SSL_TLSEXT_ERR_OK;
            }
        }
    }

    SSL_TLSEXT_ERR_NOACK
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_alpn() {
        let alpn = TlsContext::encode_alpn(&[b"h3", b"h3-29"]);
        assert_eq!(alpn, vec![2, b'h', b'3', 5, b'h', b'3', b'-', b'2', b'9']);
    }

    #[test]
    fn test_client_context_creation() {
        let ctx = TlsContext::new_client(&[b"h3"]);
        assert!(ctx.is_ok());
    }
}
