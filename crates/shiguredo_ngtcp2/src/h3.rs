use std::collections::{HashMap, VecDeque};
use std::ffi::c_void;
use std::ptr;

use libc::c_int;
use nghttp3_sys::*;

use crate::error::{Error, Result, check_nghttp3};
use crate::types::{Header, Http3Event, SessionId, StreamId};

/// HTTP/3 接続
pub struct Http3Connection {
    inner: *mut nghttp3_conn,
    // イベントキュー
    // Box で VecDeque のメタデータをヒープに固定し、
    // Http3Connection が move されてもポインタが無効にならないことを保証する。
    // VecDeque で FIFO 順序を保証する。
    #[allow(clippy::box_collection)]
    events: Box<VecDeque<Http3Event>>,
    // コールバック用のユーザーデータ
    user_data: Box<Http3UserData>,
}

struct Http3UserData {
    events: *mut VecDeque<Http3Event>,
    wt_send_queues: HashMap<i64, WtSendQueue>,
    request_body_queues: HashMap<i64, RequestBodyQueue>,
}

/// HTTP/3 リクエストボディ送信キュー
struct RequestBodyQueue {
    data: Vec<u8>,
    offset: usize,
    fin: bool,
}

/// WebTransport ストリーム送信キュー
struct WtSendQueue {
    data: Vec<u8>,
    offset: usize,
    fin: bool,
}

/// ストリームに紐づくボディデータ
///
/// stream_user_data として使用され、data_reader コールバックでボディを返す。
/// リクエストボディとレスポンスボディの両方で共通の型を使用することで、
/// on_stream_close コールバックでの型安全な解放を保証する。
struct StreamBodyData {
    data: Vec<u8>,
    offset: usize,
}

// SAFETY: Http3Connection は内部的にスレッドセーフに使用される
unsafe impl Send for Http3Connection {}
unsafe impl Sync for Http3Connection {}

impl Http3Connection {
    /// クライアント接続を作成
    pub fn client_new(settings: &nghttp3_settings) -> Result<Self> {
        let mut events = Box::new(VecDeque::new());
        let events_ptr = &mut *events as *mut VecDeque<Http3Event>;

        let user_data = Box::new(Http3UserData {
            events: events_ptr,
            wt_send_queues: HashMap::new(),
            request_body_queues: HashMap::new(),
        });

        let mut conn: *mut nghttp3_conn = ptr::null_mut();

        let callbacks = create_callbacks();

        let rv = unsafe {
            nghttp3_conn_client_new_versioned(
                &mut conn,
                NGHTTP3_CALLBACKS_VERSION as c_int,
                &callbacks,
                NGHTTP3_SETTINGS_VERSION as c_int,
                settings,
                nghttp3_mem_default(),
                &*user_data as *const _ as *mut c_void,
            )
        };

        check_nghttp3(rv)?;

        Ok(Self {
            inner: conn,
            events,
            user_data,
        })
    }

    /// サーバー接続を作成
    pub fn server_new(settings: &nghttp3_settings) -> Result<Self> {
        let mut events = Box::new(VecDeque::new());
        let events_ptr = &mut *events as *mut VecDeque<Http3Event>;

        let user_data = Box::new(Http3UserData {
            events: events_ptr,
            wt_send_queues: HashMap::new(),
            request_body_queues: HashMap::new(),
        });

        let mut conn: *mut nghttp3_conn = ptr::null_mut();

        let callbacks = create_callbacks();

        let rv = unsafe {
            nghttp3_conn_server_new_versioned(
                &mut conn,
                NGHTTP3_CALLBACKS_VERSION as c_int,
                &callbacks,
                NGHTTP3_SETTINGS_VERSION as c_int,
                settings,
                nghttp3_mem_default(),
                &*user_data as *const _ as *mut c_void,
            )
        };

        check_nghttp3(rv)?;

        Ok(Self {
            inner: conn,
            events,
            user_data,
        })
    }

    /// コントロールストリームをバインド
    pub fn bind_control_stream(&mut self, stream_id: StreamId) -> Result<()> {
        let rv = unsafe { nghttp3_conn_bind_control_stream(self.inner, stream_id) };
        check_nghttp3(rv)
    }

    /// QPACK ストリームをバインド
    pub fn bind_qpack_streams(
        &mut self,
        qenc_stream_id: StreamId,
        qdec_stream_id: StreamId,
    ) -> Result<()> {
        let rv =
            unsafe { nghttp3_conn_bind_qpack_streams(self.inner, qenc_stream_id, qdec_stream_id) };
        check_nghttp3(rv)
    }

