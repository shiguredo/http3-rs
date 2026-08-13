//! WebTransport セッション実装
//!
//! HTTP/3 上で WebTransport セッションを管理する。

use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use nghttp3_sys::nghttp3_settings;
use ngtcp2_sys::ngtcp2_transport_params;
use shiguredo_ngtcp2::{
    Connection, ConnectionErrorKind, ConnectionId, Error, Header, Http3Connection, Http3Event,
    Http3Settings, PacketInfo, QuicVersion, Result, SessionId, StreamId, TlsContext,
    TransportParams, varint,
};

use crate::conn::{
    SERVER_SCID_LEN, feed_stream_data_to_h3, parse_new_connection_packet, resolve_dcid,
    send_connection_close,
};
use crate::{Socket, timestamp};

/// WebTransport クライアントセッション
pub struct ClientWebTransportSession {
    socket: Socket,
    local_addr: SocketAddr,
    remote_addr: SocketAddr,
    // TLS コンテキスト (SSL_CTX を保持するため)
    _tls_ctx: TlsContext,
    // QUIC 接続
    conn: Connection,
    // HTTP/3 接続
    h3_conn: Http3Connection,
    // WebTransport セッション ID
    session_id: Option<SessionId>,
    // 受信バッファ
    recv_buf: Vec<u8>,
    // 送信バッファ
    send_buf: Vec<u8>,
    // コントロールストリームをバインド済みか
    control_streams_bound: bool,
}

// SAFETY: ClientWebTransportSession の全フィールドは Send/Sync を実装している
unsafe impl Send for ClientWebTransportSession {}
unsafe impl Sync for ClientWebTransportSession {}

impl ClientWebTransportSession {
    /// WebTransport セッションを作成
    ///
    /// # Arguments
    ///
    /// * `remote_addr` - 接続先アドレス
    /// * `server_name` - サーバー名 (SNI 兼ホスト名検証、DNS 名限定)
    /// * `path` - WebTransport パス (例: "/webtransport")
    pub async fn connect(remote_addr: SocketAddr, server_name: &str, _path: &str) -> Result<Self> {
        Self::connect_with_options(remote_addr, server_name, _path, true).await
    }

    /// WebTransport セッションを作成 (証明書検証なし)
    ///
    /// テスト用の自己署名証明書で使用する。
    ///
    /// # Arguments
    ///
    /// * `remote_addr` - 接続先アドレス
    /// * `server_name` - サーバー名 (SNI 兼ホスト名検証、DNS 名限定)
    /// * `path` - WebTransport パス (例: "/webtransport")
    pub async fn connect_insecure(
        remote_addr: SocketAddr,
        server_name: &str,
        _path: &str,
    ) -> Result<Self> {
        Self::connect_with_options(remote_addr, server_name, _path, false).await
    }

    /// WebTransport セッションを作成 (カスタム CA 証明書付き)
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
    /// * `path` - WebTransport パス (例: "/webtransport")
    /// * `ca_cert_pem` - CA 証明書の PEM 文字列
    pub async fn connect_with_ca(
        remote_addr: SocketAddr,
        server_name: &str,
        _path: &str,
        ca_cert_pem: &str,
    ) -> Result<Self> {
        Self::connect_with_options_internal(
            remote_addr,
            server_name,
            _path,
            Some(ca_cert_pem),
            true,
        )
        .await
    }

    /// WebTransport セッションを作成 (オプション付き)
    ///
    /// # Arguments
    ///
    /// * `remote_addr` - 接続先アドレス
    /// * `server_name` - サーバー名 (SNI 兼ホスト名検証、DNS 名限定)
    /// * `path` - WebTransport パス (例: "/webtransport")
    /// * `verify_peer` - サーバー証明書を検証するかどうか
    async fn connect_with_options(
        remote_addr: SocketAddr,
        server_name: &str,
        _path: &str,
        verify_peer: bool,
    ) -> Result<Self> {
        Self::connect_with_options_internal(remote_addr, server_name, _path, None, verify_peer)
            .await
    }

