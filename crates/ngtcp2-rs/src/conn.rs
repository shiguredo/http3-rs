use std::collections::VecDeque;
use std::ffi::c_void;
use std::net::SocketAddr;
use std::ptr;

use libc::c_int;
use nghttp3_sys::{nghttp3_conn, nghttp3_conn_add_ack_offset};
use ngtcp2_sys::*;

use crate::crypto::TlsSession;
use crate::error::{Error, Result, check_ngtcp2};
use crate::types::{ConnectionId, PacketInfo, QuicVersion, StreamId};

/// ストリームデータイベント
#[derive(Debug, Clone)]
pub struct StreamData {
    /// ストリーム ID
    pub stream_id: StreamId,
    /// 受信データ
    pub data: Vec<u8>,
    /// ストリーム終了フラグ
    pub fin: bool,
}

/// DATAGRAM イベント
#[derive(Debug, Clone)]
pub struct Datagram {
    /// データグラムデータ
    pub data: Vec<u8>,
}

/// QUIC 接続
pub struct Connection {
    inner: *mut ngtcp2_conn,
    // コールバック用のユーザーデータ
    user_data: Box<ConnectionUserData>,
    // TLS セッション (高レベル API で使用)
    _tls_session: Option<TlsSession>,
    // ngtcp2_crypto_conn_ref (SSL に設定するため、高レベル API で使用)
    _conn_ref: Option<Box<ConnRef>>,
}

struct ConnectionUserData {
    // 受信したストリームデータのキュー
    stream_data_queue: VecDeque<StreamData>,
    // 受信した DATAGRAM のキュー
    datagram_queue: VecDeque<Datagram>,
    // NEW_CONNECTION_ID フレームでピアに発行した CID の記録
    //
    // サーバー実装はピアが DCID として使用する可能性のある CID を
    // ルーティングテーブルに登録するために使用する (RFC 9000 Section 5.1.1)。
    issued_cids: Vec<ConnectionId>,
    // nghttp3_conn へのポインタ
    //
    // acked_stream_data_offset コールバックから ACK 済みデータ量を通知するために使用する。
    // Http3Connection が無効になった場合に呼び出す set_h3_conn_null でクリアされる。
    h3_conn_ptr: *mut c_void,
}

/// ngtcp2_crypto_conn_ref のラッパー
/// SSL_set_app_data で SSL に設定し、TLS コールバックから ngtcp2_conn を取得するために使用
struct ConnRef {
    inner: ngtcp2_crypto_conn_ref,
}

// SAFETY: Connection は内部的にスレッドセーフに使用される
unsafe impl Send for Connection {}
unsafe impl Sync for Connection {}

impl Connection {
    /// クライアント接続を作成 (低レベル API)
    ///
    /// 呼び出し元が独自のコールバックと user_data を管理する。
    /// `poll_stream_data()` / `poll_datagram()` は使用できない
    /// (コールバックが内部キューに書き込まないため)。
    ///
    /// # Safety
    /// callbacks と settings は有効なポインタである必要がある
    #[expect(clippy::too_many_arguments)]
    pub unsafe fn client_new_raw(
        dcid: &ConnectionId,
        scid: &ConnectionId,
        local_addr: SocketAddr,
        remote_addr: SocketAddr,
        version: u32,
        callbacks: *const ngtcp2_callbacks,
        settings: *const ngtcp2_settings,
        params: *const ngtcp2_transport_params,
        user_data: *mut c_void,
    ) -> Result<Self> {
        let mut conn: *mut ngtcp2_conn = ptr::null_mut();

        let dcid_raw = cid_to_raw(dcid);
        let scid_raw = cid_to_raw(scid);

        let (local_sockaddr, local_len) = sockaddr_to_raw(&local_addr);
        let (remote_sockaddr, remote_len) = sockaddr_to_raw(&remote_addr);

        let path = ngtcp2_path {
            local: ngtcp2_addr {
                addr: &local_sockaddr as *const _ as *mut _,
                addrlen: local_len,
            },
            remote: ngtcp2_addr {
                addr: &remote_sockaddr as *const _ as *mut _,
                addrlen: remote_len,
            },
            user_data: ptr::null_mut(),
        };

        let rv = unsafe {
            ngtcp2_conn_client_new_versioned(
                &mut conn,
                &dcid_raw,
                &scid_raw,
                &path,
                version,
                NGTCP2_CALLBACKS_VERSION as c_int,
                callbacks,
                NGTCP2_SETTINGS_VERSION as c_int,
                settings,
                NGTCP2_TRANSPORT_PARAMS_VERSION as c_int,
                params,
                ptr::null(),
                user_data,
            )
        };

        check_ngtcp2(rv)?;

        let user_data_box = Box::new(ConnectionUserData {
            stream_data_queue: VecDeque::new(),
            datagram_queue: VecDeque::new(),
            issued_cids: Vec::new(),
            h3_conn_ptr: ptr::null_mut(),
        });

        Ok(Self {
            inner: conn,
            user_data: user_data_box,
            _tls_session: None,
            _conn_ref: None,
        })
    }