    /// ストリームからデータを読み込む
    ///
    /// # Arguments
    ///
    /// * `stream_id` - ストリーム ID
    /// * `data` - 読み込むデータ
    /// * `fin` - ストリームの終端かどうか
    /// * `ts` - 現在のタイムスタンプ (ナノ秒)
    pub fn read_stream(
        &mut self,
        stream_id: StreamId,
        data: &[u8],
        fin: bool,
        ts: u64,
    ) -> Result<usize> {
        // events ポインタを更新
        self.user_data.events = &mut *self.events as *mut VecDeque<Http3Event>;

        let rv = unsafe {
            nghttp3_conn_read_stream2(
                self.inner,
                stream_id,
                data.as_ptr(),
                data.len(),
                if fin { 1 } else { 0 },
                ts,
            )
        };

        if rv < 0 {
            return Err(Error::from_nghttp3(rv as c_int));
        }

        Ok(rv as usize)
    }

    /// ストリームに書き込むデータを取得
    pub fn write_stream(&mut self, buf: &mut [nghttp3_vec]) -> Result<(StreamId, bool, usize)> {
        let mut stream_id: i64 = 0;
        let mut fin: c_int = 0;

        let rv = unsafe {
            nghttp3_conn_writev_stream(
                self.inner,
                &mut stream_id,
                &mut fin,
                buf.as_mut_ptr(),
                buf.len(),
            )
        };

        if rv < 0 {
            return Err(Error::from_nghttp3(rv as c_int));
        }

        Ok((stream_id, fin != 0, rv as usize))
    }

    /// 書き込みオフセットを追加
    pub fn add_write_offset(&mut self, stream_id: StreamId, n: usize) -> Result<()> {
        let rv = unsafe { nghttp3_conn_add_write_offset(self.inner, stream_id, n) };
        check_nghttp3(rv)
    }

    /// ACK オフセットを追加
    pub fn add_ack_offset(&mut self, stream_id: StreamId, n: u64) -> Result<()> {
        let rv = unsafe { nghttp3_conn_add_ack_offset(self.inner, stream_id, n) };
        check_nghttp3(rv)
    }

    /// リクエストを送信
    pub fn submit_request(&mut self, stream_id: StreamId, headers: &[Header]) -> Result<()> {
        let nvs: Vec<nghttp3_nv> = headers
            .iter()
            .map(|h| nghttp3_nv {
                name: h.name.as_ptr() as *mut _,
                value: h.value.as_ptr() as *mut _,
                namelen: h.name.len(),
                valuelen: h.value.len(),
                flags: NGHTTP3_NV_FLAG_NONE as u8,
            })
            .collect();

        let rv = unsafe {
            nghttp3_conn_submit_request(
                self.inner,
                stream_id,
                nvs.as_ptr(),
                nvs.len(),
                ptr::null(),
                ptr::null_mut(),
            )
        };

        check_nghttp3(rv)
    }

    /// リクエストを送信（ボディあり）
    ///
    /// ボディは一括で送信される。大きなボディやストリーミング送信が必要な場合は
    /// `submit_request()` の後に `send_request_body()` を使用する。
    pub fn submit_request_with_body(
        &mut self,
        stream_id: StreamId,
        headers: &[Header],
        body: Vec<u8>,
    ) -> Result<()> {
        let nvs: Vec<nghttp3_nv> = headers
            .iter()
            .map(|h| nghttp3_nv {
                name: h.name.as_ptr() as *mut _,
                value: h.value.as_ptr() as *mut _,
                namelen: h.name.len(),
                valuelen: h.value.len(),
                flags: NGHTTP3_NV_FLAG_NONE as u8,
            })
            .collect();

        // ボディデータを保持する構造体を作成
        let body_data = Box::new(StreamBodyData {
            data: body,
            offset: 0,
        });
        let body_data_ptr = Box::into_raw(body_data);

        // ボディありのリクエスト用の data_reader を設定
        let dr = nghttp3_data_reader {
            read_data: Some(body_read_data_callback),
        };

        let rv = unsafe {
            nghttp3_conn_submit_request(
                self.inner,
                stream_id,
                nvs.as_ptr(),
                nvs.len(),
                &dr,
                body_data_ptr as *mut c_void,
            )
        };

        if rv < 0 {
            // エラー時はメモリを解放
            unsafe { drop(Box::from_raw(body_data_ptr)) };
            return Err(Error::Nghttp3("submit_request failed".to_string(), rv));
        }

        Ok(())
    }