    /// 内部接続メソッド (CA 証明書指定付き)
    async fn connect_with_options_internal(
        remote_addr: SocketAddr,
        server_name: &str,
        _path: &str,
        ca_cert_pem: Option<&str>,
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

        // WebTransport 用のトランスポートパラメータ
        let params = TransportParams::new().with_datagram(65535).into_raw();

        // WebTransport 用の HTTP/3 設定
        let h3_settings = Http3Settings::new().with_webtransport().into_raw();

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

        Ok(Self {
            socket,
            local_addr,
            remote_addr,
            _tls_ctx: tls_ctx,
            conn,
            h3_conn,
            session_id: None,
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

    /// セッション ID を取得
    pub fn session_id(&self) -> Option<SessionId> {
        self.session_id
    }

    /// ハンドシェイクを完了する
    pub async fn handshake(&mut self) -> Result<()> {
        let timeout = Duration::from_secs(30);
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            // 送信データを処理
            self.flush().await?;

            // ハンドシェイク完了を確認
            if self.conn.is_handshake_completed() {
                // コントロールストリームをバインド
                if !self.control_streams_bound {
                    self.bind_control_streams()?;
                }
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
                result = self.socket.recv_from(&mut self.recv_buf) => {
                    match result {
                        Ok((len, from)) => {
                            if from == self.remote_addr {
                                let data = self.recv_buf[..len].to_vec();
                                self.handle_recv(&data)?;
                            }
                        }
                        Err(e) => {
                            return Err(Error::Internal(format!("recv error: {}", e)));
                        }
                    }
                }

                _ = tokio::time::sleep(timer_duration) => {
                    let ts = timestamp();
                    self.conn.handle_expiry(ts)?;
                }

                _ = tokio::time::sleep_until(deadline) => {
                    return Err(Error::Internal("handshake timeout".to_string()));
                }
            }
        }
    }

    /// WebTransport セッションを開始
    ///
    /// # Arguments
    ///
    /// * `authority` - ホスト名 (例: "localhost:4433")
    /// * `path` - WebTransport パス (例: "/webtransport")
    pub async fn open_session(&mut self, authority: &str, path: &str) -> Result<SessionId> {
        // ハンドシェイク完了を確認
        if !self.conn.is_handshake_completed() {
            return Err(Error::Internal("handshake not completed".to_string()));
        }

        // SETTINGS 交換を完了するために、送受信ループを実行
        // nghttp3 は相手からの SETTINGS を受信するまで WebTransport リクエストを許可しない
        let timeout = Duration::from_secs(5);
        let deadline = tokio::time::Instant::now() + timeout;

        let headers = vec![
            Header::method("CONNECT"),
            Header::new(b":protocol", b"webtransport"),
            Header::scheme("https"),
            Header::authority(authority),
            Header::path(path),
        ];

        // ストリームは 1 つだけ開いて再利用する
        // (失敗のたびに新しいストリームを開くと未使用ストリームがリークする)
        let mut stream_id = None;

        loop {
            // 接続状態を確認
            if self.conn.is_in_draining_period() {
                return Err(Error::Ngtcp2("ERR_DRAINING".to_string(), -224));
            }
            if self.conn.is_in_closing_period() {
                return Err(Error::Internal("connection closing".to_string()));
            }

            // 送信データを処理
            self.flush().await?;

            // ストリームがまだ開かれていなければ開く
            if stream_id.is_none() {
                stream_id = self.conn.open_bidi_stream().ok();
            }

            // WebTransport CONNECT リクエストを送信
            if let Some(sid) = stream_id {
                match self.h3_conn.submit_wt_request(sid, &headers) {
                    Ok(()) => {
                        // セッション ID を保存 (ストリーム ID がセッション ID になる)
                        self.session_id = Some(sid);
                        // CONNECT リクエストを実際に送信
                        self.flush().await?;
                        return Ok(sid);
                    }
                    Err(Error::Nghttp3(_, -102)) => {
                        // ERR_INVALID_STATE: SETTINGS がまだ交換されていない
                        // 同じストリームで再試行する
                    }
                    Err(e) => {
                        return Err(e);
                    }
                }
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
                result = self.socket.recv_from(&mut self.recv_buf) => {
                    match result {
                        Ok((len, from)) => {
                            if from == self.remote_addr {
                                let data = self.recv_buf[..len].to_vec();
                                self.handle_recv(&data)?;
                            }
                        }
                        Err(e) => {
                            return Err(Error::Internal(format!("recv error: {}", e)));
                        }
                    }
                }

                _ = tokio::time::sleep(timer_duration) => {
                    let ts = timestamp();
                    self.conn.handle_expiry(ts)?;
                }

                _ = tokio::time::sleep_until(deadline) => {
                    return Err(Error::Internal("settings exchange timeout".to_string()));
                }
            }
        }
    }

    /// 双方向ストリームを開く
    pub fn open_bidi_stream(&mut self) -> Result<StreamId> {
        let session_id = self
            .session_id
            .ok_or(Error::Internal("session not established".to_string()))?;

        // QUIC ストリームを開く
        let stream_id = self.conn.open_bidi_stream()?;

        // WebTransport データストリームとして登録
        self.h3_conn.open_wt_data_stream(session_id, stream_id)?;

        Ok(stream_id)
    }

    /// 単方向ストリームを開く
    pub fn open_uni_stream(&mut self) -> Result<StreamId> {
        let session_id = self
            .session_id
            .ok_or(Error::Internal("session not established".to_string()))?;

        // QUIC 単方向ストリームを開く
        let stream_id = self.conn.open_uni_stream()?;

        // WebTransport データストリームとして登録
        self.h3_conn.open_wt_data_stream(session_id, stream_id)?;

        Ok(stream_id)
    }

    /// WebTransport ストリームにデータを送信
    ///
    /// `open_bidi_stream()` で開いたストリームにアプリケーションデータを送信する。
    /// データは nghttp3 の WebTransport フレーミングを通して送信される。
    ///
    /// QUIC 輻輳制御ウィンドウを超えるデータは、ACK を受信しながら複数回に
    /// 分けて送信する。全データが ngtcp2 のバッファに渡るまでブロックする。
    ///
    /// # Arguments
    ///
    /// * `stream_id` - ストリーム ID (`open_bidi_stream()` の戻り値)
    /// * `data` - 送信するデータ
    /// * `fin` - ストリームを終了するかどうか
    pub async fn send_stream_data(
        &mut self,
        stream_id: StreamId,
        data: &[u8],
        fin: bool,
    ) -> Result<()> {
        self.h3_conn.send_wt_stream_data(stream_id, data, fin)?;

        // 全データが ngtcp2 のバッファに渡るまでフラッシュを繰り返す。
        // QUIC 輻輳制御ウィンドウが満杯の場合は StreamDataBlocked が返るため、
        // ACK を受信してウィンドウを広げてからストリームのブロックを解除して再試行する。
        loop {
            let ts = timestamp();
            let (h3_packets, blocked_streams) = self.write_h3_streams_tracked(ts)?;

            for pkt in h3_packets {
                self.socket
                    .send_to(&pkt, self.remote_addr)
                    .await
                    .map_err(|e| Error::Internal(format!("send error: {}", e)))?;
            }

            // 残りの QUIC パケットを送信
            loop {
                let ts = timestamp();
                let (written, _) = self.conn.write_pkt(&mut self.send_buf, ts)?;
                if written == 0 {
                    break;
                }
                self.socket
                    .send_to(&self.send_buf[..written], self.remote_addr)
                    .await
                    .map_err(|e| Error::Internal(format!("send error: {}", e)))?;
            }

            if blocked_streams.is_empty() {
                // ブロックされたストリームなし -> 全データを ngtcp2 に渡した
                break;
            }

            // 輻輳ウィンドウが満杯 -> ACK を受信してウィンドウを広げる
            self.recv_ack(Duration::from_millis(50)).await?;

            // ブロックされていたストリームを再開可能にする
            for sid in &blocked_streams {
                self.h3_conn.unblock_stream(*sid)?;
            }
        }

        Ok(())
    }

    /// イベントをポーリング
    pub fn poll(&mut self) -> Option<Http3Event> {
        self.h3_conn.poll_event()
    }

    /// WebTransport DATAGRAM を送信
    ///
    /// WebTransport セッションを通じて DATAGRAM を送信する。
    /// DATAGRAM は信頼性のない配信であり、順序も保証されない。
    ///
    /// # Arguments
    ///
    /// * `data` - 送信するデータ
    ///
    /// # Returns
    ///
    /// * `Ok(true)` - データが送信キューに追加された
    /// * `Ok(false)` - データが受け入れられなかった (輻輳制御など)
    pub async fn send_datagram(&mut self, data: &[u8]) -> Result<bool> {
        let session_id = self
            .session_id
            .ok_or(Error::Internal("session not established".to_string()))?;

        // リモートピアの DATAGRAM サポートを確認
        if !self.conn.can_send_datagram() {
            return Err(Error::Internal(
                "remote peer does not support DATAGRAM".to_string(),
            ));
        }

        // HTTP/3 DATAGRAM フォーマット: Quarter Stream ID + Payload
        // Quarter Stream ID = session_id / 4
        let quarter_stream_id = session_id as u64 / 4;
        let mut datagram = Vec::with_capacity(8 + data.len());
        varint::encode_to_vec(quarter_stream_id, &mut datagram);
        datagram.extend_from_slice(data);

        // QUIC DATAGRAM として送信
        let ts = timestamp();
        let (written, accepted) = self
            .conn
            .write_datagram(&mut self.send_buf, &datagram, ts)?;

        if written > 0 {
            self.socket
                .send_to(&self.send_buf[..written], self.remote_addr)
                .await
                .map_err(|e| Error::Internal(format!("send error: {}", e)))?;
        }

        Ok(accepted)
    }

    /// WebTransport DATAGRAM を受信
    ///
    /// 受信キューから DATAGRAM を取り出す。
    /// セッションに属さない DATAGRAM は無視される。
    ///
    /// # Returns
    ///
    /// * `Some(data)` - 受信した DATAGRAM のペイロード
    /// * `None` - 受信データなし
    pub fn recv_datagram(&mut self) -> Option<Vec<u8>> {
        let session_id = self.session_id?;
        let expected_quarter_stream_id = session_id as u64 / 4;

        while let Some(datagram) = self.conn.poll_datagram() {
            // Quarter Stream ID をデコード
            if let Some((quarter_stream_id, consumed)) = varint::decode(&datagram.data)
                && quarter_stream_id == expected_quarter_stream_id
            {
                return Some(datagram.data[consumed..].to_vec());
            }
        }

        None
    }

    /// ネットワーク I/O を 1 回実行する
    ///
    /// 送信データをフラッシュし、パケットを 1 回受信して処理する。
    /// 処理後は `poll()` でイベントを取得できる。
    pub async fn recv(&mut self, timeout: Duration) -> Result<()> {
        // 送信データを処理
        self.flush().await?;

        // タイムアウト計算
        let expiry = self.conn.get_expiry();
        let now = timestamp();
        let timer_duration = if expiry > now {
            Duration::from_nanos(expiry - now).min(timeout)
        } else {
            Duration::from_millis(1)
        };

        tokio::select! {
            result = self.socket.recv_from(&mut self.recv_buf) => {
                match result {
                    Ok((len, from)) => {
                        if from == self.remote_addr {
                            let data = self.recv_buf[..len].to_vec();
                            self.handle_recv(&data)?;
                        }
                    }
                    Err(e) => {
                        return Err(Error::Internal(format!("recv error: {}", e)));
                    }
                }
            }
            _ = tokio::time::sleep(timer_duration) => {
                let ts = timestamp();
                self.conn.handle_expiry(ts)?;
            }
        }

        Ok(())
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
                result = self.socket.recv_from(&mut self.recv_buf) => {
                    match result {
                        Ok((len, from)) => {
                            if from == self.remote_addr {
                                let data = self.recv_buf[..len].to_vec();
                                self.handle_recv(&data)?;
                            }
                        }
                        Err(e) => {
                            return Err(Error::Internal(format!("recv error: {}", e)));
                        }
                    }
                }

                _ = tokio::time::sleep(timer_duration) => {
                    let ts = timestamp();
                    self.conn.handle_expiry(ts)?;
                }
            }
        }
    }