    /// サーバー接続を作成 (低レベル API)
    ///
    /// 呼び出し元が独自のコールバックと user_data を管理する。
    /// `poll_stream_data()` / `poll_datagram()` は使用できない
    /// (コールバックが内部キューに書き込まないため)。
    ///
    /// # Safety
    /// callbacks と settings は有効なポインタである必要がある
    #[expect(clippy::too_many_arguments)]
    pub unsafe fn server_new_raw(
        dcid: &ConnectionId,
        scid: &ConnectionId,
        local_addr: SocketAddr,
        remote_addr: SocketAddr,
        version: u32,
        callbacks: *const ngtcp2_callbacks,
        settings: *const ngtcp2_settings,
        params: *const ngtcp2_transport_params,
        user_data: *mut c_void,
    ) -> Result<Self> {
        let mut conn: *mut ngtcp2_conn = ptr::null_mut();

        let dcid_raw = cid_to_raw(dcid);
        let scid_raw = cid_to_raw(scid);

        let (local_sockaddr, local_len) = sockaddr_to_raw(&local_addr);
        let (remote_sockaddr, remote_len) = sockaddr_to_raw(&remote_addr);

        let path = ngtcp2_path {
            local: ngtcp2_addr {
                addr: &local_sockaddr as *const _ as *mut _,
                addrlen: local_len,
            },
            remote: ngtcp2_addr {
                addr: &remote_sockaddr as *const _ as *mut _,
                addrlen: remote_len,
            },
            user_data: ptr::null_mut(),
        };

        let rv = unsafe {
            ngtcp2_conn_server_new_versioned(
                &mut conn,
                &dcid_raw,
                &scid_raw,
                &path,
                version,
                NGTCP2_CALLBACKS_VERSION as c_int,
                callbacks,
                NGTCP2_SETTINGS_VERSION as c_int,
                settings,
                NGTCP2_TRANSPORT_PARAMS_VERSION as c_int,
                params,
                ptr::null(),
                user_data,
            )
        };

        check_ngtcp2(rv)?;

        let user_data_box = Box::new(ConnectionUserData {
            stream_data_queue: VecDeque::new(),
            datagram_queue: VecDeque::new(),
            issued_cids: Vec::new(),
            h3_conn_ptr: ptr::null_mut(),
        });

        Ok(Self {
            inner: conn,
            user_data: user_data_box,
            _tls_session: None,
            _conn_ref: None,
        })
    }

    /// クライアント接続を作成 (高レベル API)
    ///
    /// ngtcp2_crypto コールバックを自動設定し、TLS セッションを管理する。
    ///
    /// # Arguments
    ///
    /// * `dcid` - 宛先コネクション ID
    /// * `scid` - 送信元コネクション ID
    /// * `local_addr` - ローカルアドレス
    /// * `remote_addr` - リモートアドレス
    /// * `server_name` - サーバー名 (SNI 兼ホスト名検証、DNS 名限定)
    /// * `tls_session` - TLS セッション
    /// * `params` - トランスポートパラメータ
    /// * `initial_ts` - 初期タイムスタンプ (ナノ秒)
    #[expect(clippy::too_many_arguments)]
    pub fn client_new(
        dcid: &ConnectionId,
        scid: &ConnectionId,
        local_addr: SocketAddr,
        remote_addr: SocketAddr,
        server_name: &str,
        mut tls_session: TlsSession,
        params: &ngtcp2_transport_params,
        initial_ts: u64,
    ) -> Result<Self> {
        // SNI を設定
        tls_session.set_server_name(server_name)?;

        // コールバックを設定
        let callbacks = create_client_callbacks();

        // 設定を作成
        let mut settings: ngtcp2_settings = unsafe { std::mem::zeroed() };
        unsafe {
            ngtcp2_settings_default_versioned(NGTCP2_SETTINGS_VERSION as c_int, &mut settings);
        }
        settings.initial_ts = initial_ts;
        settings.max_tx_udp_payload_size = 1350;

        // user_data を作成
        let mut user_data_box = Box::new(ConnectionUserData {
            stream_data_queue: VecDeque::new(),
            datagram_queue: VecDeque::new(),
            issued_cids: Vec::new(),
            h3_conn_ptr: ptr::null_mut(),
        });
        let user_data_ptr = &mut *user_data_box as *mut ConnectionUserData as *mut c_void;

        let mut conn: *mut ngtcp2_conn = ptr::null_mut();

        let dcid_raw = cid_to_raw(dcid);
        let scid_raw = cid_to_raw(scid);

        let (local_sockaddr, local_len) = sockaddr_to_raw(&local_addr);
        let (remote_sockaddr, remote_len) = sockaddr_to_raw(&remote_addr);

        let path = ngtcp2_path {
            local: ngtcp2_addr {
                addr: &local_sockaddr as *const _ as *mut _,
                addrlen: local_len,
            },
            remote: ngtcp2_addr {
                addr: &remote_sockaddr as *const _ as *mut _,
                addrlen: remote_len,
            },
            user_data: ptr::null_mut(),
        };

        let rv = unsafe {
            ngtcp2_conn_client_new_versioned(
                &mut conn,
                &dcid_raw,
                &scid_raw,
                &path,
                QuicVersion::V1 as u32,
                NGTCP2_CALLBACKS_VERSION as c_int,
                &callbacks,
                NGTCP2_SETTINGS_VERSION as c_int,
                &settings,
                NGTCP2_TRANSPORT_PARAMS_VERSION as c_int,
                params,
                ptr::null(),
                user_data_ptr,
            )
        };

        check_ngtcp2(rv)?;

        // conn_ref を作成 (TLS コールバックから ngtcp2_conn を取得するために必要)
        let mut conn_ref = Box::new(ConnRef {
            inner: ngtcp2_crypto_conn_ref {
                get_conn: Some(conn_ref_get_conn_callback),
                user_data: conn as *mut c_void,
            },
        });

        // SSL に conn_ref を設定
        // SSL_set_app_data は SSL_set_ex_data(ssl, 0, data) のマクロ
        let conn_ref_ptr = &mut conn_ref.inner as *mut ngtcp2_crypto_conn_ref;
        unsafe {
            aws_lc_sys::SSL_set_ex_data(tls_session.as_ptr(), 0, conn_ref_ptr as *mut c_void);
        }

        // TLS ネイティブハンドルを設定
        unsafe {
            ngtcp2_conn_set_tls_native_handle(conn, tls_session.as_void_ptr());
        }

        // クライアントのトランスポートパラメータを TLS に設定
        //
        // ngtcp2_crypto の client_initial_cb でも設定されるが、
        // aws-lc の動作確認のため事前に設定しておく。
        let mut tp_buf = [0u8; 512];
        let tp_len = unsafe {
            ngtcp2_conn_encode_local_transport_params(conn, tp_buf.as_mut_ptr(), tp_buf.len())
        };
        if tp_len < 0 {
            unsafe { ngtcp2_conn_del(conn) };
            return Err(Error::from_ngtcp2(tp_len as i32));
        }
        if let Err(e) = tls_session.set_quic_transport_params(&tp_buf[..tp_len as usize]) {
            unsafe { ngtcp2_conn_del(conn) };
            return Err(e);
        }

        Ok(Self {
            inner: conn,
            user_data: user_data_box,
            _tls_session: Some(tls_session),
            _conn_ref: Some(conn_ref),
        })
    }

