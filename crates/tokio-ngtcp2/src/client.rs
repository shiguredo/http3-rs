//! HTTP/3 クライアント実装

use std::net::SocketAddr;
use std::time::Duration;

use nghttp3_sys::nghttp3_settings;
use ngtcp2_sys::ngtcp2_transport_params;
use shiguredo_ngtcp2::{
    Connection, ConnectionId, Error, Header, Http3Connection, Http3Event, Http3Settings,
    PacketInfo, Result, StreamId, TlsContext, TransportParams,
};

use crate::{Socket, timestamp};

/// HTTP/3 クライアント
pub struct Client {
    socket: Socket,
    local_addr: SocketAddr,
    remote_addr: SocketAddr,
    // TLS コンテキスト (SSL_CTX を保持するため)
    _tls_ctx: TlsContext,
    // QUIC 接続
    conn: Connection,
    // HTTP/3 接続
    h3_conn: Http3Connection,
    // 受信バッファ
    recv_buf: Vec<u8>,
    // 送信バッファ
    send_buf: Vec<u8>,
    // コントロールストリームをバインド済みか
    control_streams_bound: bool,
}

// SAFETY: Client の全フィールドは Send/Sync を実装している
// (Connection, Http3Connection は unsafe impl Send/Sync 済み)
unsafe impl Send for Client {}
unsafe impl Sync for Client {}

impl Client {
    /// 新しいクライアントを作成
    ///
    /// サーバー証明書のチェーン検証とホスト名検証を行う (RFC 9114 Section 3.1)。
    /// 検証に使うトラストストアはデフォルト CA パスと `SSL_CERT_FILE` /
    /// `SSL_CERT_DIR` 環境変数に依存する。macOS ではシステムの CA が
    /// 自動的に読み込まれない環境があるため、`connect_with_ca` で CA を
    /// 明示指定するか、`SSL_CERT_FILE` を設定すること。
    ///
    /// # Arguments
    ///
    /// * `remote_addr` - 接続先アドレス
    /// * `server_name` - サーバー名 (SNI 兼ホスト名検証、DNS 名限定)
    /// * `transport_params` - QUIC トランスポートパラメータ (None でデフォルト)
    /// * `h3_settings` - HTTP/3 設定 (None でデフォルト)
    pub async fn connect(
        remote_addr: SocketAddr,
        server_name: &str,
        transport_params: Option<ngtcp2_transport_params>,
        h3_settings: Option<nghttp3_settings>,
    ) -> Result<Self> {
        Self::connect_with_options(
            remote_addr,
            server_name,
            transport_params,
            h3_settings,
            true,
        )
        .await
    }

    /// 新しいクライアントを作成 (証明書検証なし)
    ///
    /// テスト用の自己署名証明書で使用する。
    ///
    /// # Arguments
    ///
    /// * `remote_addr` - 接続先アドレス
    /// * `server_name` - サーバー名 (SNI 兼ホスト名検証、DNS 名限定)
    /// * `transport_params` - QUIC トランスポートパラメータ (None でデフォルト)
    /// * `h3_settings` - HTTP/3 設定 (None でデフォルト)
    pub async fn connect_insecure(
        remote_addr: SocketAddr,
        server_name: &str,
        transport_params: Option<ngtcp2_transport_params>,
        h3_settings: Option<nghttp3_settings>,
    ) -> Result<Self> {
        Self::connect_with_options(
            remote_addr,
            server_name,
            transport_params,
            h3_settings,
            false,
        )
        .await
    }

    /// 新しいクライアントを作成 (証明書検証なし、デフォルト設定)
    ///
    /// テスト用のシンプルな API。デフォルトのトランスポートパラメータと HTTP/3 設定を使用する。
    /// tokio::spawn 内で使用可能 (Send + 'static を満たす)。
    ///
    /// # Arguments
    ///
    /// * `remote_addr` - 接続先アドレス
    /// * `server_name` - サーバー名 (SNI 兼ホスト名検証、DNS 名限定)
    pub async fn connect_insecure_default(
        remote_addr: SocketAddr,
        server_name: &str,
    ) -> Result<Self> {
        Self::connect_internal(remote_addr, server_name, false).await
    }

