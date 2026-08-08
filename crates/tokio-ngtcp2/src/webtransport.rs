//! WebTransport セッション実装
//!
//! HTTP/3 上で WebTransport セッションを管理する。

use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use nghttp3_sys::nghttp3_settings;
use ngtcp2_sys::ngtcp2_transport_params;
use shiguredo_ngtcp2::{
    Connection, ConnectionId, Error, Header, Http3Connection, Http3Event, Http3Settings,
    PacketInfo, Result, SessionId, StreamId, TlsContext, TransportParams, varint,
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
    // 接続マップ
    connections: std::collections::HashMap<SocketAddr, ServerWtConnection>,
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

        Ok(Self {
            socket,
            local_addr,
            tls_ctx,
            transport_params,
            h3_settings,
            connections: std::collections::HashMap::new(),
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
                            self.handle_recv(&data, from, &mut handler).await?;
                        }
                        Err(e) => {
                            eprintln!("[webtransport server] recv error: {}", e);
                            continue;
                        }
                    }
                }

                _ = tokio::time::sleep(timer_duration) => {
                    self.handle_timeouts().await?;
                }
            }

            self.flush_all().await?;
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

    async fn handle_recv<F>(&mut self, data: &[u8], from: SocketAddr, handler: &mut F) -> Result<()>
    where
        F: FnMut(SocketAddr, SessionId, Http3Event) -> bool,
    {
        let ts = timestamp();
        let pkt_info = PacketInfo::default();

        if let Some(conn) = self.connections.get_mut(&from) {
            conn.conn
                .read_pkt(&self.local_addr, &from, &pkt_info, data, ts)?;

            // 受信したストリームデータを HTTP/3 に渡す
            while let Some(stream_data) = conn.conn.poll_stream_data() {
                let consumed = conn.h3_conn.read_stream(
                    stream_data.stream_id,
                    &stream_data.data,
                    stream_data.fin,
                    ts,
                )?;
                if consumed > 0 {
                    conn.conn
                        .extend_max_stream_offset(stream_data.stream_id, consumed as u64)?;
                    conn.conn.extend_max_offset(consumed as u64);
                }
            }

            let handshake_completed = conn.conn.is_handshake_completed();
            if handshake_completed && !conn.control_streams_bound {
                bind_wt_control_streams(conn)?;
            }

            // HTTP/3 イベントを処理
            while let Some(event) = conn.h3_conn.poll_event() {
                // WebTransport CONNECT リクエストを処理
                if let Http3Event::HeadersEnd { stream_id, .. } = &event {
                    let session_id = *stream_id;
                    if handler(from, session_id, event) {
                        // セッションを受け入れ
                        let response_headers = vec![Header::status(200)];
                        conn.h3_conn
                            .submit_wt_response(session_id, &response_headers)?;
                        conn.h3_conn.server_confirm_wt_session(session_id, ts)?;
                        conn.session_id = Some(session_id);
                    }
                } else if let Some(session_id) = conn.session_id {
                    handler(from, session_id, event);
                }
            }

            return Ok(());
        }

        // 新しい接続を作成
        if data.len() < 6 {
            return Ok(());
        }

        let first_byte = data[0];
        if first_byte & 0x80 == 0 {
            // Short header - 既存の接続にルーティングされるべき
            return Ok(());
        }

        // QUIC バージョンを読み取る (bytes 1-4, ビッグエンディアン)
        // 注: 現在はバージョンネゴシエーションを行わないため未使用
        let _version = u32::from_be_bytes([data[1], data[2], data[3], data[4]]);

        // DCID Length (offset 5)
        let dcid_len = data[5] as usize;
        if data.len() < 6 + dcid_len {
            return Ok(());
        }
        let original_dcid_bytes = &data[6..6 + dcid_len];
        let original_dcid = match ConnectionId::new(original_dcid_bytes) {
            Some(cid) => cid,
            None => {
                return Ok(());
            }
        };

        // SCID Length (offset 6 + DCID_len)
        let scid_offset = 6 + dcid_len;
        if data.len() < scid_offset + 1 {
            return Ok(());
        }
        let client_scid_len = data[scid_offset] as usize;
        if data.len() < scid_offset + 1 + client_scid_len {
            return Ok(());
        }
        let client_scid_bytes = &data[scid_offset + 1..scid_offset + 1 + client_scid_len];
        let client_scid = match ConnectionId::new(client_scid_bytes) {
            Some(cid) => cid,
            None => {
                return Ok(());
            }
        };

        let server_scid = ConnectionId::random(16)
            .ok_or(Error::Internal("failed to generate scid".to_string()))?;

        let tls_session = self.tls_ctx.create_session()?;

        // サーバー用のトランスポートパラメータを作成
        // original_dcid はクライアントからの最初の Initial パケットの DCID
        let params = TransportParams::from_raw(self.transport_params)
            .with_original_dcid(&original_dcid)
            .into_raw();

        // server_new の引数:
        // - dcid: クライアントの SCID (サーバーがクライアントに送るパケットの DCID になる)
        // - scid: サーバーの SCID
        let mut conn = Connection::server_new(
            &client_scid,
            &server_scid,
            self.local_addr,
            from,
            tls_session,
            &params,
            ts,
        )?;

        conn.read_pkt(&self.local_addr, &from, &pkt_info, data, ts)?;

        let h3_conn = Http3Connection::server_new(&self.h3_settings)?;

        let server_conn = ServerWtConnection {
            conn,
            h3_conn,
            session_id: None,
            control_streams_bound: false,
            opened_wt_streams: std::collections::HashSet::new(),
        };

        self.connections.insert(from, server_conn);

        Ok(())
    }

    async fn handle_timeouts(&mut self) -> Result<()> {
        let ts = timestamp();

        for conn in self.connections.values_mut() {
            let expiry = conn.conn.get_expiry();
            if expiry <= ts {
                conn.conn.handle_expiry(ts)?;
            }
        }

        Ok(())
    }

    async fn flush_all(&mut self) -> Result<()> {
        let ts = timestamp();

        let addrs: Vec<SocketAddr> = self.connections.keys().copied().collect();

        for addr in addrs {
            // HTTP/3 ストリームデータを書き込み、パケットを収集 (同期処理)
            let h3_packets = if let Some(conn) = self.connections.get_mut(&addr) {
                write_h3_streams_for_wt_connection(conn, &mut self.send_buf, ts)?
            } else {
                Vec::new()
            };

            // 収集した HTTP/3 パケットを送信
            for pkt in h3_packets {
                self.socket
                    .send_to(&pkt, addr)
                    .await
                    .map_err(|e| Error::Internal(format!("send error: {}", e)))?;
            }

            // 残りの QUIC パケットを送信
            if let Some(conn) = self.connections.get_mut(&addr) {
                loop {
                    let (written, _pkt_info) = conn.conn.write_pkt(&mut self.send_buf, ts)?;

                    if written == 0 {
                        break;
                    }

                    self.socket
                        .send_to(&self.send_buf[..written], addr)
                        .await
                        .map_err(|e| Error::Internal(format!("send error: {}", e)))?;
                }
            }
        }

        Ok(())
    }

    fn remove_closed_connections(&mut self) {
        self.connections.retain(|addr, conn| {
            let should_remove =
                conn.conn.is_in_closing_period() || conn.conn.is_in_draining_period();
            if should_remove {
                eprintln!("[webtransport server] connection closed: {}", addr);
            }
            !should_remove
        });
    }

    /// 特定の接続で双方向ストリームを開く
    pub fn open_bidi_stream_for(&mut self, addr: &SocketAddr) -> Result<StreamId> {
        let conn = self
            .connections
            .get_mut(addr)
            .ok_or(Error::Internal(format!("connection not found: {}", addr)))?;
        let session_id = conn
            .session_id
            .ok_or(Error::Internal("session not established".to_string()))?;
        let stream_id = conn.conn.open_bidi_stream()?;
        conn.h3_conn.open_wt_data_stream(session_id, stream_id)?;
        conn.opened_wt_streams.insert(stream_id);
        Ok(stream_id)
    }

    /// 特定の接続のストリームにデータを送信
    pub fn send_stream_data_for(
        &mut self,
        addr: &SocketAddr,
        stream_id: StreamId,
        data: &[u8],
        fin: bool,
    ) -> Result<()> {
        let conn = self
            .connections
            .get_mut(addr)
            .ok_or(Error::Internal(format!("connection not found: {}", addr)))?;
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
                        self.handle_recv(&data, from, handler).await?;
                    }
                    Err(e) => {
                        return Err(Error::Internal(format!("recv error: {}", e)));
                    }
                }
            }
            _ = tokio::time::sleep(timer_duration) => {
                self.handle_timeouts().await?;
            }
        }

        self.flush_all().await?;
        self.remove_closed_connections();
        Ok(())
    }

    /// 確立済みセッションを持つ接続のアドレスを取得
    pub fn get_established_addrs(&self) -> Vec<SocketAddr> {
        self.connections
            .iter()
            .filter(|(_, conn)| conn.session_id.is_some())
            .map(|(addr, _)| *addr)
            .collect()
    }

    /// 送信データをフラッシュする
    pub async fn flush(&mut self) -> Result<()> {
        self.flush_all().await
    }

    /// 特定クライアントに DATAGRAM を送信
    ///
    /// WebTransport セッションを通じて特定のクライアントに DATAGRAM を送信する。
    /// DATAGRAM は信頼性のない配信であり、順序も保証されない。
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
        let conn = self
            .connections
            .get_mut(addr)
            .ok_or(Error::Internal(format!("connection not found: {}", addr)))?;

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
                .send_to(&self.send_buf[..written], *addr)
                .await
                .map_err(|e| Error::Internal(format!("send error: {}", e)))?;
        }

        Ok(accepted)
    }

    /// 特定クライアントから DATAGRAM を受信
    ///
    /// 指定したクライアントの受信キューから DATAGRAM を取り出す。
    /// セッションに属さない DATAGRAM は無視される。
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
        let conn = self.connections.get_mut(addr)?;
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

    /// 特定の接続で単方向ストリームを開く
    pub fn open_uni_stream_for(&mut self, addr: &SocketAddr) -> Result<StreamId> {
        let conn = self
            .connections
            .get_mut(addr)
            .ok_or(Error::Internal(format!("connection not found: {}", addr)))?;
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

    while let Ok((stream_id, fin, count)) = conn.h3_conn.write_stream(&mut vecs) {
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

        if h3_data.is_empty() {
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