    /// ストリーミング送信用リクエストを開始
    ///
    /// `send_request_body()` と組み合わせて使用する。
    /// ヘッダーのみを送信し、ボディは後から `send_request_body()` で送信する。
    ///
    /// # Arguments
    ///
    /// * `stream_id` - ストリーム ID
    /// * `headers` - リクエストヘッダー
    pub fn submit_request_streaming(
        &mut self,
        stream_id: StreamId,
        headers: &[Header],
    ) -> Result<()> {
        let nvs: Vec<nghttp3_nv> = headers
            .iter()
            .map(|h| nghttp3_nv {
                name: h.name.as_ptr() as *mut _,
                value: h.value.as_ptr() as *mut _,
                namelen: h.name.len(),
                valuelen: h.value.len(),
                flags: NGHTTP3_NV_FLAG_NONE as u8,
            })
            .collect();

        // ストリーミング用の data_reader を設定
        let dr = nghttp3_data_reader {
            read_data: Some(request_body_queue_read_data_callback),
        };

        let rv = unsafe {
            nghttp3_conn_submit_request(
                self.inner,
                stream_id,
                nvs.as_ptr(),
                nvs.len(),
                &dr,
                ptr::null_mut(),
            )
        };

        check_nghttp3(rv)
    }

    /// リクエストボディを追加送信
    ///
    /// `submit_request_streaming()` で開始したリクエストに追加のボディデータを送信する。
    /// データは送信キューに追加され、次の `write_stream()` 呼び出し時に送信される。
    ///
    /// # Arguments
    ///
    /// * `stream_id` - ストリーム ID
    /// * `data` - 送信するデータ
    /// * `fin` - ストリームを終了するかどうか
    pub fn send_request_body(&mut self, stream_id: StreamId, data: &[u8], fin: bool) -> Result<()> {
        let queue = self
            .user_data
            .request_body_queues
            .entry(stream_id)
            .or_insert(RequestBodyQueue {
                data: Vec::new(),
                offset: 0,
                fin: false,
            });

        queue.data.extend_from_slice(data);
        if fin {
            queue.fin = true;
        }

        // nghttp3 にデータが利用可能であることを通知
        self.resume_stream(stream_id)?;

        Ok(())
    }

    /// レスポンスを送信（ボディなし）
    pub fn submit_response(&mut self, stream_id: StreamId, headers: &[Header]) -> Result<()> {
        let nvs: Vec<nghttp3_nv> = headers
            .iter()
            .map(|h| nghttp3_nv {
                name: h.name.as_ptr() as *mut _,
                value: h.value.as_ptr() as *mut _,
                namelen: h.name.len(),
                valuelen: h.value.len(),
                flags: NGHTTP3_NV_FLAG_NONE as u8,
            })
            .collect();

        // ボディなしのレスポンス用の data_reader を設定
        // EOF フラグを返すことで FIN を送信する
        let dr = nghttp3_data_reader {
            read_data: Some(empty_response_read_data_callback),
        };

        let rv = unsafe {
            nghttp3_conn_submit_response(self.inner, stream_id, nvs.as_ptr(), nvs.len(), &dr)
        };

        check_nghttp3(rv)
    }

    /// レスポンスを送信（ボディあり）
    pub fn submit_response_with_body(
        &mut self,
        stream_id: StreamId,
        headers: &[Header],
        body: Vec<u8>,
    ) -> Result<()> {
        let nvs: Vec<nghttp3_nv> = headers
            .iter()
            .map(|h| nghttp3_nv {
                name: h.name.as_ptr() as *mut _,
                value: h.value.as_ptr() as *mut _,
                namelen: h.name.len(),
                valuelen: h.value.len(),
                flags: NGHTTP3_NV_FLAG_NONE as u8,
            })
            .collect();

        // ボディデータを保持する構造体を作成
        let body_data = Box::new(StreamBodyData {
            data: body,
            offset: 0,
        });
        let body_data_ptr = Box::into_raw(body_data);

        // ストリームユーザーデータを設定
        let rv = unsafe {
            nghttp3_conn_set_stream_user_data(self.inner, stream_id, body_data_ptr as *mut c_void)
        };
        if rv < 0 {
            // エラー時はメモリを解放
            unsafe { drop(Box::from_raw(body_data_ptr)) };
            return Err(Error::Nghttp3(
                "set_stream_user_data failed".to_string(),
                rv,
            ));
        }

        // ボディありのレスポンス用の data_reader を設定
        let dr = nghttp3_data_reader {
            read_data: Some(body_read_data_callback),
        };

        let rv = unsafe {
            nghttp3_conn_submit_response(self.inner, stream_id, nvs.as_ptr(), nvs.len(), &dr)
        };

        if rv < 0 {
            // エラー時はストリームユーザーデータを解放
            unsafe {
                let _ = nghttp3_conn_set_stream_user_data(self.inner, stream_id, ptr::null_mut());
                drop(Box::from_raw(body_data_ptr));
            }
            return Err(Error::Nghttp3("submit_response failed".to_string(), rv));
        }

        Ok(())
    }