    /// コントロールストリームをバインド
    fn bind_control_streams(&mut self) -> Result<()> {
        let ctrl_stream_id = self.conn.open_uni_stream()?;
        self.h3_conn.bind_control_stream(ctrl_stream_id)?;

        let qpack_enc_stream_id = self.conn.open_uni_stream()?;
        let qpack_dec_stream_id = self.conn.open_uni_stream()?;
        self.h3_conn
            .bind_qpack_streams(qpack_enc_stream_id, qpack_dec_stream_id)?;

        self.control_streams_bound = true;
        Ok(())
    }

    /// 受信データを処理
    fn handle_recv(&mut self, data: &[u8]) -> Result<()> {
        let ts = timestamp();
        let pkt_info = PacketInfo::default();
        self.conn
            .read_pkt(&self.local_addr, &self.remote_addr, &pkt_info, data, ts)?;

        // 受信したストリームデータを HTTP/3 に渡す
        self.process_stream_data(ts)?;

        Ok(())
    }

    /// ストリームデータを HTTP/3 に渡す
    fn process_stream_data(&mut self, ts: u64) -> Result<()> {
        while let Some(stream_data) = self.conn.poll_stream_data() {
            // HTTP/3 にストリームデータを渡す
            let consumed = self.h3_conn.read_stream(
                stream_data.stream_id,
                &stream_data.data,
                stream_data.fin,
                ts,
            )?;

            // 消費したデータ分だけオフセットを拡張
            if consumed > 0 {
                self.conn
                    .extend_max_stream_offset(stream_data.stream_id, consumed as u64)?;
                self.conn.extend_max_offset(consumed as u64);
            }
        }
        Ok(())
    }

    /// 送信データを処理
    async fn flush(&mut self) -> Result<()> {
        let ts = timestamp();

        // ハンドシェイク完了後かつ制御ストリームがバインドされている場合、HTTP/3 データを送信
        if self.conn.is_handshake_completed() && self.control_streams_bound {
            // HTTP/3 ストリームデータを同期的に収集
            let h3_packets = self.write_h3_streams(ts)?;

            // パケットを送信
            for pkt in h3_packets {
                self.socket
                    .send_to(&pkt, self.remote_addr)
                    .await
                    .map_err(|e| Error::Internal(format!("send error: {}", e)))?;
            }
        }

        // 残りの QUIC パケットを送信
        loop {
            let (written, _pkt_info) = self.conn.write_pkt(&mut self.send_buf, ts)?;

            if written == 0 {
                break;
            }

            self.socket
                .send_to(&self.send_buf[..written], self.remote_addr)
                .await
                .map_err(|e| Error::Internal(format!("send error: {}", e)))?;
        }

        Ok(())
    }

    /// ACK 受信専用の recv (flush を呼ばない)
    ///
    /// send_stream_data の輻輳制御ループ内で使用する。
    /// 通常の recv() は flush() を先に呼ぶが、このメソッドはそれを省略する。
    async fn recv_ack(&mut self, timeout: Duration) -> Result<()> {
        let expiry = self.conn.get_expiry();
        let now = timestamp();
        let timer_duration = if expiry > now {
            Duration::from_nanos(expiry - now).min(timeout)
        } else {
            Duration::from_millis(1)
        };

        tokio::select! {
            result = self.socket.recv_from(&mut self.recv_buf) => {
                match result {
                    Ok((len, from)) => {
                        if from == self.remote_addr {
                            let data = self.recv_buf[..len].to_vec();
                            self.handle_recv(&data)?;
                        }
                    }
                    Err(e) => {
                        return Err(Error::Internal(format!("recv error: {}", e)));
                    }
                }
            }
            _ = tokio::time::sleep(timer_duration) => {
                let ts = timestamp();
                self.conn.handle_expiry(ts)?;
            }
        }

        Ok(())
    }