    /// 内部接続メソッド (デフォルト設定を使用)
    ///
    /// `connect_with_options_internal` に委譲すると `Option<nghttp3_settings>`
    /// 引数 (nghttp3_vec の生ポインタを含み Send でない) が future に保持され、
    /// `tokio::spawn` 内で呼べなくなるため、直接実装する。
    async fn connect_internal(
        remote_addr: SocketAddr,
        server_name: &str,
        verify_peer: bool,
    ) -> Result<Self> {
        // ローカルアドレスにバインド
        let local_addr: SocketAddr = if remote_addr.is_ipv4() {
            "0.0.0.0:0".parse().expect("literal IPv4 bind addr")
        } else {
            "[::]:0".parse().expect("literal IPv6 bind addr")
        };

        let socket = Socket::bind(local_addr)
            .await
            .map_err(|e| Error::Internal(format!("failed to bind socket: {}", e)))?;

        let local_addr = socket.local_addr();

        // デフォルトのトランスポートパラメータ
        let params = TransportParams::new().into_raw();

        // デフォルトの HTTP/3 設定
        let h3_settings = Http3Settings::new().into_raw();

        // TLS コンテキストとセッションを作成
        let tls_ctx = TlsContext::new_client_with_options(&[b"h3"], verify_peer)?;
        let tls_session = tls_ctx.create_session()?;

        // コネクション ID を生成
        let dcid = ConnectionId::random(16)
            .ok_or(Error::Internal("failed to generate dcid".to_string()))?;
        let scid = ConnectionId::random(16)
            .ok_or(Error::Internal("failed to generate scid".to_string()))?;

        // タイムスタンプ
        let ts = timestamp();

        // QUIC 接続を作成
        let conn = Connection::client_new(
            &dcid,
            &scid,
            local_addr,
            remote_addr,
            server_name,
            tls_session,
            &params,
            ts,
        )?;

        // HTTP/3 接続を作成
        let h3_conn = Http3Connection::client_new(&h3_settings)?;

        // ACK 済みストリームデータオフセットを nghttp3 に通知するため、
        // ngtcp2 コールバックから nghttp3_conn へのポインタを設定する
        // SAFETY: conn と h3_conn は同じ構造体の中で共に保持され、
        // 破棄順はフィールド宣言順 (h3_conn が先にドロップされる) のため、
        // コールバック実行中にポインタが無効になることはない
        let mut conn = conn;
        unsafe { conn.set_h3_conn_ptr(h3_conn.as_mut_ptr() as *mut std::ffi::c_void) };

        Ok(Self {
            socket,
            local_addr,
            remote_addr,
            _tls_ctx: tls_ctx,
            conn,
            h3_conn,
            recv_buf: vec![0u8; 65535],
            send_buf: vec![0u8; 1350],
            control_streams_bound: false,
        })
    }

    /// 新しいクライアントを作成 (オプション付き)
    ///
    /// # Arguments
    ///
    /// * `remote_addr` - 接続先アドレス
    /// * `server_name` - サーバー名 (SNI 兼ホスト名検証、DNS 名限定)
    /// * `transport_params` - QUIC トランスポートパラメータ (None でデフォルト)
    /// * `h3_settings` - HTTP/3 設定 (None でデフォルト)
    /// * `verify_peer` - サーバー証明書を検証するかどうか
    async fn connect_with_options(
        remote_addr: SocketAddr,
        server_name: &str,
        transport_params: Option<ngtcp2_transport_params>,
        h3_settings: Option<nghttp3_settings>,
        verify_peer: bool,
    ) -> Result<Self> {
        Self::connect_with_options_internal(
            remote_addr,
            server_name,
            None,
            transport_params,
            h3_settings,
            verify_peer,
        )
        .await
    }