    /// トレーラーを送信
    pub fn submit_trailers(&mut self, stream_id: StreamId, headers: &[Header]) -> Result<()> {
        let nvs: Vec<nghttp3_nv> = headers
            .iter()
            .map(|h| nghttp3_nv {
                name: h.name.as_ptr() as *mut _,
                value: h.value.as_ptr() as *mut _,
                namelen: h.name.len(),
                valuelen: h.value.len(),
                flags: NGHTTP3_NV_FLAG_NONE as u8,
            })
            .collect();

        let rv =
            unsafe { nghttp3_conn_submit_trailers(self.inner, stream_id, nvs.as_ptr(), nvs.len()) };

        check_nghttp3(rv)
    }

    /// シャットダウン通知を送信
    pub fn submit_shutdown_notice(&mut self) -> Result<()> {
        let rv = unsafe { nghttp3_conn_submit_shutdown_notice(self.inner) };
        check_nghttp3(rv)
    }

    /// ストリームをブロック
    pub fn block_stream(&mut self, stream_id: StreamId) {
        unsafe { nghttp3_conn_block_stream(self.inner, stream_id) };
    }

    /// ストリームのブロックを解除
    pub fn unblock_stream(&mut self, stream_id: StreamId) -> Result<()> {
        let rv = unsafe { nghttp3_conn_unblock_stream(self.inner, stream_id) };
        check_nghttp3(rv)
    }

    /// ストリームが書き込み可能かどうか
    pub fn is_stream_writable(&self, stream_id: StreamId) -> bool {
        unsafe { nghttp3_conn_is_stream_writable(self.inner, stream_id) != 0 }
    }

    /// ストリームを再開
    pub fn resume_stream(&mut self, stream_id: StreamId) -> Result<()> {
        let rv = unsafe { nghttp3_conn_resume_stream(self.inner, stream_id) };
        check_nghttp3(rv)
    }

    /// ストリームを閉じる
    pub fn close_stream(&mut self, stream_id: StreamId, error_code: u64) -> Result<()> {
        let rv = unsafe { nghttp3_conn_close_stream(self.inner, stream_id, error_code) };
        check_nghttp3(rv)
    }

    /// ストリームの書き込みをシャットダウン
    ///
    /// 指定したストリームへの書き込みを禁止する。
    /// `block_stream` と同様に動作するが、`unblock_stream` で解除することはできない。
    pub fn shutdown_stream_write(&mut self, stream_id: StreamId) {
        unsafe { nghttp3_conn_shutdown_stream_write(self.inner, stream_id) };
    }

    /// クライアントの最大双方向ストリーム数を設定
    pub fn set_max_client_streams_bidi(&mut self, max_streams: u64) {
        unsafe { nghttp3_conn_set_max_client_streams_bidi(self.inner, max_streams) };
    }

    /// イベントを取得
    ///
    /// FIFO 順序で返す (push された順)。
    pub fn poll_event(&mut self) -> Option<Http3Event> {
        self.events.pop_front()
    }

    /// 内部ポインタを取得
    pub fn as_ptr(&self) -> *mut nghttp3_conn {
        self.inner
    }

    // ========================================
    // WebTransport 関連メソッド
    // ========================================

    /// WebTransport リクエストを送信
    ///
    /// クライアントが WebTransport セッション確立リクエストを送信する。
    /// ヘッダーには以下を含める必要がある:
    /// - :method = "CONNECT"
    /// - :scheme = "https"
    /// - :protocol = "webtransport"
    /// - :authority
    /// - :path
    pub fn submit_wt_request(&mut self, stream_id: StreamId, headers: &[Header]) -> Result<()> {
        let nvs: Vec<nghttp3_nv> = headers
            .iter()
            .map(|h| nghttp3_nv {
                name: h.name.as_ptr() as *mut _,
                value: h.value.as_ptr() as *mut _,
                namelen: h.name.len(),
                valuelen: h.value.len(),
                flags: NGHTTP3_NV_FLAG_NONE as u8,
            })
            .collect();

        let rv = unsafe {
            nghttp3_conn_submit_wt_request(
                self.inner,
                stream_id,
                nvs.as_ptr(),
                nvs.len(),
                ptr::null_mut(),
            )
        };

        check_nghttp3(rv)
    }