    /// HTTP/3 ストリームデータを書き込み、輻輳でブロックされたストリーム ID も返す
    ///
    /// send_stream_data の輻輳制御ループ内で使用する。
    fn write_h3_streams_tracked(&mut self, ts: u64) -> Result<(Vec<Vec<u8>>, Vec<StreamId>)> {
        use nghttp3_sys::nghttp3_vec;

        let mut packets = Vec::new();
        let mut blocked = Vec::new();

        let mut vecs = [nghttp3_vec {
            base: std::ptr::null_mut(),
            len: 0,
        }; 16];

        while let Ok((sid, fin, count)) = self.h3_conn.write_stream(&mut vecs) {
            if count == 0 {
                break;
            }

            let mut h3_data = Vec::new();
            for vec in vecs.iter().take(count) {
                if vec.len == 0 || vec.base.is_null() {
                    continue;
                }
                let data = unsafe { std::slice::from_raw_parts(vec.base as *const u8, vec.len) };
                h3_data.extend_from_slice(data);
            }

            if h3_data.is_empty() {
                continue;
            }

            let result = self
                .conn
                .write_stream(&mut self.send_buf, sid, &h3_data, fin, ts);

            match result {
                Ok((pkt_written, data_written)) => {
                    if pkt_written > 0 {
                        packets.push(self.send_buf[..pkt_written].to_vec());
                    }
                    if let Some(dw) = data_written
                        && dw > 0
                    {
                        self.h3_conn.add_write_offset(sid, dw)?;
                    } else if pkt_written == 0 {
                        // ngtcp2 の輻輳ウィンドウ満杯でデータを書き込めなかった
                        self.h3_conn.block_stream(sid);
                        blocked.push(sid);
                        break;
                    }
                }
                Err(Error::StreamDataBlocked(_)) => {
                    self.h3_conn.block_stream(sid);
                    blocked.push(sid);
                    continue;
                }
                Err(Error::StreamShutWr(_)) => {
                    self.h3_conn.shutdown_stream_write(sid);
                    continue;
                }
                Err(e) => return Err(e),
            }
        }

        Ok((packets, blocked))
    }

    /// HTTP/3 ストリームデータを同期的に書き込み、パケットを収集
    ///
    /// ngtcp2 examples に従い、特定のエラーをハンドリング
    fn write_h3_streams(&mut self, ts: u64) -> Result<Vec<Vec<u8>>> {
        use nghttp3_sys::nghttp3_vec;

        let mut packets = Vec::new();

        // HTTP/3 から書き込むべきデータを取得
        let mut vecs = [nghttp3_vec {
            base: std::ptr::null_mut(),
            len: 0,
        }; 16];

        while let Ok((stream_id, fin, count)) = self.h3_conn.write_stream(&mut vecs) {
            if count == 0 {
                break;
            }

            // nghttp3_vec からデータをコピー
            let mut h3_data = Vec::new();
            for vec in vecs.iter().take(count) {
                if vec.len == 0 || vec.base.is_null() {
                    continue;
                }
                let data = unsafe { std::slice::from_raw_parts(vec.base as *const u8, vec.len) };
                h3_data.extend_from_slice(data);
            }

            if h3_data.is_empty() {
                continue;
            }

            // QUIC ストリームに書き込む
            // ngtcp2 examples に従い、特定のエラーをハンドリング
            let result = self
                .conn
                .write_stream(&mut self.send_buf, stream_id, &h3_data, fin, ts);

            match result {
                Ok((pkt_written, data_written)) => {
                    // パケットをコピーして収集
                    if pkt_written > 0 {
                        packets.push(self.send_buf[..pkt_written].to_vec());
                    }

                    // nghttp3 に書き込んだ量を通知
                    if let Some(dw) = data_written
                        && dw > 0
                    {
                        self.h3_conn.add_write_offset(stream_id, dw)?;
                    }
                }
                Err(Error::StreamDataBlocked(_)) => {
                    self.h3_conn.block_stream(stream_id);
                    continue;
                }
                Err(Error::StreamShutWr(_)) => {
                    self.h3_conn.shutdown_stream_write(stream_id);
                    continue;
                }
                Err(e) => return Err(e),
            }
        }

        Ok(packets)
    }
}

/// WebTransport サーバーセッション
pub struct ServerWebTransportSession {
    socket: Socket,
    local_addr: SocketAddr,
    // TLS コンテキスト
    tls_ctx: TlsContext,
    // トランスポートパラメータ
    transport_params: ngtcp2_transport_params,
    // HTTP/3 設定
    h3_settings: nghttp3_settings,
    // 接続マップ (サーバー SCID -> 接続)
    connections: std::collections::HashMap<ConnectionId, ServerWtConnection>,
    // DCID -> 接続キーのルーティングマップ (RFC 9000 Section 5.2)
    cid_map: std::collections::HashMap<ConnectionId, ConnectionId>,
    // Short header パケットの DCID 照合に使う長さの集合
    //
    // Short header は DCID 長を運ばないため (RFC 9000 Section 17.3)、
    // サーバーが発行した CID の長さで照合する。
    short_cid_lengths: std::collections::BTreeSet<usize>,
    // 受信バッファ
    recv_buf: Vec<u8>,
    // 送信バッファ
    send_buf: Vec<u8>,
}

struct ServerWtConnection {
    conn: Connection,
    h3_conn: Http3Connection,
    // WebTransport セッション ID
    session_id: Option<SessionId>,
    control_streams_bound: bool,
    // open_wt_data_stream 済みのストリーム ID セット
    opened_wt_streams: std::collections::HashSet<StreamId>,
    // クライアントのアドレス
    remote_addr: SocketAddr,
}

// SAFETY: ServerWebTransportSession の全フィールドは Send/Sync を実装している
unsafe impl Send for ServerWebTransportSession {}
unsafe impl Sync for ServerWebTransportSession {}

impl ServerWebTransportSession {
    /// WebTransport サーバーを作成
    pub async fn bind(addr: SocketAddr, cert_path: &Path, key_path: &Path) -> Result<Self> {
        let socket = Socket::bind(addr)
            .await
            .map_err(|e| Error::Internal(format!("failed to bind socket: {}", e)))?;

        let local_addr = socket.local_addr();

        // TLS コンテキストを作成
        let tls_ctx = TlsContext::new_server(cert_path, key_path, &[b"h3"])?;

        // WebTransport 用のトランスポートパラメータ (DATAGRAM 有効)
        let transport_params = TransportParams::new().with_datagram(65535).into_raw();

        // WebTransport 用の HTTP/3 設定
        let h3_settings = Http3Settings::new().with_webtransport().into_raw();

        // サーバーが発行する CID はすべて SERVER_SCID_LEN の長さ
        let mut short_cid_lengths = std::collections::BTreeSet::new();
        short_cid_lengths.insert(SERVER_SCID_LEN);

        Ok(Self {
            socket,
            local_addr,
            tls_ctx,
            transport_params,
            h3_settings,
            connections: std::collections::HashMap::new(),
            cid_map: std::collections::HashMap::new(),
            short_cid_lengths,
            recv_buf: vec![0u8; 65535],
            send_buf: vec![0u8; 1350],
        })
    }

    /// ローカルアドレスを取得
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// サーバーを実行
    ///
    /// # Arguments
    ///
    /// * `handler` - WebTransport イベントハンドラ
    ///   - 引数: (クライアントアドレス, セッション ID, HTTP/3 イベント)
    ///   - 戻り値: true でセッションを受け入れ
    ///
    /// # エラー処理
    ///
    /// パケット処理・ストリーム処理のエラーは接続単位で処理され、サーバーループは
    /// 継続する。そのためこのメソッドはエラーを返さない (後方互換のため戻り値型は
    /// Result のまま)。
    pub async fn run<F>(&mut self, mut handler: F) -> Result<()>
    where
        F: FnMut(SocketAddr, SessionId, Http3Event) -> bool,
    {
        loop {
            let timer_duration = self.compute_timer_duration();

            tokio::select! {
                result = self.socket.recv_from(&mut self.recv_buf) => {
                    match result {
                        Ok((len, from)) => {
                            let data = self.recv_buf[..len].to_vec();
                            self.handle_recv(&data, from, &mut handler).await;
                        }
                        Err(e) => {
                            eprintln!("[webtransport server] recv error: {}", e);
                            continue;
                        }
                    }
                }

                _ = tokio::time::sleep(timer_duration) => {
                    self.handle_timeouts().await;
                }
            }

            self.flush_all().await;
            self.remove_closed_connections();
        }
    }