    /// 新しいクライアントを作成 (カスタム CA 証明書付き)
    ///
    /// サーバー証明書の検証に使用する CA 証明書 (PEM 形式) を指定する。
    /// ロードした CA はシステムのトラストストアに追加される (置換はしない)。
    /// PEM バンドル (連結された複数証明書) を渡した場合は先頭の 1 枚のみが
    /// ロードされる。
    ///
    /// # Arguments
    ///
    /// * `remote_addr` - 接続先アドレス
    /// * `server_name` - サーバー名 (SNI 兼ホスト名検証、DNS 名限定)
    /// * `ca_cert_pem` - CA 証明書の PEM 文字列
    /// * `transport_params` - QUIC トランスポートパラメータ (None でデフォルト)
    /// * `h3_settings` - HTTP/3 設定 (None でデフォルト)
    pub async fn connect_with_ca(
        remote_addr: SocketAddr,
        server_name: &str,
        ca_cert_pem: &str,
        transport_params: Option<ngtcp2_transport_params>,
        h3_settings: Option<nghttp3_settings>,
    ) -> Result<Self> {
        Self::connect_with_options_internal(
            remote_addr,
            server_name,
            Some(ca_cert_pem),
            transport_params,
            h3_settings,
            true,
        )
        .await
    }

    /// 内部接続メソッド (CA 証明書指定付き)
    async fn connect_with_options_internal(
        remote_addr: SocketAddr,
        server_name: &str,
        ca_cert_pem: Option<&str>,
        transport_params: Option<ngtcp2_transport_params>,
        h3_settings: Option<nghttp3_settings>,
        verify_peer: bool,
    ) -> Result<Self> {
        // ローカルアドレスにバインド
        let local_addr: SocketAddr = if remote_addr.is_ipv4() {
            "0.0.0.0:0".parse().expect("literal IPv4 bind addr")
        } else {
            "[::]:0".parse().expect("literal IPv6 bind addr")
        };

        let socket = Socket::bind(local_addr)
            .await
            .map_err(|e| Error::Internal(format!("failed to bind socket: {}", e)))?;

        let local_addr = socket.local_addr();

        // トランスポートパラメータ
        let params = transport_params.unwrap_or_else(|| TransportParams::new().into_raw());

        // HTTP/3 設定
        let h3_settings = h3_settings.unwrap_or_else(|| Http3Settings::new().into_raw());

        // TLS コンテキストとセッションを作成
        let mut tls_ctx = TlsContext::new_client_with_options(&[b"h3"], verify_peer)?;
        if let Some(ca_cert_pem) = ca_cert_pem {
            tls_ctx.add_ca_cert_pem(ca_cert_pem)?;
        }
        let tls_session = tls_ctx.create_session()?;

        // コネクション ID を生成
        let dcid = ConnectionId::random(16)
            .ok_or(Error::Internal("failed to generate dcid".to_string()))?;
        let scid = ConnectionId::random(16)
            .ok_or(Error::Internal("failed to generate scid".to_string()))?;

        // タイムスタンプ
        let ts = timestamp();

        // QUIC 接続を作成
        let conn = Connection::client_new(
            &dcid,
            &scid,
            local_addr,
            remote_addr,
            server_name,
            tls_session,
            &params,
            ts,
        )?;

        // HTTP/3 接続を作成
        let h3_conn = Http3Connection::client_new(&h3_settings)?;

        // ACK 済みストリームデータオフセットを nghttp3 に通知するため、
        // ngtcp2 コールバックから nghttp3_conn へのポインタを設定する
        // SAFETY: conn と h3_conn は同じ構造体の中で共に保持され、
        // 破棄順はフィールド宣言順 (h3_conn が先にドロップされる) のため、
        // コールバック実行中にポインタが無効になることはない
        let mut conn = conn;
        unsafe { conn.set_h3_conn_ptr(h3_conn.as_mut_ptr() as *mut std::ffi::c_void) };

        Ok(Self {
            socket,
            local_addr,
            remote_addr,
            _tls_ctx: tls_ctx,
            conn,
            h3_conn,
            recv_buf: vec![0u8; 65535],
            send_buf: vec![0u8; 1350],
            control_streams_bound: false,
        })
    }

    /// ローカルアドレスを取得
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// リモートアドレスを取得
    pub fn remote_addr(&self) -> SocketAddr {
        self.remote_addr
    }

    /// HTTP/3 リクエストを送信
    pub fn send_request(&mut self, headers: &[Header]) -> Result<StreamId> {
        // ハンドシェイク完了を確認
        if !self.conn.is_handshake_completed() {
            return Err(Error::Internal("handshake not completed".to_string()));
        }

        // コントロールストリームをバインド (初回のみ)
        if !self.control_streams_bound {
            self.bind_control_streams()?;
        }

        // QUIC ストリームを開く
        let stream_id = self.conn.open_bidi_stream()?;

        // HTTP/3 リクエストを送信
        self.h3_conn.submit_request(stream_id, headers)?;

        Ok(stream_id)
    }