    /// WebTransport レスポンスを送信
    ///
    /// サーバーが WebTransport セッション確立レスポンスを送信する。
    /// ヘッダーには 2xx ステータスコードを含める必要がある。
    pub fn submit_wt_response(&mut self, stream_id: StreamId, headers: &[Header]) -> Result<()> {
        let nvs: Vec<nghttp3_nv> = headers
            .iter()
            .map(|h| nghttp3_nv {
                name: h.name.as_ptr() as *mut _,
                value: h.value.as_ptr() as *mut _,
                namelen: h.name.len(),
                valuelen: h.value.len(),
                flags: NGHTTP3_NV_FLAG_NONE as u8,
            })
            .collect();

        let rv = unsafe {
            nghttp3_conn_submit_wt_response(self.inner, stream_id, nvs.as_ptr(), nvs.len())
        };

        check_nghttp3(rv)
    }

    /// WebTransport セッションを確認 (サーバー側)
    ///
    /// `submit_wt_response` 後に呼び出す (end_headers コールバック外の場合)。
    pub fn server_confirm_wt_session(&mut self, session_id: SessionId, ts: u64) -> Result<()> {
        let rv = unsafe { nghttp3_conn_server_confirm_wt_session(self.inner, session_id, ts) };

        check_nghttp3(rv)
    }

    /// WebTransport データストリームを開く
    ///
    /// WebTransport セッション上でデータストリームを開く。
    /// 双方向と単方向の両方のストリームで使用可能。
    pub fn open_wt_data_stream(
        &mut self,
        session_id: SessionId,
        stream_id: StreamId,
    ) -> Result<()> {
        // データリーダーコールバックを設定
        let dr = nghttp3_data_reader {
            read_data: Some(wt_read_data_callback),
        };

        let rv = unsafe {
            nghttp3_conn_open_wt_data_stream(
                self.inner,
                session_id,
                stream_id,
                &dr,
                ptr::null_mut(),
            )
        };

        check_nghttp3(rv)
    }

    /// WebTransport ストリームにデータを送信
    ///
    /// データを送信キューに追加し、nghttp3 にストリームを再開させる。
    /// 実際のデータ送信は次の `write_stream()` 呼び出し時に行われる。
    ///
    /// # Arguments
    ///
    /// * `stream_id` - ストリーム ID
    /// * `data` - 送信するデータ
    /// * `fin` - ストリームを終了するかどうか
    pub fn send_wt_stream_data(
        &mut self,
        stream_id: StreamId,
        data: &[u8],
        fin: bool,
    ) -> Result<()> {
        let queue = self
            .user_data
            .wt_send_queues
            .entry(stream_id)
            .or_insert(WtSendQueue {
                data: Vec::new(),
                offset: 0,
                fin: false,
            });

        queue.data.extend_from_slice(data);
        if fin {
            queue.fin = true;
        }

        // nghttp3 にデータが利用可能であることを通知
        self.resume_stream(stream_id)?;

        Ok(())
    }

    /// WebTransport セッションを閉じる
    ///
    /// WebTransport セッションを閉じて、全ての関連するストリームをシャットダウンする。
    pub fn close_wt_session(
        &mut self,
        session_id: SessionId,
        error_code: u32,
        msg: Option<&[u8]>,
    ) -> Result<()> {
        let (msg_ptr, msg_len) = match msg {
            Some(m) => (m.as_ptr(), m.len()),
            None => (ptr::null(), 0),
        };

        let rv = unsafe {
            nghttp3_conn_close_wt_session(self.inner, session_id, error_code, msg_ptr, msg_len)
        };

        check_nghttp3(rv)
    }
}

/// WebTransport データリーダーコールバック
///
/// conn_user_data 内の wt_send_queues から送信データを取得する。
/// キューにデータがない場合は WOULDBLOCK を返す。
unsafe extern "C" fn wt_read_data_callback(
    _conn: *mut nghttp3_conn,
    stream_id: i64,
    vec: *mut nghttp3_vec,
    _veccnt: usize,
    pflags: *mut u32,
    conn_user_data: *mut c_void,
    _stream_user_data: *mut c_void,
) -> isize {
    if conn_user_data.is_null() || vec.is_null() {
        if !pflags.is_null() {
            unsafe { *pflags = NGHTTP3_DATA_FLAG_NONE };
        }
        return NGHTTP3_ERR_WOULDBLOCK as isize;
    }

    unsafe {
        let user_data = &mut *(conn_user_data as *mut Http3UserData);

        if let Some(queue) = user_data.wt_send_queues.get_mut(&stream_id) {
            let remaining = queue.data.len() - queue.offset;

            if remaining == 0 {
                if queue.fin {
                    *pflags = NGHTTP3_DATA_FLAG_EOF;
                    return 0;
                }
                return NGHTTP3_ERR_WOULDBLOCK as isize;
            }

            (*vec).base = queue.data.as_ptr().add(queue.offset) as *mut u8;
            (*vec).len = remaining;
            queue.offset += remaining;

            if queue.fin {
                *pflags = NGHTTP3_DATA_FLAG_EOF;
            } else {
                *pflags = NGHTTP3_DATA_FLAG_NONE;
            }

            return 1;
        }

        // キューがない場合は WOULDBLOCK
        *pflags = NGHTTP3_DATA_FLAG_NONE;
        NGHTTP3_ERR_WOULDBLOCK as isize
    }
}