    fn compute_timer_duration(&self) -> Duration {
        let now = timestamp();
        let mut min_duration = Duration::from_secs(1);

        for conn in self.connections.values() {
            let expiry = conn.conn.get_expiry();
            if expiry > now {
                let duration = Duration::from_nanos(expiry - now);
                if duration < min_duration {
                    min_duration = duration;
                }
            } else {
                return Duration::from_millis(1);
            }
        }

        min_duration
    }

    /// 受信データを処理する
    ///
    /// 到着パケットを DCID で既存・新規の接続に振り分ける (RFC 9000 Section 5.2)。
    /// エラーは接続単位で処理し、サーバーループは継続する。
    async fn handle_recv<F>(&mut self, data: &[u8], from: SocketAddr, handler: &mut F)
    where
        F: FnMut(SocketAddr, SessionId, Http3Event) -> bool,
    {
        // DCID で既存接続を検索
        if let Some(conn_key) = resolve_dcid(&self.cid_map, &self.short_cid_lengths, data) {
            self.handle_existing_connection(&conn_key, data, from, handler)
                .await;
            return;
        }

        // DCID 未登録のパケット: Long header なら新規接続、Short header なら破棄する
        if data.is_empty() || data[0] & 0x80 == 0 {
            return;
        }
        self.handle_new_connection(data, from).await;
    }

    /// 既存接続へのパケットを処理する
    async fn handle_existing_connection<F>(
        &mut self,
        conn_key: &ConnectionId,
        data: &[u8],
        from: SocketAddr,
        handler: &mut F,
    ) where
        F: FnMut(SocketAddr, SessionId, Http3Event) -> bool,
    {
        let ts = timestamp();
        let pkt_info = PacketInfo::default();

        // QUIC パケットを処理する
        let read_result = match self.connections.get_mut(conn_key) {
            Some(conn) => conn
                .conn
                .read_pkt(&self.local_addr, &from, &pkt_info, data, ts),
            None => return,
        };
        if let Err(e) = read_result {
            self.handle_conn_error(conn_key, e).await;
            return;
        }

        // 受信したストリームデータを HTTP/3 に渡す
        let step_result = match self.connections.get_mut(conn_key) {
            Some(conn) => feed_stream_data_to_h3(&mut conn.conn, &mut conn.h3_conn, ts),
            None => return,
        };
        if let Err(e) = step_result {
            self.handle_conn_error(conn_key, e).await;
            return;
        }

        // ハンドシェイク完了後、コントロールストリームをバインド
        let bind_result = match self.connections.get_mut(conn_key) {
            Some(conn) if conn.conn.is_handshake_completed() && !conn.control_streams_bound => {
                bind_wt_control_streams(conn)
            }
            _ => Ok(()),
        };
        if let Err(e) = bind_result {
            self.handle_conn_error(conn_key, e).await;
            return;
        }

        // HTTP/3 イベントを処理
        loop {
            let event = match self.connections.get_mut(conn_key) {
                Some(conn) => conn.h3_conn.poll_event(),
                None => return,
            };
            let Some(event) = event else {
                break;
            };

            // WebTransport CONNECT リクエストを処理
            if let Http3Event::HeadersEnd { stream_id, .. } = &event {
                let session_id = *stream_id;
                if handler(from, session_id, event) {
                    // セッションを受け入れ
                    let step_result = match self.connections.get_mut(conn_key) {
                        Some(conn) => {
                            let response_headers = vec![Header::status(200)];
                            match conn
                                .h3_conn
                                .submit_wt_response(session_id, &response_headers)
                            {
                                Ok(()) => {
                                    match conn.h3_conn.server_confirm_wt_session(session_id, ts) {
                                        Ok(()) => {
                                            conn.session_id = Some(session_id);
                                            Ok(())
                                        }
                                        Err(e) => Err(e),
                                    }
                                }
                                Err(e) => Err(e),
                            }
                        }
                        None => return,
                    };
                    if let Err(e) = step_result {
                        self.handle_conn_error(conn_key, e).await;
                        return;
                    }
                }
            } else if let Some(session_id) =
                self.connections.get(conn_key).and_then(|c| c.session_id)
            {
                handler(from, session_id, event);
            }
        }
    }

    /// 新規接続を作成する
    ///
    /// Initial の処理は接続をルーティングテーブルに登録する前に行う。
    /// 不正な Initial でエラーが返ってもサーバーは継続する
    /// (RFC 9000 Section 11.1: Initial の AEAD は強力な認証を提供しないため、
    /// 不正な Initial パケットは破棄してよい)。復号に成功した上での致命的な
    /// エラー (NGTCP2_ERR_CRYPTO 等) は、状態を保持せずに CONNECTION_CLOSE を
    /// 送って破棄する (RFC 9000 Section 10.2.3: 状態を確立していないサーバーは
    /// closing 状態に入らない)。
    async fn handle_new_connection(&mut self, data: &[u8], from: SocketAddr) {
        // 新規接続パケットをパースする (RFC 9000 Section 17.2)
        let Some(info) = parse_new_connection_packet(data) else {
            return;
        };

        // サポート外の QUIC バージョンは接続状態を作らずに破棄する
        // (RFC 9000 Section 5.2.2 の Version Negotiation パケット送信は未実装)
        if info.version != QuicVersion::V1 as u32 {
            return;
        }

        let server_scid = match ConnectionId::random(SERVER_SCID_LEN) {
            Some(cid) => cid,
            None => {
                eprintln!("[webtransport server] failed to generate scid");
                return;
            }
        };

        let ts = timestamp();

        let tls_session = match self.tls_ctx.create_session() {
            Ok(session) => session,
            Err(e) => {
                eprintln!("[webtransport server] failed to create TLS session: {}", e);
                return;
            }
        };

        // サーバー用のトランスポートパラメータを作成して QUIC 接続を作成する
        // server_new の引数:
        // - dcid: クライアントの SCID (サーバーがクライアントに送るパケットの DCID になる)
        // - scid: サーバーの SCID
        // トランスポートパラメータは await をまたいで保持しないようブロックに閉じる
        // (ngtcp2_transport_params は Send でないポインタフィールドを持つため)
        let mut conn = {
            // original_dcid はクライアントからの最初の Initial パケットの DCID
            let params = TransportParams::from_raw(self.transport_params)
                .with_original_dcid(&info.original_dcid)
                .into_raw();

            match Connection::server_new(
                &info.client_scid,
                &server_scid,
                self.local_addr,
                from,
                tls_session,
                &params,
                ts,
            ) {
                Ok(conn) => conn,
                Err(e) => {
                    eprintln!("[webtransport server] failed to create connection: {}", e);
                    return;
                }
            }
        };

        // 最初のパケットを処理する
        // エラーは接続単位で処理し、サーバーは継続する
        let pkt_info = PacketInfo::default();
        if let Err(e) = conn.read_pkt(&self.local_addr, &from, &pkt_info, data, ts) {
            match e.classify_connection_error() {
                ConnectionErrorKind::TransportClose | ConnectionErrorKind::ApplicationClose => {
                    // 致命的エラー: CONNECTION_CLOSE を送って接続状態は破棄する
                    send_connection_close(&mut conn, &self.socket, from, &mut self.send_buf, &e)
                        .await;
                }
                _ => {
                    // 黙って破棄する
                }
            }
            return;
        }

        let h3_conn = match Http3Connection::server_new(&self.h3_settings) {
            Ok(h3_conn) => h3_conn,
            Err(e) => {
                eprintln!(
                    "[webtransport server] failed to create HTTP/3 connection: {}",
                    e
                );
                return;
            }
        };

        // ルーティングテーブルに登録する
        self.cid_map
            .insert(info.original_dcid.clone(), server_scid.clone());
        self.cid_map
            .insert(server_scid.clone(), server_scid.clone());
        self.connections.insert(
            server_scid,
            ServerWtConnection {
                conn,
                h3_conn,
                session_id: None,
                control_streams_bound: false,
                opened_wt_streams: std::collections::HashSet::new(),
                remote_addr: from,
            },
        );
    }