    /// HTTP/3 リクエストをボディ付きで送信
    ///
    /// ボディは一括で送信される。大きなボディやストリーミング送信が必要な場合は
    /// `send_request_streaming()` の後に `send_body()` を使用する。
    ///
    /// # Arguments
    ///
    /// * `headers` - リクエストヘッダー
    /// * `body` - リクエストボディ
    pub fn send_request_with_body(
        &mut self,
        headers: &[Header],
        body: Vec<u8>,
    ) -> Result<StreamId> {
        // ハンドシェイク完了を確認
        if !self.conn.is_handshake_completed() {
            return Err(Error::Internal("handshake not completed".to_string()));
        }

        // コントロールストリームをバインド (初回のみ)
        if !self.control_streams_bound {
            self.bind_control_streams()?;
        }

        // QUIC ストリームを開く
        let stream_id = self.conn.open_bidi_stream()?;

        // HTTP/3 リクエストをボディ付きで送信
        self.h3_conn
            .submit_request_with_body(stream_id, headers, body)?;

        Ok(stream_id)
    }

    /// ストリーミング送信用 HTTP/3 リクエストを開始
    ///
    /// `send_body()` と組み合わせて使用する。
    /// ヘッダーのみを送信し、ボディは後から `send_body()` で送信する。
    ///
    /// # Arguments
    ///
    /// * `headers` - リクエストヘッダー
    pub fn send_request_streaming(&mut self, headers: &[Header]) -> Result<StreamId> {
        // ハンドシェイク完了を確認
        if !self.conn.is_handshake_completed() {
            return Err(Error::Internal("handshake not completed".to_string()));
        }

        // コントロールストリームをバインド (初回のみ)
        if !self.control_streams_bound {
            self.bind_control_streams()?;
        }

        // QUIC ストリームを開く
        let stream_id = self.conn.open_bidi_stream()?;

        // ストリーミング用リクエストを開始
        self.h3_conn.submit_request_streaming(stream_id, headers)?;

        Ok(stream_id)
    }

    /// リクエストボディを追加送信
    ///
    /// `send_request_streaming()` で開始したリクエストに追加のボディデータを送信する。
    ///
    /// # Arguments
    ///
    /// * `stream_id` - ストリーム ID
    /// * `data` - 送信するデータ
    /// * `fin` - ストリームを終了するかどうか
    pub async fn send_body(&mut self, stream_id: StreamId, data: &[u8], fin: bool) -> Result<()> {
        self.h3_conn.send_request_body(stream_id, data, fin)?;
        self.flush().await?;
        Ok(())
    }

    /// コントロールストリームをバインド
    fn bind_control_streams(&mut self) -> Result<()> {
        // コントロールストリーム
        let ctrl_stream_id = self.conn.open_uni_stream()?;
        self.h3_conn.bind_control_stream(ctrl_stream_id)?;

        // QPACK エンコーダストリーム
        let qpack_enc_stream_id = self.conn.open_uni_stream()?;

        // QPACK デコーダストリーム
        let qpack_dec_stream_id = self.conn.open_uni_stream()?;

        self.h3_conn
            .bind_qpack_streams(qpack_enc_stream_id, qpack_dec_stream_id)?;

        self.control_streams_bound = true;
        Ok(())
    }

    /// イベントをポーリング
    pub fn poll(&mut self) -> Option<Http3Event> {
        self.h3_conn.poll_event()
    }