/// 空のレスポンス用データリーダーコールバック
///
/// ボディなしのレスポンス（ヘッダーのみ）を送信する場合に使用する。
/// EOF フラグを設定して、ストリームを終了（FIN を送信）する。
unsafe extern "C" fn empty_response_read_data_callback(
    _conn: *mut nghttp3_conn,
    _stream_id: i64,
    _vec: *mut nghttp3_vec,
    _veccnt: usize,
    pflags: *mut u32,
    _conn_user_data: *mut c_void,
    _stream_user_data: *mut c_void,
) -> isize {
    // EOF フラグを設定してストリームを終了
    if !pflags.is_null() {
        unsafe { *pflags = NGHTTP3_DATA_FLAG_EOF };
    }
    // データなし（0 バイト）を返す
    0
}

/// ボディデータ用データリーダーコールバック
///
/// stream_user_data に設定された StreamBodyData からボディデータを読み取って返す。
/// リクエストボディとレスポンスボディの両方で共通に使用する。
unsafe extern "C" fn body_read_data_callback(
    _conn: *mut nghttp3_conn,
    _stream_id: i64,
    vec: *mut nghttp3_vec,
    veccnt: usize,
    pflags: *mut u32,
    _conn_user_data: *mut c_void,
    stream_user_data: *mut c_void,
) -> isize {
    if stream_user_data.is_null() || veccnt == 0 || vec.is_null() {
        // ユーザーデータがない場合は EOF
        if !pflags.is_null() {
            unsafe { *pflags = NGHTTP3_DATA_FLAG_EOF };
        }
        return 0;
    }

    unsafe {
        let body = &mut *(stream_user_data as *mut StreamBodyData);
        let remaining = body.data.len() - body.offset;

        if remaining == 0 {
            // 全データ送信済み、EOF を返す
            *pflags = NGHTTP3_DATA_FLAG_EOF;
            return 0;
        }

        // データを nghttp3_vec に設定
        (*vec).base = body.data.as_ptr().add(body.offset) as *mut u8;
        (*vec).len = remaining;

        // オフセットを更新（nghttp3 が add_write_offset を呼んだ後に実際に消費される）
        body.offset += remaining;

        // 全データ返したので EOF フラグを設定
        *pflags = NGHTTP3_DATA_FLAG_EOF;

        1 // 1 つの vec を使用
    }
}

/// リクエストボディキュー用データリーダーコールバック
///
/// conn_user_data 内の request_body_queues から送信データを取得する。
/// キューにデータがない場合は WOULDBLOCK を返す。
unsafe extern "C" fn request_body_queue_read_data_callback(
    _conn: *mut nghttp3_conn,
    stream_id: i64,
    vec: *mut nghttp3_vec,
    _veccnt: usize,
    pflags: *mut u32,
    conn_user_data: *mut c_void,
    _stream_user_data: *mut c_void,
) -> isize {
    if conn_user_data.is_null() || vec.is_null() {
        if !pflags.is_null() {
            unsafe { *pflags = NGHTTP3_DATA_FLAG_NONE };
        }
        return NGHTTP3_ERR_WOULDBLOCK as isize;
    }

    unsafe {
        let user_data = &mut *(conn_user_data as *mut Http3UserData);

        if let Some(queue) = user_data.request_body_queues.get_mut(&stream_id) {
            let remaining = queue.data.len() - queue.offset;

            if remaining == 0 {
                if queue.fin {
                    *pflags = NGHTTP3_DATA_FLAG_EOF;
                    return 0;
                }
                return NGHTTP3_ERR_WOULDBLOCK as isize;
            }

            (*vec).base = queue.data.as_ptr().add(queue.offset) as *mut u8;
            (*vec).len = remaining;
            queue.offset += remaining;

            if queue.fin {
                *pflags = NGHTTP3_DATA_FLAG_EOF;
            } else {
                *pflags = NGHTTP3_DATA_FLAG_NONE;
            }

            return 1;
        }

        // キューがない場合は WOULDBLOCK
        *pflags = NGHTTP3_DATA_FLAG_NONE;
        NGHTTP3_ERR_WOULDBLOCK as isize
    }
}