    /// 接続単位のエラー処理
    ///
    /// エラーの分類は ngtcp2 の API 契約に従う (`ngtcp2_conn_read_pkt` /
    /// `ngtcp2_conn_handle_expiry` のドキュメント参照)。サーバーループには
    /// エラーを伝播させない。
    async fn handle_conn_error(&mut self, conn_key: &ConnectionId, err: Error) {
        match err.classify_connection_error() {
            ConnectionErrorKind::Ignore => {
                // パケットの破棄指示やストリーム単位のシグナル。何もしない
            }
            ConnectionErrorKind::SilentDrop => {
                // 接続を黙って破棄する
                self.remove_connection(conn_key);
            }
            ConnectionErrorKind::Terminal => {
                // closing / draining 状態に移行済み。remove_closed_connections が除去する
            }
            ConnectionErrorKind::TransportClose | ConnectionErrorKind::ApplicationClose => {
                eprintln!("[webtransport server] closing connection: {}", err);
                let Some(remote) = self.connections.get(conn_key).map(|c| c.remote_addr) else {
                    return;
                };
                let sent = {
                    let Some(conn) = self.connections.get_mut(conn_key) else {
                        return;
                    };
                    // CONNECTION_CLOSE を送ると closing 状態になり、
                    // remove_closed_connections が除去する
                    send_connection_close(
                        &mut conn.conn,
                        &self.socket,
                        remote,
                        &mut self.send_buf,
                        &err,
                    )
                    .await
                };
                if !sent {
                    // CONNECTION_CLOSE を書き込めない場合は黙って破棄する
                    self.remove_connection(conn_key);
                }
            }
            ConnectionErrorKind::Internal => {
                // 内部エラー。CONNECTION_CLOSE は送らず、接続を破棄して継続する
                eprintln!("[webtransport server] connection error: {}", err);
                self.remove_connection(conn_key);
            }
        }
    }

    async fn handle_timeouts(&mut self) {
        let ts = timestamp();

        let keys: Vec<ConnectionId> = self.connections.keys().cloned().collect();
        for conn_key in keys {
            let expired = match self.connections.get(&conn_key) {
                Some(conn) => conn.conn.get_expiry() <= ts,
                None => continue,
            };
            if !expired {
                continue;
            }
            let result = match self.connections.get_mut(&conn_key) {
                Some(conn) => conn.conn.handle_expiry(ts),
                None => continue,
            };
            if let Err(e) = result {
                // NGTCP2_ERR_IDLE_CLOSE などは接続単位で処理する
                self.handle_conn_error(&conn_key, e).await;
            }
        }
    }

    async fn flush_all(&mut self) {
        let ts = timestamp();

        let keys: Vec<ConnectionId> = self.connections.keys().cloned().collect();

        for conn_key in keys {
            let Some(remote) = self.connections.get(&conn_key).map(|c| c.remote_addr) else {
                continue;
            };

            // HTTP/3 ストリームデータを書き込み、パケットを収集 (同期処理)
            let h3_result = match self.connections.get_mut(&conn_key) {
                Some(conn) => write_h3_streams_for_wt_connection(conn, &mut self.send_buf, ts),
                None => continue,
            };
            let h3_packets = match h3_result {
                Ok(packets) => packets,
                Err(e) => {
                    self.handle_conn_error(&conn_key, e).await;
                    continue;
                }
            };

            // 収集した HTTP/3 パケットを送信
            let mut send_failed = false;
            for pkt in h3_packets {
                if let Err(e) = self.socket.send_to(&pkt, remote).await {
                    eprintln!("[webtransport server] send error: {}", e);
                    self.remove_connection(&conn_key);
                    send_failed = true;
                    break;
                }
            }
            if send_failed {
                continue;
            }

            // 残りの QUIC パケットを送信
            while let Some(conn) = self.connections.get_mut(&conn_key) {
                match conn.conn.write_pkt(&mut self.send_buf, ts) {
                    Ok((0, _)) => break,
                    Ok((written, _)) => {
                        if let Err(e) = self.socket.send_to(&self.send_buf[..written], remote).await
                        {
                            eprintln!("[webtransport server] send error: {}", e);
                            self.remove_connection(&conn_key);
                            break;
                        }
                    }
                    Err(e) => {
                        // 送信経路のエラーも接続単位で処理する
                        self.handle_conn_error(&conn_key, e).await;
                        break;
                    }
                }
            }

            // NEW_CONNECTION_ID で発行した CID をルーティングテーブルに登録する
            // 発行は write_stream / write_pkt の中で行われるため、送信後に回収する
            // (RFC 9000 Section 5.1.1: 発行した CID を運ぶパケットは受け付ける MUST)
            if let Some(conn) = self.connections.get_mut(&conn_key) {
                for cid in conn.conn.poll_issued_cids() {
                    self.short_cid_lengths.insert(cid.len());
                    self.cid_map.insert(cid, conn_key.clone());
                }
            }
        }
    }

    /// クローズした接続を削除
    ///
    /// closing / draining 期間に入った接続は即座に除去する。
    /// CONNECTION_CLOSE の再送 (RFC 9000 Section 11.1 の SHOULD) と
    /// draining 期間の維持 (RFC 9000 Section 10.2) は行わない選択であり、
    /// 終了状態の接続を保持し続けるコストを避けるための DoS 対策としての
    /// 意図的な逸脱。除去後に届くパケットは未知 DCID として破棄される
    /// (RFC 9000 Section 5.2.2)。
    fn remove_closed_connections(&mut self) {
        let closed: Vec<ConnectionId> = self
            .connections
            .iter()
            .filter(|(_, conn)| {
                conn.conn.is_in_closing_period() || conn.conn.is_in_draining_period()
            })
            .map(|(key, _)| key.clone())
            .collect();

        for conn_key in closed {
            self.remove_connection(&conn_key);
        }
    }