    /// サーバー接続を作成 (高レベル API)
    ///
    /// ngtcp2_crypto コールバックを自動設定し、TLS セッションを管理する。
    ///
    /// # Arguments
    ///
    /// * `dcid` - 宛先コネクション ID (クライアントから受信した SCID)
    /// * `scid` - 送信元コネクション ID (サーバーが生成)
    /// * `local_addr` - ローカルアドレス
    /// * `remote_addr` - リモートアドレス
    /// * `tls_session` - TLS セッション
    /// * `params` - トランスポートパラメータ
    /// * `initial_ts` - 初期タイムスタンプ (ナノ秒)
    pub fn server_new(
        dcid: &ConnectionId,
        scid: &ConnectionId,
        local_addr: SocketAddr,
        remote_addr: SocketAddr,
        mut tls_session: TlsSession,
        params: &ngtcp2_transport_params,
        initial_ts: u64,
    ) -> Result<Self> {
        // コールバックを設定
        let callbacks = create_server_callbacks();

        // 設定を作成
        let mut settings: ngtcp2_settings = unsafe { std::mem::zeroed() };
        unsafe {
            ngtcp2_settings_default_versioned(NGTCP2_SETTINGS_VERSION as c_int, &mut settings);
        }
        settings.initial_ts = initial_ts;
        settings.max_tx_udp_payload_size = 1350;

        // user_data を作成
        let mut user_data_box = Box::new(ConnectionUserData {
            stream_data_queue: VecDeque::new(),
            datagram_queue: VecDeque::new(),
            issued_cids: Vec::new(),
            h3_conn_ptr: ptr::null_mut(),
        });
        let user_data_ptr = &mut *user_data_box as *mut ConnectionUserData as *mut c_void;

        let mut conn: *mut ngtcp2_conn = ptr::null_mut();

        let dcid_raw = cid_to_raw(dcid);
        let scid_raw = cid_to_raw(scid);

        let (local_sockaddr, local_len) = sockaddr_to_raw(&local_addr);
        let (remote_sockaddr, remote_len) = sockaddr_to_raw(&remote_addr);

        let path = ngtcp2_path {
            local: ngtcp2_addr {
                addr: &local_sockaddr as *const _ as *mut _,
                addrlen: local_len,
            },
            remote: ngtcp2_addr {
                addr: &remote_sockaddr as *const _ as *mut _,
                addrlen: remote_len,
            },
            user_data: ptr::null_mut(),
        };

        let rv = unsafe {
            ngtcp2_conn_server_new_versioned(
                &mut conn,
                &dcid_raw,
                &scid_raw,
                &path,
                QuicVersion::V1 as u32,
                NGTCP2_CALLBACKS_VERSION as c_int,
                &callbacks,
                NGTCP2_SETTINGS_VERSION as c_int,
                &settings,
                NGTCP2_TRANSPORT_PARAMS_VERSION as c_int,
                params,
                ptr::null(),
                user_data_ptr,
            )
        };

        check_ngtcp2(rv)?;

        // conn_ref を作成 (TLS コールバックから ngtcp2_conn を取得するために必要)
        let mut conn_ref = Box::new(ConnRef {
            inner: ngtcp2_crypto_conn_ref {
                get_conn: Some(conn_ref_get_conn_callback),
                user_data: conn as *mut c_void,
            },
        });

        // SSL に conn_ref を設定
        // SSL_set_app_data は SSL_set_ex_data(ssl, 0, data) のマクロ
        unsafe {
            aws_lc_sys::SSL_set_ex_data(
                tls_session.as_ptr(),
                0,
                &mut conn_ref.inner as *mut ngtcp2_crypto_conn_ref as *mut c_void,
            );
        }

        // TLS ネイティブハンドルを設定
        unsafe {
            ngtcp2_conn_set_tls_native_handle(conn, tls_session.as_void_ptr());
        }

        // サーバーのトランスポートパラメータを TLS に設定
        //
        // aws-lc では、SSL_set_quic_transport_params が設定されていないと、
        // ClientHello の quic_transport_parameters 拡張が無視される。
        // これは aws-lc の ext_quic_transport_params_parse_clienthello の実装による:
        // hs->config->quic_transport_params が空の場合、クライアントのパラメータは
        // 保存されず、後で SSL_get_peer_quic_transport_params が空のデータを返す。
        //
        // ngtcp2_crypto は通常、HANDSHAKE 鍵のインストール時にサーバーのパラメータを
        // 設定するが、これは ClientHello 処理の後になる。
        // そのため、ここで事前に設定する必要がある。
        let mut tp_buf = [0u8; 512];
        let tp_len = unsafe {
            ngtcp2_conn_encode_local_transport_params(conn, tp_buf.as_mut_ptr(), tp_buf.len())
        };
        if tp_len < 0 {
            unsafe { ngtcp2_conn_del(conn) };
            return Err(Error::from_ngtcp2(tp_len as i32));
        }
        if let Err(e) = tls_session.set_quic_transport_params(&tp_buf[..tp_len as usize]) {
            unsafe { ngtcp2_conn_del(conn) };
            return Err(e);
        }

        Ok(Self {
            inner: conn,
            user_data: user_data_box,
            _tls_session: Some(tls_session),
            _conn_ref: Some(conn_ref),
        })
    }

    /// パケットを読み込む
    pub fn read_pkt(
        &mut self,
        local_addr: &SocketAddr,
        remote_addr: &SocketAddr,
        pkt_info: &PacketInfo,
        data: &[u8],
        ts: u64,
    ) -> Result<()> {
        let (local_sockaddr, local_len) = sockaddr_to_raw(local_addr);
        let (remote_sockaddr, remote_len) = sockaddr_to_raw(remote_addr);

        let path = ngtcp2_path {
            local: ngtcp2_addr {
                addr: &local_sockaddr as *const _ as *mut _,
                addrlen: local_len,
            },
            remote: ngtcp2_addr {
                addr: &remote_sockaddr as *const _ as *mut _,
                addrlen: remote_len,
            },
            user_data: ptr::null_mut(),
        };

        let pi = ngtcp2_pkt_info { ecn: pkt_info.ecn };

        let rv = unsafe {
            ngtcp2_conn_read_pkt_versioned(
                self.inner,
                &path,
                NGTCP2_PKT_INFO_VERSION as c_int,
                &pi,
                data.as_ptr(),
                data.len(),
                ts,
            )
        };

        check_ngtcp2(rv as c_int)
    }