impl Drop for Http3Connection {
    fn drop(&mut self) {
        if !self.inner.is_null() {
            unsafe { nghttp3_conn_del(self.inner) };
        }
    }
}

/// コールバックを作成
fn create_callbacks() -> nghttp3_callbacks {
    nghttp3_callbacks {
        acked_stream_data: Some(on_acked_stream_data),
        stream_close: Some(on_stream_close),
        recv_data: Some(on_recv_data),
        deferred_consume: None,
        begin_headers: Some(on_begin_headers),
        recv_header: Some(on_recv_header),
        end_headers: Some(on_end_headers),
        begin_trailers: Some(on_begin_trailers),
        recv_trailer: Some(on_recv_trailer),
        end_trailers: Some(on_end_trailers),
        stop_sending: None,
        end_stream: Some(on_end_stream),
        reset_stream: Some(on_reset_stream),
        shutdown: None,
        recv_settings: None,
        recv_origin: None,
        end_origin: None,
        rand: Some(on_rand),
        recv_settings2: None,
        recv_wt_data: Some(on_recv_wt_data),
    }
}

unsafe extern "C" fn on_acked_stream_data(
    _conn: *mut nghttp3_conn,
    _stream_id: i64,
    _datalen: u64,
    _conn_user_data: *mut c_void,
    _stream_user_data: *mut c_void,
) -> c_int {
    0
}

unsafe extern "C" fn on_stream_close(
    _conn: *mut nghttp3_conn,
    stream_id: i64,
    app_error_code: u64,
    conn_user_data: *mut c_void,
    stream_user_data: *mut c_void,
) -> c_int {
    // stream_user_data が StreamBodyData の場合、メモリを解放
    if !stream_user_data.is_null() {
        unsafe {
            drop(Box::from_raw(stream_user_data as *mut StreamBodyData));
        }
    }

    if conn_user_data.is_null() {
        return 0;
    }

    unsafe {
        let user_data = &mut *(conn_user_data as *mut Http3UserData);

        // 送信キューをクリーンアップ
        user_data.wt_send_queues.remove(&stream_id);
        user_data.request_body_queues.remove(&stream_id);

        if !user_data.events.is_null() {
            (*user_data.events).push_back(Http3Event::StreamClose {
                stream_id,
                error_code: app_error_code,
            });
        }
    }

    0
}

unsafe extern "C" fn on_recv_data(
    _conn: *mut nghttp3_conn,
    stream_id: i64,
    data: *const u8,
    datalen: usize,
    conn_user_data: *mut c_void,
    _stream_user_data: *mut c_void,
) -> c_int {
    if conn_user_data.is_null() {
        return 0;
    }

    unsafe {
        let user_data = &*(conn_user_data as *const Http3UserData);
        if !user_data.events.is_null() {
            let data_slice = std::slice::from_raw_parts(data, datalen);
            (*user_data.events).push_back(Http3Event::Data {
                stream_id,
                data: data_slice.to_vec(),
            });
        }
    }

    0
}

unsafe extern "C" fn on_begin_headers(
    _conn: *mut nghttp3_conn,
    stream_id: i64,
    conn_user_data: *mut c_void,
    _stream_user_data: *mut c_void,
) -> c_int {
    if conn_user_data.is_null() {
        return 0;
    }

    unsafe {
        let user_data = &*(conn_user_data as *const Http3UserData);
        if !user_data.events.is_null() {
            (*user_data.events).push_back(Http3Event::HeadersBegin { stream_id });
        }
    }

    0
}

unsafe extern "C" fn on_recv_header(
    _conn: *mut nghttp3_conn,
    stream_id: i64,
    _token: i32,
    name: *mut nghttp3_rcbuf,
    value: *mut nghttp3_rcbuf,
    _flags: u8,
    conn_user_data: *mut c_void,
    _stream_user_data: *mut c_void,
) -> c_int {
    if conn_user_data.is_null() {
        return 0;
    }

    unsafe {
        let user_data = &*(conn_user_data as *const Http3UserData);
        if !user_data.events.is_null() {
            let name_vec = nghttp3_rcbuf_get_buf(name);
            let value_vec = nghttp3_rcbuf_get_buf(value);

            let name_slice = std::slice::from_raw_parts(name_vec.base, name_vec.len);
            let value_slice = std::slice::from_raw_parts(value_vec.base, value_vec.len);

            (*user_data.events).push_back(Http3Event::Header {
                stream_id,
                header: Header::new(name_slice.to_vec(), value_slice.to_vec()),
            });
        }
    }

    0
}