    /// 接続をマップとルーティングテーブルから除去する
    fn remove_connection(&mut self, conn_key: &ConnectionId) {
        self.connections.remove(conn_key);
        self.cid_map.retain(|_, key| key != conn_key);
    }

    /// アドレスから接続キーを解決する (旧 API 用)
    ///
    /// 同一アドレスから複数の接続が張られている場合は接続を一意に特定できない
    /// ためエラーを返す。
    fn find_conn_key_by_addr(&self, addr: &SocketAddr) -> Result<ConnectionId> {
        let mut keys = self
            .connections
            .iter()
            .filter(|(_, conn)| conn.remote_addr == *addr)
            .map(|(key, _)| key.clone());
        let Some(conn_key) = keys.next() else {
            return Err(Error::Internal(format!("connection not found: {}", addr)));
        };
        if keys.next().is_some() {
            return Err(Error::Internal(format!(
                "multiple connections from the same address: {}",
                addr
            )));
        }
        Ok(conn_key)
    }

    /// 特定の接続で双方向ストリームを開く (クライアントアドレス指定)
    ///
    /// 旧 API。複数接続を扱う場合は `open_bidi_stream_by_conn_id` を使うこと。
    pub fn open_bidi_stream_for(&mut self, addr: &SocketAddr) -> Result<StreamId> {
        let conn_key = self.find_conn_key_by_addr(addr)?;
        self.open_bidi_stream_by_conn_id(&conn_key)
    }

    /// 特定の接続で双方向ストリームを開く (コネクション ID 指定)
    pub fn open_bidi_stream_by_conn_id(&mut self, conn_id: &ConnectionId) -> Result<StreamId> {
        let conn = self
            .connections
            .get_mut(conn_id)
            .ok_or(Error::Internal(format!(
                "connection not found: {}",
                conn_id
            )))?;
        let session_id = conn
            .session_id
            .ok_or(Error::Internal("session not established".to_string()))?;
        let stream_id = conn.conn.open_bidi_stream()?;
        conn.h3_conn.open_wt_data_stream(session_id, stream_id)?;
        conn.opened_wt_streams.insert(stream_id);
        Ok(stream_id)
    }

    /// 特定の接続のストリームにデータを送信 (クライアントアドレス指定)
    ///
    /// 旧 API。複数接続を扱う場合は `send_stream_data_by_conn_id` を使うこと。
    pub fn send_stream_data_for(
        &mut self,
        addr: &SocketAddr,
        stream_id: StreamId,
        data: &[u8],
        fin: bool,
    ) -> Result<()> {
        let conn_key = self.find_conn_key_by_addr(addr)?;
        self.send_stream_data_by_conn_id(&conn_key, stream_id, data, fin)
    }

    /// 特定の接続のストリームにデータを送信 (コネクション ID 指定)
    pub fn send_stream_data_by_conn_id(
        &mut self,
        conn_id: &ConnectionId,
        stream_id: StreamId,
        data: &[u8],
        fin: bool,
    ) -> Result<()> {
        let conn = self
            .connections
            .get_mut(conn_id)
            .ok_or(Error::Internal(format!(
                "connection not found: {}",
                conn_id
            )))?;
        let session_id = conn
            .session_id
            .ok_or(Error::Internal("session not established".to_string()))?;
        // クライアント開始ストリームへの初回書き込み前に open_wt_data_stream が必要
        if conn.opened_wt_streams.insert(stream_id) {
            conn.h3_conn.open_wt_data_stream(session_id, stream_id)?;
        }
        conn.h3_conn.send_wt_stream_data(stream_id, data, fin)?;
        Ok(())
    }

    /// パケットを 1 回受信して処理する
    ///
    /// `run()` のように無限ループせず、1 回の受信処理とフラッシュを行う。
    /// ストリームの開設やデータ送信と組み合わせて使用する。
    pub async fn recv_once<F>(&mut self, timeout_duration: Duration, handler: &mut F) -> Result<()>
    where
        F: FnMut(SocketAddr, SessionId, Http3Event) -> bool,
    {
        let timer_duration = self.compute_timer_duration().min(timeout_duration);

        tokio::select! {
            result = self.socket.recv_from(&mut self.recv_buf) => {
                match result {
                    Ok((len, from)) => {
                        let data = self.recv_buf[..len].to_vec();
                        self.handle_recv(&data, from, handler).await;
                    }
                    Err(e) => {
                        return Err(Error::Internal(format!("recv error: {}", e)));
                    }
                }
            }
            _ = tokio::time::sleep(timer_duration) => {
                self.handle_timeouts().await;
            }
        }

        self.flush_all().await;
        self.remove_closed_connections();
        Ok(())
    }

    /// 確立済みセッションを持つ接続のアドレスを取得
    pub fn get_established_addrs(&self) -> Vec<SocketAddr> {
        self.connections
            .iter()
            .filter(|(_, conn)| conn.session_id.is_some())
            .map(|(_, conn)| conn.remote_addr)
            .collect()
    }

    /// 確立済みセッションを持つ接続のコネクション ID を取得
    pub fn get_established_conn_ids(&self) -> Vec<ConnectionId> {
        self.connections
            .iter()
            .filter(|(_, conn)| conn.session_id.is_some())
            .map(|(key, _)| key.clone())
            .collect()
    }

    /// 送信データをフラッシュする
    pub async fn flush(&mut self) -> Result<()> {
        self.flush_all().await;
        Ok(())
    }

    /// 特定クライアントに DATAGRAM を送信 (クライアントアドレス指定)
    ///
    /// WebTransport セッションを通じて特定のクライアントに DATAGRAM を送信する。
    /// DATAGRAM は信頼性のない配信であり、順序も保証されない。
    ///
    /// 旧 API。複数接続を扱う場合は `send_datagram_by_conn_id` を使うこと。
    ///
    /// # Arguments
    ///
    /// * `addr` - 送信先クライアントのアドレス
    /// * `data` - 送信するデータ
    ///
    /// # Returns
    ///
    /// * `Ok(true)` - データが送信キューに追加された
    /// * `Ok(false)` - データが受け入れられなかった (輻輳制御など)
    pub async fn send_datagram_for(&mut self, addr: &SocketAddr, data: &[u8]) -> Result<bool> {
        let conn_key = self.find_conn_key_by_addr(addr)?;
        self.send_datagram_by_conn_id(&conn_key, data).await
    }