    /// パケットを書き込む
    pub fn write_pkt(&mut self, buf: &mut [u8], ts: u64) -> Result<(usize, PacketInfo)> {
        let mut pi = ngtcp2_pkt_info { ecn: 0 };

        // path に有効なバッファを設定する必要がある
        // ngtcp2 は path に出力パス情報を書き込むため、
        // addr フィールドに有効なバッファを設定する必要がある
        let mut local_addr: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
        let mut remote_addr: libc::sockaddr_storage = unsafe { std::mem::zeroed() };

        let mut path = ngtcp2_path {
            local: ngtcp2_addr {
                addr: &mut local_addr as *mut _ as *mut _,
                addrlen: std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t,
            },
            remote: ngtcp2_addr {
                addr: &mut remote_addr as *mut _ as *mut _,
                addrlen: std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t,
            },
            user_data: ptr::null_mut(),
        };

        let rv = unsafe {
            ngtcp2_conn_write_pkt_versioned(
                self.inner,
                &mut path,
                NGTCP2_PKT_INFO_VERSION as c_int,
                &mut pi,
                buf.as_mut_ptr(),
                buf.len(),
                ts,
            )
        };

        if rv < 0 {
            return Err(Error::from_ngtcp2(rv as c_int));
        }

        Ok((rv as usize, PacketInfo { ecn: pi.ecn }))
    }

    /// ストリームにデータを書き込む
    ///
    /// ngtcp2 examples に従い、NGTCP2_WRITE_STREAM_FLAG_MORE を使用する。
    /// これにより複数のストリームデータを 1 つのパケットにまとめることができる。
    ///
    /// # Returns
    ///
    /// - `Ok((pkt_written, Some(data_written)))`: パケットが生成された、またはデータがバッファに追加された
    /// - `Err(StreamDataBlocked(stream_id))`: ストリームがフロー制御でブロック
    /// - `Err(StreamShutWr(stream_id))`: ストリームの書き込みがシャットダウン
    pub fn write_stream(
        &mut self,
        buf: &mut [u8],
        stream_id: StreamId,
        data: &[u8],
        fin: bool,
        ts: u64,
    ) -> Result<(usize, Option<usize>)> {
        let mut pi = ngtcp2_pkt_info { ecn: 0 };
        let mut datalen: ngtcp2_ssize = -1;

        // path に有効なバッファを設定する必要がある
        // ngtcp2 は path に出力パス情報を書き込むため、
        // addr フィールドに有効なバッファを設定する必要がある
        let mut local_addr: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
        let mut remote_addr: libc::sockaddr_storage = unsafe { std::mem::zeroed() };

        let mut path = ngtcp2_path {
            local: ngtcp2_addr {
                addr: &mut local_addr as *mut _ as *mut _,
                addrlen: std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t,
            },
            remote: ngtcp2_addr {
                addr: &mut remote_addr as *mut _ as *mut _,
                addrlen: std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t,
            },
            user_data: ptr::null_mut(),
        };

        let vec = ngtcp2_vec {
            base: data.as_ptr() as *mut _,
            len: data.len(),
        };

        // ngtcp2 examples に従い、NGTCP2_WRITE_STREAM_FLAG_MORE を使用
        let mut flags = ngtcp2_sys::NGTCP2_WRITE_STREAM_FLAG_MORE;
        if fin {
            flags |= ngtcp2_sys::NGTCP2_WRITE_STREAM_FLAG_FIN;
        }

        let rv = unsafe {
            ngtcp2_conn_writev_stream_versioned(
                self.inner,
                &mut path,
                NGTCP2_PKT_INFO_VERSION as c_int,
                &mut pi,
                buf.as_mut_ptr(),
                buf.len(),
                &mut datalen,
                flags,
                stream_id,
                &vec,
                1,
                ts,
            )
        };

        // ngtcp2 examples に従い、特定のエラーを識別可能にする
        if rv == ngtcp2_sys::NGTCP2_ERR_WRITE_MORE as ngtcp2_ssize {
            // データがバッファに追加されたがパケットはまだ生成されていない
            let data_written = if datalen >= 0 {
                Some(datalen as usize)
            } else {
                None
            };
            return Ok((0, data_written));
        }

        if rv == ngtcp2_sys::NGTCP2_ERR_STREAM_DATA_BLOCKED as ngtcp2_ssize {
            return Err(Error::StreamDataBlocked(stream_id));
        }

        if rv == ngtcp2_sys::NGTCP2_ERR_STREAM_SHUT_WR as ngtcp2_ssize {
            return Err(Error::StreamShutWr(stream_id));
        }

        if rv < 0 {
            return Err(Error::from_ngtcp2(rv as c_int));
        }

        let data_written = if datalen >= 0 {
            Some(datalen as usize)
        } else {
            None
        };

        Ok((rv as usize, data_written))
    }

    /// 双方向ストリームを開く
    pub fn open_bidi_stream(&mut self) -> Result<StreamId> {
        let mut stream_id: i64 = 0;
        let rv =
            unsafe { ngtcp2_conn_open_bidi_stream(self.inner, &mut stream_id, ptr::null_mut()) };
        check_ngtcp2(rv)?;
        Ok(stream_id)
    }

    /// 単方向ストリームを開く
    pub fn open_uni_stream(&mut self) -> Result<StreamId> {
        let mut stream_id: i64 = 0;
        let rv =
            unsafe { ngtcp2_conn_open_uni_stream(self.inner, &mut stream_id, ptr::null_mut()) };
        check_ngtcp2(rv)?;
        Ok(stream_id)
    }

    /// ストリームをシャットダウン (双方向)
    pub fn shutdown_stream(&mut self, stream_id: StreamId, error_code: u64) -> Result<()> {
        let rv = unsafe { ngtcp2_conn_shutdown_stream(self.inner, 0, stream_id, error_code) };
        check_ngtcp2(rv)
    }