    /// イベントループを実行 (ハンドシェイクのみ)
    pub async fn handshake(&mut self) -> Result<()> {
        let timeout = Duration::from_secs(30);
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            // 送信データを処理
            self.flush().await?;

            // ハンドシェイク完了を確認
            if self.conn.is_handshake_completed() {
                return Ok(());
            }

            // タイムアウト計算
            let expiry = self.conn.get_expiry();
            let now = timestamp();
            let timer_duration = if expiry > now {
                Duration::from_nanos(expiry - now)
            } else {
                Duration::from_millis(1)
            };

            tokio::select! {
                // 受信データを処理
                result = self.socket.recv_from(&mut self.recv_buf) => {
                    match result {
                        Ok((len, from)) => {
                            if from == self.remote_addr {
                                let data = self.recv_buf[..len].to_vec();
                                self.handle_recv(&data).await?;
                            }
                        }
                        Err(e) => {
                            return Err(Error::Internal(format!("recv error: {}", e)));
                        }
                    }
                }

                // タイムアウト
                _ = tokio::time::sleep(timer_duration) => {
                    let ts = timestamp();
                    self.conn.handle_expiry(ts)?;
                }

                // 全体タイムアウト
                _ = tokio::time::sleep_until(deadline) => {
                    return Err(Error::Internal("handshake timeout".to_string()));
                }
            }
        }
    }

    /// イベントループを実行
    pub async fn run(&mut self) -> Result<()> {
        loop {
            // 送信データを処理
            self.flush().await?;

            // 接続が閉じていたら終了
            if self.conn.is_in_closing_period() || self.conn.is_in_draining_period() {
                return Ok(());
            }

            // タイムアウト計算
            let expiry = self.conn.get_expiry();
            let now = timestamp();
            let timer_duration = if expiry > now {
                Duration::from_nanos(expiry - now)
            } else {
                Duration::from_millis(1)
            };

            tokio::select! {
                // 受信データを処理
                result = self.socket.recv_from(&mut self.recv_buf) => {
                    match result {
                        Ok((len, from)) => {
                            if from == self.remote_addr {
                                let data = self.recv_buf[..len].to_vec();
                                self.handle_recv(&data).await?;
                            }
                        }
                        Err(e) => {
                            return Err(Error::Internal(format!("recv error: {}", e)));
                        }
                    }
                }

                // タイムアウト
                _ = tokio::time::sleep(timer_duration) => {
                    let ts = timestamp();
                    self.conn.handle_expiry(ts)?;
                }
            }
        }
    }

    /// 受信データを処理
    async fn handle_recv(&mut self, data: &[u8]) -> Result<()> {
        let ts = timestamp();
        let pkt_info = PacketInfo::default();

        // QUIC パケットを処理
        self.conn
            .read_pkt(&self.local_addr, &self.remote_addr, &pkt_info, data, ts)?;

        // ハンドシェイク完了後、HTTP/3 を処理
        if self.conn.is_handshake_completed() && !self.control_streams_bound {
            self.bind_control_streams()?;
        }

        // 受信したストリームデータを HTTP/3 に渡す
        // ngtcp2 examples に従い、データと fin を一緒に渡す
        while let Some(stream_data) = self.conn.poll_stream_data() {
            let consumed = self.h3_conn.read_stream(
                stream_data.stream_id,
                &stream_data.data,
                stream_data.fin,
                ts,
            )?;

            if consumed > 0 {
                self.conn
                    .extend_max_stream_offset(stream_data.stream_id, consumed as u64)?;
                self.conn.extend_max_offset(consumed as u64);
            }
        }

        Ok(())
    }

    /// 送信データを処理
    pub async fn flush(&mut self) -> Result<()> {
        let ts = timestamp();

        // ハンドシェイク完了後、HTTP/3 ストリームデータを書き込む
        if self.conn.is_handshake_completed() && self.control_streams_bound {
            // HTTP/3 ストリームデータを書き込んでパケットを収集
            let packets = self.write_h3_streams(ts)?;

            // 収集したパケットを送信
            for pkt in packets {
                self.socket
                    .send_to(&pkt, self.remote_addr)
                    .await
                    .map_err(|e| Error::Internal(format!("send error: {}", e)))?;
            }
        }

        // 残りの QUIC パケットを送信
        loop {
            // QUIC パケットを書き込む
            let (written, _pkt_info) = self.conn.write_pkt(&mut self.send_buf, ts)?;

            if written == 0 {
                break;
            }
            // UDP で送信
            self.socket
                .send_to(&self.send_buf[..written], self.remote_addr)
                .await
                .map_err(|e| Error::Internal(format!("send error: {}", e)))?;
        }

        Ok(())
    }

    /// 受信データを処理 (1 回)
    ///
    /// ソケットからデータを受信し、QUIC/HTTP/3 の処理を行う。
    /// タイムアウト時間を指定し、その間に受信がなければ戻る。
    pub async fn recv(&mut self, timeout: Duration) -> Result<()> {
        tokio::select! {
            result = self.socket.recv_from(&mut self.recv_buf) => {
                match result {
                    Ok((len, from)) => {
                        if from == self.remote_addr {
                            let data = self.recv_buf[..len].to_vec();
                            self.handle_recv(&data).await?;
                        }
                    }
                    Err(e) => {
                        return Err(Error::Internal(format!("recv error: {}", e)));
                    }
                }
            }

            _ = tokio::time::sleep(timeout) => {
                // タイムアウト - 何もしない
            }
        }

        // タイムアウト処理
        // handle_recv が ngtcp2 に新しいタイムスタンプを渡した後に
        // 古いタイムスタンプで handle_expiry を呼ぶと単調性アサーションに違反するため、
        // select! の後に新しいタイムスタンプを取得する。
        let ts = timestamp();
        let expiry = self.conn.get_expiry();
        if expiry <= ts {
            self.conn.handle_expiry(ts)?;
        }

        Ok(())
    }

    /// HTTP/3 ストリームデータを書き込み、パケットを収集 (同期処理)
    ///
    /// ngtcp2 examples に従い、NGTCP2_WRITE_STREAM_FLAG_MORE を使用して
    /// 複数のストリームデータを 1 つのパケットにまとめる。
    fn write_h3_streams(&mut self, ts: u64) -> Result<Vec<Vec<u8>>> {
        use nghttp3_sys::nghttp3_vec;

        let mut packets = Vec::new();

        // HTTP/3 から書き込むべきデータを取得
        let mut vecs = [nghttp3_vec {
            base: std::ptr::null_mut(),
            len: 0,
        }; 16];

        while let Ok((stream_id, fin, count)) = self.h3_conn.write_stream(&mut vecs) {
            if count == 0 && !fin {
                break;
            }

            // nghttp3_vec からデータをコピー (ポインタの有効期限問題を回避)
            let mut h3_data = Vec::new();
            for vec in vecs.iter().take(count) {
                if vec.len == 0 || vec.base.is_null() {
                    continue;
                }
                let data = unsafe { std::slice::from_raw_parts(vec.base as *const u8, vec.len) };
                h3_data.extend_from_slice(data);
            }

            // データが空でも FIN が立っている場合は送信する必要がある
            if h3_data.is_empty() && !fin {
                continue;
            }

            // QUIC ストリームに書き込む
            // ngtcp2 examples に従い、特定のエラーをハンドリング
            let result = self
                .conn
                .write_stream(&mut self.send_buf, stream_id, &h3_data, fin, ts);

            match result {
                Ok((pkt_written, data_written)) => {
                    // nghttp3 に書き込んだ量を通知 (WRITE_MORE でもデータは進んでいる)
                    if let Some(dw) = data_written
                        && (dw > 0 || fin)
                    {
                        self.h3_conn.add_write_offset(stream_id, dw)?;
                    }

                    // WRITE_MORE + データなし: パケットが満杯になり、これ以上
                    // ストリームデータを詰められない。パケットは flush の
                    // write_pkt で完成・送信されるため、ここでループを抜ける。
                    if pkt_written == 0 && data_written.is_none() {
                        break;
                    }

                    // パケットをコピーして収集
                    if pkt_written > 0 {
                        packets.push(self.send_buf[..pkt_written].to_vec());
                    }

                    continue;
                }
                Err(Error::StreamDataBlocked(_)) => {
                    // ngtcp2 examples: nghttp3_conn_block_stream を呼び出して続行。
                    // block_stream 後も nghttp3 が同じストリームのデータを返し続ける
                    // 可能性はゼロではないため、次の ACK (extend_max_stream_data
                    // コールバック) で unblock されるまでループを抜ける
                    // (RFC 9000 Section 4.1)。
                    self.h3_conn.block_stream(stream_id);
                    break;
                }
                Err(Error::StreamShutWr(_)) => {
                    // ngtcp2 examples: nghttp3_conn_shutdown_stream_write を呼び出して続行
                    self.h3_conn.shutdown_stream_write(stream_id);
                    continue;
                }
                Err(e) => return Err(e),
            }
        }

        Ok(packets)
    }
}