unsafe extern "C" fn on_end_headers(
    _conn: *mut nghttp3_conn,
    stream_id: i64,
    fin: c_int,
    conn_user_data: *mut c_void,
    _stream_user_data: *mut c_void,
) -> c_int {
    if conn_user_data.is_null() {
        return 0;
    }

    unsafe {
        let user_data = &*(conn_user_data as *const Http3UserData);
        if !user_data.events.is_null() {
            (*user_data.events).push_back(Http3Event::HeadersEnd {
                stream_id,
                fin: fin != 0,
            });
        }
    }

    0
}

unsafe extern "C" fn on_begin_trailers(
    _conn: *mut nghttp3_conn,
    stream_id: i64,
    conn_user_data: *mut c_void,
    _stream_user_data: *mut c_void,
) -> c_int {
    if conn_user_data.is_null() {
        return 0;
    }

    unsafe {
        let user_data = &*(conn_user_data as *const Http3UserData);
        if !user_data.events.is_null() {
            (*user_data.events).push_back(Http3Event::TrailersBegin { stream_id });
        }
    }

    0
}

unsafe extern "C" fn on_recv_trailer(
    _conn: *mut nghttp3_conn,
    stream_id: i64,
    _token: i32,
    name: *mut nghttp3_rcbuf,
    value: *mut nghttp3_rcbuf,
    _flags: u8,
    conn_user_data: *mut c_void,
    _stream_user_data: *mut c_void,
) -> c_int {
    if conn_user_data.is_null() {
        return 0;
    }

    unsafe {
        let user_data = &*(conn_user_data as *const Http3UserData);
        if !user_data.events.is_null() {
            let name_vec = nghttp3_rcbuf_get_buf(name);
            let value_vec = nghttp3_rcbuf_get_buf(value);

            let name_slice = std::slice::from_raw_parts(name_vec.base, name_vec.len);
            let value_slice = std::slice::from_raw_parts(value_vec.base, value_vec.len);

            (*user_data.events).push_back(Http3Event::Trailer {
                stream_id,
                header: Header::new(name_slice.to_vec(), value_slice.to_vec()),
            });
        }
    }

    0
}

unsafe extern "C" fn on_end_trailers(
    _conn: *mut nghttp3_conn,
    stream_id: i64,
    _fin: c_int,
    conn_user_data: *mut c_void,
    _stream_user_data: *mut c_void,
) -> c_int {
    if conn_user_data.is_null() {
        return 0;
    }

    unsafe {
        let user_data = &*(conn_user_data as *const Http3UserData);
        if !user_data.events.is_null() {
            (*user_data.events).push_back(Http3Event::TrailersEnd { stream_id });
        }
    }

    0
}

unsafe extern "C" fn on_end_stream(
    _conn: *mut nghttp3_conn,
    stream_id: i64,
    conn_user_data: *mut c_void,
    _stream_user_data: *mut c_void,
) -> c_int {
    if conn_user_data.is_null() {
        return 0;
    }

    unsafe {
        let user_data = &*(conn_user_data as *const Http3UserData);
        if !user_data.events.is_null() {
            (*user_data.events).push_back(Http3Event::StreamEnd { stream_id });
        }
    }

    0
}

unsafe extern "C" fn on_reset_stream(
    _conn: *mut nghttp3_conn,
    stream_id: i64,
    app_error_code: u64,
    conn_user_data: *mut c_void,
    _stream_user_data: *mut c_void,
) -> c_int {
    if conn_user_data.is_null() {
        return 0;
    }

    unsafe {
        let user_data = &*(conn_user_data as *const Http3UserData);
        if !user_data.events.is_null() {
            (*user_data.events).push_back(Http3Event::Reset {
                stream_id,
                error_code: app_error_code,
            });
        }
    }

    0
}

unsafe extern "C" fn on_rand(dest: *mut u8, destlen: usize) {
    if dest.is_null() || destlen == 0 {
        return;
    }

    unsafe {
        let slice = std::slice::from_raw_parts_mut(dest, destlen);
        let _ = aws_lc_rs::rand::fill(slice);
    }
}

unsafe extern "C" fn on_recv_wt_data(
    _conn: *mut nghttp3_conn,
    session_id: i64,
    stream_id: i64,
    data: *const u8,
    datalen: usize,
    conn_user_data: *mut c_void,
    _stream_user_data: *mut c_void,
) -> c_int {
    if conn_user_data.is_null() {
        return 0;
    }

    unsafe {
        let user_data = &*(conn_user_data as *const Http3UserData);
        if !user_data.events.is_null() {
            let data_slice = std::slice::from_raw_parts(data, datalen);
            (*user_data.events).push_back(Http3Event::WebTransportData {
                session_id,
                stream_id,
                data: data_slice.to_vec(),
            });
        }
    }

    0
}