    /// ストリームの書き込み側をシャットダウン (FIN 送信)
    pub fn shutdown_stream_write(&mut self, stream_id: StreamId, error_code: u64) -> Result<()> {
        let rv = unsafe { ngtcp2_conn_shutdown_stream_write(self.inner, 0, stream_id, error_code) };
        check_ngtcp2(rv)
    }

    /// ストリームの最大オフセットを拡張
    pub fn extend_max_stream_offset(&mut self, stream_id: StreamId, datalen: u64) -> Result<()> {
        let rv = unsafe { ngtcp2_conn_extend_max_stream_offset(self.inner, stream_id, datalen) };
        check_ngtcp2(rv)
    }

    /// 接続の最大オフセットを拡張
    pub fn extend_max_offset(&mut self, datalen: u64) {
        unsafe { ngtcp2_conn_extend_max_offset(self.inner, datalen) };
    }

    /// 次の有効期限を取得
    pub fn get_expiry(&self) -> u64 {
        unsafe { ngtcp2_conn_get_expiry(self.inner) }
    }

    /// タイムアウトを処理
    pub fn handle_expiry(&mut self, ts: u64) -> Result<()> {
        let rv = unsafe { ngtcp2_conn_handle_expiry(self.inner, ts) };
        check_ngtcp2(rv)
    }

    /// クロージング期間中かどうか
    pub fn is_in_closing_period(&self) -> bool {
        unsafe { ngtcp2_conn_in_closing_period(self.inner) != 0 }
    }

    /// ドレイニング期間中かどうか
    pub fn is_in_draining_period(&self) -> bool {
        unsafe { ngtcp2_conn_in_draining_period(self.inner) != 0 }
    }

    /// ハンドシェイクが完了したかどうか
    pub fn is_handshake_completed(&self) -> bool {
        unsafe { ngtcp2_conn_get_handshake_completed(self.inner) != 0 }
    }

    /// TLS ネイティブハンドルを設定
    ///
    /// # Safety
    /// handle は有効な TLS ネイティブハンドルポインタである必要がある
    pub unsafe fn set_tls_native_handle(&mut self, handle: *mut c_void) {
        unsafe { ngtcp2_conn_set_tls_native_handle(self.inner, handle) };
    }

    /// TLS ネイティブハンドルを取得
    pub fn get_tls_native_handle(&self) -> *mut c_void {
        unsafe { ngtcp2_conn_get_tls_native_handle(self.inner) }
    }

    /// keep-alive タイムアウトを設定
    pub fn set_keep_alive_timeout(&mut self, timeout: u64) {
        unsafe { ngtcp2_conn_set_keep_alive_timeout(self.inner, timeout) };
    }

    /// 残りの最大データ量を取得
    pub fn get_max_data_left(&self) -> u64 {
        unsafe { ngtcp2_conn_get_max_data_left(self.inner) }
    }

    /// 残りの双方向ストリーム数を取得
    pub fn get_streams_bidi_left(&self) -> u64 {
        unsafe { ngtcp2_conn_get_streams_bidi_left(self.inner) }
    }

    /// 残りの単方向ストリーム数を取得
    pub fn get_streams_uni_left(&self) -> u64 {
        unsafe { ngtcp2_conn_get_streams_uni_left(self.inner) }
    }

    /// 受信したストリームデータを取得
    ///
    /// キューから1つのストリームデータを取り出す。
    /// データがない場合は None を返す。
    pub fn poll_stream_data(&mut self) -> Option<StreamData> {
        self.user_data.stream_data_queue.pop_front()
    }

    /// 受信したストリームデータがあるかどうか
    pub fn has_stream_data(&self) -> bool {
        !self.user_data.stream_data_queue.is_empty()
    }

    /// リモートピアの max_datagram_frame_size を取得
    ///
    /// リモートピアが DATAGRAM をサポートしている場合、そのサイズを返す。
    /// サポートしていない場合や、トランスポートパラメータがまだ交換されていない場合は 0 を返す。
    pub fn get_remote_max_datagram_frame_size(&self) -> u64 {
        let params = unsafe { ngtcp2_conn_get_remote_transport_params(self.inner) };
        if params.is_null() {
            return 0;
        }
        unsafe { (*params).max_datagram_frame_size }
    }

    /// ローカルの max_datagram_frame_size を取得
    ///
    /// ローカルが DATAGRAM をサポートしている場合、そのサイズを返す。
    pub fn get_local_max_datagram_frame_size(&self) -> u64 {
        let params = unsafe { ngtcp2_conn_get_local_transport_params(self.inner) };
        if params.is_null() {
            return 0;
        }
        unsafe { (*params).max_datagram_frame_size }
    }

    /// リモートピアが DATAGRAM をサポートしているかどうかを確認
    ///
    /// DATAGRAM を送信する前にこの関数でサポートを確認することを推奨する。
    pub fn can_send_datagram(&self) -> bool {
        self.get_remote_max_datagram_frame_size() > 0
    }