    /// 特定の接続に DATAGRAM を送信 (コネクション ID 指定)
    ///
    /// # Arguments
    ///
    /// * `conn_id` - 送信先のコネクション ID
    /// * `data` - 送信するデータ
    ///
    /// # Returns
    ///
    /// * `Ok(true)` - データが送信キューに追加された
    /// * `Ok(false)` - データが受け入れられなかった (輻輳制御など)
    pub async fn send_datagram_by_conn_id(
        &mut self,
        conn_id: &ConnectionId,
        data: &[u8],
    ) -> Result<bool> {
        let conn = self
            .connections
            .get_mut(conn_id)
            .ok_or(Error::Internal(format!(
                "connection not found: {}",
                conn_id
            )))?;

        let session_id = conn
            .session_id
            .ok_or(Error::Internal("session not established".to_string()))?;

        // リモートピアの DATAGRAM サポートを確認
        if !conn.conn.can_send_datagram() {
            return Err(Error::Internal(
                "remote peer does not support DATAGRAM".to_string(),
            ));
        }

        // HTTP/3 DATAGRAM フォーマット: Quarter Stream ID + Payload
        // Quarter Stream ID = session_id / 4
        let quarter_stream_id = session_id as u64 / 4;
        let mut datagram = Vec::with_capacity(8 + data.len());
        varint::encode_to_vec(quarter_stream_id, &mut datagram);
        datagram.extend_from_slice(data);

        // QUIC DATAGRAM として送信
        let ts = timestamp();
        let (written, accepted) = conn
            .conn
            .write_datagram(&mut self.send_buf, &datagram, ts)?;

        if written > 0 {
            self.socket
                .send_to(&self.send_buf[..written], conn.remote_addr)
                .await
                .map_err(|e| Error::Internal(format!("send error: {}", e)))?;
        }

        // write_datagram の中で NEW_CONNECTION_ID が発行された場合は
        // すぐにルーティングテーブルへ登録する (RFC 9000 Section 5.1.1)
        for cid in conn.conn.poll_issued_cids() {
            self.short_cid_lengths.insert(cid.len());
            self.cid_map.insert(cid, conn_id.clone());
        }

        Ok(accepted)
    }

    /// 特定クライアントから DATAGRAM を受信 (クライアントアドレス指定)
    ///
    /// 指定したクライアントの受信キューから DATAGRAM を取り出す。
    /// セッションに属さない DATAGRAM は無視される。
    ///
    /// 旧 API。複数接続を扱う場合は `recv_datagram_by_conn_id` を使うこと。
    ///
    /// # Arguments
    ///
    /// * `addr` - クライアントのアドレス
    ///
    /// # Returns
    ///
    /// * `Some(data)` - 受信した DATAGRAM のペイロード
    /// * `None` - 受信データなし
    pub fn recv_datagram_for(&mut self, addr: &SocketAddr) -> Option<Vec<u8>> {
        let conn_key = self.find_conn_key_by_addr(addr).ok()?;
        self.recv_datagram_by_conn_id(&conn_key)
    }

    /// 特定の接続から DATAGRAM を受信 (コネクション ID 指定)
    ///
    /// # Returns
    ///
    /// * `Some(data)` - 受信した DATAGRAM のペイロード
    /// * `None` - 受信データなし
    pub fn recv_datagram_by_conn_id(&mut self, conn_id: &ConnectionId) -> Option<Vec<u8>> {
        let conn = self.connections.get_mut(conn_id)?;
        let session_id = conn.session_id?;
        let expected_quarter_stream_id = session_id as u64 / 4;

        while let Some(datagram) = conn.conn.poll_datagram() {
            // Quarter Stream ID をデコード
            if let Some((quarter_stream_id, consumed)) = varint::decode(&datagram.data)
                && quarter_stream_id == expected_quarter_stream_id
            {
                return Some(datagram.data[consumed..].to_vec());
            }
        }

        None
    }

    /// 特定の接続で単方向ストリームを開く (クライアントアドレス指定)
    ///
    /// 旧 API。複数接続を扱う場合は `open_uni_stream_by_conn_id` を使うこと。
    pub fn open_uni_stream_for(&mut self, addr: &SocketAddr) -> Result<StreamId> {
        let conn_key = self.find_conn_key_by_addr(addr)?;
        self.open_uni_stream_by_conn_id(&conn_key)
    }

    /// 特定の接続で単方向ストリームを開く (コネクション ID 指定)
    pub fn open_uni_stream_by_conn_id(&mut self, conn_id: &ConnectionId) -> Result<StreamId> {
        let conn = self
            .connections
            .get_mut(conn_id)
            .ok_or(Error::Internal(format!(
                "connection not found: {}",
                conn_id
            )))?;
        let session_id = conn
            .session_id
            .ok_or(Error::Internal("session not established".to_string()))?;
        let stream_id = conn.conn.open_uni_stream()?;
        conn.h3_conn.open_wt_data_stream(session_id, stream_id)?;
        conn.opened_wt_streams.insert(stream_id);
        Ok(stream_id)
    }
}

fn bind_wt_control_streams(conn: &mut ServerWtConnection) -> Result<()> {
    let ctrl_stream_id = conn.conn.open_uni_stream()?;
    conn.h3_conn.bind_control_stream(ctrl_stream_id)?;

    let qpack_enc_stream_id = conn.conn.open_uni_stream()?;
    let qpack_dec_stream_id = conn.conn.open_uni_stream()?;
    conn.h3_conn
        .bind_qpack_streams(qpack_enc_stream_id, qpack_dec_stream_id)?;

    conn.control_streams_bound = true;
    Ok(())
}

/// HTTP/3 ストリームデータを書き込み、パケットを収集 (同期処理)
///
/// ngtcp2 examples に従い、特定のエラーをハンドリング
fn write_h3_streams_for_wt_connection(
    conn: &mut ServerWtConnection,
    send_buf: &mut [u8],
    ts: u64,
) -> Result<Vec<Vec<u8>>> {
    use nghttp3_sys::nghttp3_vec;

    let mut packets = Vec::new();

    // ハンドシェイク完了後のみ HTTP/3 ストリームを処理
    if !conn.conn.is_handshake_completed() || !conn.control_streams_bound {
        return Ok(packets);
    }

    // HTTP/3 から書き込むべきデータを取得
    let mut vecs = [nghttp3_vec {
        base: std::ptr::null_mut(),
        len: 0,
    }; 16];

    loop {
        // H3 層のエラーは呼び出し側で接続単位に処理する
        let (stream_id, fin, count) = conn.h3_conn.write_stream(&mut vecs)?;
        if count == 0 {
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
        let result = conn
            .conn
            .write_stream(send_buf, stream_id, &h3_data, fin, ts);

        match result {
            Ok((pkt_written, data_written)) => {
                // パケットをコピーして収集
                if pkt_written > 0 {
                    packets.push(send_buf[..pkt_written].to_vec());
                }

                // nghttp3 に書き込んだ量を通知
                if let Some(dw) = data_written
                    && dw > 0
                {
                    conn.h3_conn.add_write_offset(stream_id, dw)?;
                }
            }
            Err(Error::StreamDataBlocked(_)) => {
                conn.h3_conn.block_stream(stream_id);
                continue;
            }
            Err(Error::StreamShutWr(_)) => {
                conn.h3_conn.shutdown_stream_write(stream_id);
                continue;
            }
            Err(e) => {
                return Err(e);
            }
        }
    }

    Ok(packets)
}