    /// DATAGRAM を送信
    ///
    /// QUIC DATAGRAM フレームでデータを送信する。
    /// DATAGRAM は信頼性のない配信であり、順序も保証されない。
    ///
    /// # Arguments
    ///
    /// * `buf` - 出力バッファ (パケット用)
    /// * `data` - 送信するデータ
    /// * `ts` - タイムスタンプ (ナノ秒)
    ///
    /// # Returns
    ///
    /// * `Ok((written, accepted))` - written: パケットサイズ, accepted: データが受け入れられたか
    ///
    /// # Errors
    ///
    /// * リモートピアが DATAGRAM をサポートしていない場合は ERR_INVALID_STATE を返す
    pub fn write_datagram(
        &mut self,
        buf: &mut [u8],
        data: &[u8],
        ts: u64,
    ) -> Result<(usize, bool)> {
        // リモートピアが DATAGRAM をサポートしているか確認
        if !self.can_send_datagram() {
            return Err(Error::Ngtcp2(
                "ERR_INVALID_STATE: remote peer does not support DATAGRAM".to_string(),
                NGTCP2_ERR_INVALID_STATE,
            ));
        }

        // inner が null でないことを確認
        if self.inner.is_null() {
            return Err(Error::Internal("connection is null".to_string()));
        }

        let mut pi = ngtcp2_pkt_info { ecn: 0 };
        let mut accepted: c_int = 0;

        let vec = ngtcp2_vec {
            base: data.as_ptr() as *mut _,
            len: data.len(),
        };

        // path に NULL を渡す (パス情報が不要な場合)
        // ngtcp2_path を zeroed で初期化すると内部ポインタが NULL になり SIGSEGV が発生する
        let rv = unsafe {
            ngtcp2_conn_writev_datagram_versioned(
                self.inner,
                ptr::null_mut(), // path は不要なので NULL
                NGTCP2_PKT_INFO_VERSION as c_int,
                &mut pi,
                buf.as_mut_ptr(),
                buf.len(),
                &mut accepted,
                NGTCP2_WRITE_DATAGRAM_FLAG_NONE,
                0, // dgram_id
                &vec,
                1,
                ts,
            )
        };

        if rv < 0 {
            return Err(Error::from_ngtcp2(rv as c_int));
        }

        Ok((rv as usize, accepted != 0))
    }

    /// 受信した DATAGRAM を取得
    ///
    /// キューから1つの DATAGRAM を取り出す。
    /// データがない場合は None を返す。
    pub fn poll_datagram(&mut self) -> Option<Datagram> {
        self.user_data.datagram_queue.pop_front()
    }

    /// 受信した DATAGRAM があるかどうか
    pub fn has_datagram(&self) -> bool {
        !self.user_data.datagram_queue.is_empty()
    }

    /// NEW_CONNECTION_ID フレームで発行した CID を取り出す
    ///
    /// ngtcp2 はピアの `active_connection_id_limit` に応じて追加の CID を発行する。
    /// サーバー実装は発行済み CID をルーティングテーブルに登録し、ピアが DCID として
    /// 使用するパケットを正しく接続に振り分ける (RFC 9000 Section 5.1.1)。
    /// 一度取り出した CID は再度返さない。
    pub fn poll_issued_cids(&mut self) -> Vec<ConnectionId> {
        std::mem::take(&mut self.user_data.issued_cids)
    }

    /// トランスポートエラーの CONNECTION_CLOSE パケットを書き込む
    ///
    /// QUIC 接続を閉じるために使用する。
    /// トランスポートエラーコードを含む CONNECTION_CLOSE フレームを生成する。
    ///
    /// # Arguments
    ///
    /// * `buf` - 出力バッファ
    /// * `error_code` - トランスポートエラーコード (例: NGTCP2_NO_ERROR)
    /// * `reason` - エラー理由 (空でも可)
    /// * `ts` - タイムスタンプ (ナノ秒)
    pub fn write_connection_close(
        &mut self,
        buf: &mut [u8],
        error_code: u64,
        reason: &[u8],
        ts: u64,
    ) -> Result<usize> {
        let mut ccerr: ngtcp2_ccerr = unsafe { std::mem::zeroed() };
        unsafe {
            ngtcp2_ccerr_default(&mut ccerr);
            ngtcp2_ccerr_set_transport_error(&mut ccerr, error_code, reason.as_ptr(), reason.len());
        }

        let mut pi = ngtcp2_pkt_info { ecn: 0 };

        let rv = unsafe {
            ngtcp2_conn_write_connection_close_versioned(
                self.inner,
                ptr::null_mut(),
                NGTCP2_PKT_INFO_VERSION as c_int,
                &mut pi,
                buf.as_mut_ptr(),
                buf.len(),
                &ccerr,
                ts,
            )
        };

        if rv < 0 {
            return Err(Error::from_ngtcp2(rv as c_int));
        }

        Ok(rv as usize)
    }

    /// アプリケーションエラーの CONNECTION_CLOSE パケットを書き込む
    ///
    /// HTTP/3 等のアプリケーションレイヤーエラーで接続を閉じる場合に使用する。
    ///
    /// # Arguments
    ///
    /// * `buf` - 出力バッファ
    /// * `error_code` - アプリケーションエラーコード
    /// * `reason` - エラー理由 (空でも可)
    /// * `ts` - タイムスタンプ (ナノ秒)
    pub fn write_connection_close_app(
        &mut self,
        buf: &mut [u8],
        error_code: u64,
        reason: &[u8],
        ts: u64,
    ) -> Result<usize> {
        let mut ccerr: ngtcp2_ccerr = unsafe { std::mem::zeroed() };
        unsafe {
            ngtcp2_ccerr_default(&mut ccerr);
            ngtcp2_ccerr_set_application_error(
                &mut ccerr,
                error_code,
                reason.as_ptr(),
                reason.len(),
            );
        }

        let mut pi = ngtcp2_pkt_info { ecn: 0 };

        let rv = unsafe {
            ngtcp2_conn_write_connection_close_versioned(
                self.inner,
                ptr::null_mut(),
                NGTCP2_PKT_INFO_VERSION as c_int,
                &mut pi,
                buf.as_mut_ptr(),
                buf.len(),
                &ccerr,
                ts,
            )
        };

        if rv < 0 {
            return Err(Error::from_ngtcp2(rv as c_int));
        }

        Ok(rv as usize)
    }

    /// 内部ポインタを取得
    pub fn as_ptr(&self) -> *mut ngtcp2_conn {
        self.inner
    }

    /// nghttp3_conn へのポインタをユーザーデータに設定する
    ///
    /// `acked_stream_data_offset` コールバック (ピアに ACK されたストリームデータの
    /// 範囲通知) から [`Http3Connection`](crate::Http3Connection) の
    /// `nghttp3_conn_add_ack_offset` を呼び出すために使用する。
    ///
    /// # Safety
    ///
    /// `ptr` は [`Http3Connection`](crate::Http3Connection) を所有しており、
    /// `set_h3_conn_null` を呼ぶか `Connection` が破棄されるまで有効でなければならない。
    /// ポインタの行き先は同期ロックを一切持たないため、`Connection` の全メソッドは
    /// 単一スレッドから呼び出されること。
    pub unsafe fn set_h3_conn_ptr(&mut self, ptr: *mut c_void) {
        self.user_data.h3_conn_ptr = ptr;
    }

    /// nghttp3_conn へのポインタをクリアする
    ///
    /// [`Http3Connection`](crate::Http3Connection) が破棄された場合に呼び、以後の
    /// コールバックから nghttp3_conn にアクセスしないようにする。
    pub fn set_h3_conn_null(&mut self) {
        self.user_data.h3_conn_ptr = ptr::null_mut();
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        if !self.inner.is_null() {
            unsafe { ngtcp2_conn_del(self.inner) };
        }
    }
}

/// ConnectionId を ngtcp2_cid に変換
fn cid_to_raw(cid: &ConnectionId) -> ngtcp2_cid {
    let mut raw = ngtcp2_cid {
        datalen: cid.len(),
        data: [0u8; 20],
    };
    raw.data[..cid.len()].copy_from_slice(cid.as_bytes());
    raw
}

/// SocketAddr を sockaddr_storage に変換
fn sockaddr_to_raw(addr: &SocketAddr) -> (libc::sockaddr_storage, libc::socklen_t) {
    let mut storage: libc::sockaddr_storage = unsafe { std::mem::zeroed() };

    match addr {
        SocketAddr::V4(v4) => {
            let sin: &mut libc::sockaddr_in =
                unsafe { &mut *(&mut storage as *mut _ as *mut libc::sockaddr_in) };
            sin.sin_family = libc::AF_INET as libc::sa_family_t;
            sin.sin_port = v4.port().to_be();
            sin.sin_addr.s_addr = u32::from_ne_bytes(v4.ip().octets());
            (
                storage,
                std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
            )
        }
        SocketAddr::V6(v6) => {
            let sin6: &mut libc::sockaddr_in6 =
                unsafe { &mut *(&mut storage as *mut _ as *mut libc::sockaddr_in6) };
            sin6.sin6_family = libc::AF_INET6 as libc::sa_family_t;
            sin6.sin6_port = v6.port().to_be();
            sin6.sin6_addr.s6_addr = v6.ip().octets();
            sin6.sin6_flowinfo = v6.flowinfo();
            sin6.sin6_scope_id = v6.scope_id();
            (
                storage,
                std::mem::size_of::<libc::sockaddr_in6>() as libc::socklen_t,
            )
        }
    }
}

/// クライアント用コールバックを作成
fn create_client_callbacks() -> ngtcp2_callbacks {
    let mut callbacks: ngtcp2_callbacks = unsafe { std::mem::zeroed() };

    // ngtcp2_crypto_* コールバック (必須)
    callbacks.client_initial = Some(ngtcp2_crypto_client_initial_cb);
    callbacks.recv_crypto_data = Some(ngtcp2_crypto_recv_crypto_data_cb);
    callbacks.encrypt = Some(ngtcp2_crypto_encrypt_cb);
    callbacks.decrypt = Some(ngtcp2_crypto_decrypt_cb);
    callbacks.hp_mask = Some(ngtcp2_crypto_hp_mask_cb);
    callbacks.recv_retry = Some(ngtcp2_crypto_recv_retry_cb);
    callbacks.update_key = Some(ngtcp2_crypto_update_key_cb);
    callbacks.delete_crypto_aead_ctx = Some(ngtcp2_crypto_delete_crypto_aead_ctx_cb);
    callbacks.delete_crypto_cipher_ctx = Some(ngtcp2_crypto_delete_crypto_cipher_ctx_cb);
    callbacks.get_path_challenge_data = Some(ngtcp2_crypto_get_path_challenge_data_cb);
    callbacks.version_negotiation = Some(ngtcp2_crypto_version_negotiation_cb);

    // ストリームデータ受信コールバック
    callbacks.recv_stream_data = Some(recv_stream_data_callback);

    // ACK 済みストリームデータオフセットコールバック
    callbacks.acked_stream_data_offset = Some(acked_stream_data_offset_callback);

    // DATAGRAM 受信コールバック
    callbacks.recv_datagram = Some(recv_datagram_callback);

    // その他の必須コールバック
    callbacks.rand = Some(rand_callback);
    callbacks.get_new_connection_id = Some(get_new_connection_id_callback);

    callbacks
}

/// サーバー用コールバックを作成
fn create_server_callbacks() -> ngtcp2_callbacks {
    let mut callbacks: ngtcp2_callbacks = unsafe { std::mem::zeroed() };

    // ngtcp2_crypto_* コールバック (必須)
    callbacks.recv_client_initial = Some(ngtcp2_crypto_recv_client_initial_cb);
    callbacks.recv_crypto_data = Some(ngtcp2_crypto_recv_crypto_data_cb);
    callbacks.encrypt = Some(ngtcp2_crypto_encrypt_cb);
    callbacks.decrypt = Some(ngtcp2_crypto_decrypt_cb);
    callbacks.hp_mask = Some(ngtcp2_crypto_hp_mask_cb);
    callbacks.update_key = Some(ngtcp2_crypto_update_key_cb);
    callbacks.delete_crypto_aead_ctx = Some(ngtcp2_crypto_delete_crypto_aead_ctx_cb);
    callbacks.delete_crypto_cipher_ctx = Some(ngtcp2_crypto_delete_crypto_cipher_ctx_cb);
    callbacks.get_path_challenge_data = Some(ngtcp2_crypto_get_path_challenge_data_cb);
    callbacks.version_negotiation = Some(ngtcp2_crypto_version_negotiation_cb);

    // ストリームデータ受信コールバック
    callbacks.recv_stream_data = Some(recv_stream_data_callback);

    // ACK 済みストリームデータオフセットコールバック
    callbacks.acked_stream_data_offset = Some(acked_stream_data_offset_callback);

    // DATAGRAM 受信コールバック
    callbacks.recv_datagram = Some(recv_datagram_callback);

    // その他の必須コールバック
    callbacks.rand = Some(rand_callback);
    callbacks.get_new_connection_id = Some(get_new_connection_id_callback);

    callbacks
}

/// 乱数生成コールバック
unsafe extern "C" fn rand_callback(buf: *mut u8, buflen: usize, _rand_ctx: *const ngtcp2_rand_ctx) {
    // SAFETY: buf は呼び出し元から渡された有効なポインタ
    let slice = unsafe { std::slice::from_raw_parts_mut(buf, buflen) };
    let _ = aws_lc_rs::rand::fill(slice);
}

/// 新しいコネクション ID 生成コールバック
unsafe extern "C" fn get_new_connection_id_callback(
    _conn: *mut ngtcp2_conn,
    cid: *mut ngtcp2_cid,
    token: *mut u8,
    cidlen: usize,
    user_data: *mut c_void,
) -> c_int {
    // SAFETY: cid と token は呼び出し元から渡された有効なポインタ
    unsafe {
        // コネクション ID を生成
        let cid_slice = std::slice::from_raw_parts_mut((*cid).data.as_mut_ptr(), cidlen);
        if aws_lc_rs::rand::fill(cid_slice).is_err() {
            return NGTCP2_ERR_CALLBACK_FAILURE;
        }
        (*cid).datalen = cidlen;

        // トークンを生成
        let token_slice =
            std::slice::from_raw_parts_mut(token, NGTCP2_STATELESS_RESET_TOKENLEN as usize);
        if aws_lc_rs::rand::fill(token_slice).is_err() {
            return NGTCP2_ERR_CALLBACK_FAILURE;
        }

        // 発行した CID を記録する
        // サーバー実装はこの CID をルーティングテーブルに登録し、
        // ピアが DCID として使うパケットを接続に振り分ける (RFC 9000 Section 5.1.1)。
        if !user_data.is_null() {
            let conn_user_data = &mut *(user_data as *mut ConnectionUserData);
            let cid_slice = std::slice::from_raw_parts((*cid).data.as_ptr(), cidlen);
            if let Some(issued_cid) = ConnectionId::new(cid_slice) {
                conn_user_data.issued_cids.push(issued_cid);
            }
        }
    }

    0
}

/// conn_ref から ngtcp2_conn を取得するコールバック
///
/// TLS コールバック (add_handshake_data など) から呼び出される。
/// conn_ref.user_data に ngtcp2_conn へのポインタが保存されている。
unsafe extern "C" fn conn_ref_get_conn_callback(
    conn_ref: *mut ngtcp2_crypto_conn_ref,
) -> *mut ngtcp2_conn {
    // SAFETY: conn_ref は有効なポインタで、user_data には ngtcp2_conn へのポインタが保存されている
    unsafe { (*conn_ref).user_data as *mut ngtcp2_conn }
}

/// ストリームデータ受信コールバック
///
/// QUIC ストリームでデータを受信したときに呼び出される。
/// 受信したデータを user_data のキューに追加する。
/// ACK 済みストリームデータオフセットのコールバック
///
/// ピアに ACK されたストリームデータの範囲を通知する (RFC 9000 Section 19.3)。
/// nghttp3 は ACK されたデータサイズを `nghttp3_conn_add_ack_offset` で通知されることで
/// 送信バッファ内の ACK 済みデータ (要求されていない可能性のある) を
/// 資源解放できる (RFC 9204 Section 2.4 / nghttp3 の動作仕様)。
///
/// user_data には `ConnectionUserData` が渡され、`set_h3_conn_ptr` で設定された
/// nghttp3_conn ポインタを使って通知する。nghttp3_conn が未設定 (null) の場合は
/// 何もしない (Http3Connection が破棄済み)。
unsafe extern "C" fn acked_stream_data_offset_callback(
    _conn: *mut ngtcp2_conn,
    stream_id: i64,
    offset: u64,
    datalen: u64,
    user_data: *mut c_void,
    _stream_user_data: *mut c_void,
) -> c_int {
    if user_data.is_null() {
        return 0;
    }

    unsafe {
        let conn_user_data = &mut *(user_data as *mut ConnectionUserData);
        let h3_conn_ptr = conn_user_data.h3_conn_ptr;
        if h3_conn_ptr.is_null() {
            return 0;
        }
        nghttp3_conn_add_ack_offset(
            h3_conn_ptr as *mut nghttp3_conn,
            stream_id,
            offset + datalen,
        );
    }

    0
}

unsafe extern "C" fn recv_stream_data_callback(
    _conn: *mut ngtcp2_conn,
    _flags: u32,
    stream_id: i64,
    _offset: u64,
    data: *const u8,
    datalen: usize,
    user_data: *mut c_void,
    _stream_user_data: *mut c_void,
) -> c_int {
    if user_data.is_null() || (data.is_null() && datalen > 0) {
        return 0;
    }

    unsafe {
        let conn_user_data = &mut *(user_data as *mut ConnectionUserData);

        // データをコピーしてキューに追加
        let data_slice = if datalen > 0 {
            std::slice::from_raw_parts(data, datalen).to_vec()
        } else {
            Vec::new()
        };

        let fin = (_flags & NGTCP2_STREAM_DATA_FLAG_FIN) != 0;

        conn_user_data.stream_data_queue.push_back(StreamData {
            stream_id,
            data: data_slice,
            fin,
        });
    }

    0
}

/// DATAGRAM 受信コールバック
///
/// QUIC DATAGRAM フレームを受信したときに呼び出される。
/// 受信したデータを user_data のキューに追加する。
/// DATAGRAM 受信コールバック
///
/// QUIC DATAGRAM フレームを受信したときに呼び出される。
/// 受信したデータを user_data のキューに追加する。
unsafe extern "C" fn recv_datagram_callback(
    _conn: *mut ngtcp2_conn,
    _flags: u32,
    data: *const u8,
    datalen: usize,
    user_data: *mut c_void,
) -> c_int {
    if user_data.is_null() || (data.is_null() && datalen > 0) {
        return 0;
    }

    unsafe {
        let conn_user_data = &mut *(user_data as *mut ConnectionUserData);

        // データをコピーしてキューに追加
        let data_slice = if datalen > 0 {
            std::slice::from_raw_parts(data, datalen).to_vec()
        } else {
            Vec::new()
        };

        conn_user_data
            .datagram_queue
            .push_back(Datagram { data: data_slice });
    }

    0
}
