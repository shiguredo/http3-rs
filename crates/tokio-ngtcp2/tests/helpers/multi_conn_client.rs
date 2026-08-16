//! 1 つの UDP ソケットで複数の QUIC 接続を駆動するテスト用クライアント
//!
//! `tokio-ngtcp2` の Server は同一 SocketAddr からの複数接続を DCID で
//! ルーティングする (RFC 9000 Section 5.1)。その挙動を検証するために、
//! 単一の UDP ソケットで複数の QUIC 接続を張り、受信パケットを DCID で
//! 各接続に振り分けるテスト用クライアントを提供する。
//!
//! 高レベル API の `Connection::client_new` はコールバックと TLS 設定を内部で
//! 行いソケットを所有しないため、外部の 1 ソケットで駆動できる。

use std::collections::{BTreeSet, HashMap};
use std::net::SocketAddr;
use std::time::Duration;

use shiguredo_ngtcp2::{
    Connection, ConnectionId, Error, Header, Http3Connection, Http3Event, Http3Settings,
    PacketInfo, Result, StreamId, TlsContext, TransportParams,
};
use tokio::net::UdpSocket;

/// テスト用クライアントの SCID の長さ (バイト)
///
/// サーバーが送るパケットの DCID はクライアントが発行した CID になる。
/// 発行する CID はすべてこの長さのため、Short header の DCID はこの長さで
/// 照合できる。
const CLIENT_SCID_LEN: usize = 16;

/// テスト内でのタイムスタンプ (ナノ秒)
///
/// `tokio-ngtcp2` の `timestamp` は pub(crate) のため、テスト側で同等のものを用意する。
fn timestamp() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let start = START.get_or_init(std::time::Instant::now);
    start.elapsed().as_nanos() as u64
}

/// 1 つの UDP ソケットで複数の QUIC 接続を駆動するテスト用クライアント
pub struct MultiConnClient {
    socket: UdpSocket,
    local_addr: SocketAddr,
    server_addr: SocketAddr,
    // TLS コンテキスト (SSL_CTX を保持するため)
    _tls_ctx: TlsContext,
    // クライアント SCID -> 接続
    conns: HashMap<ConnectionId, TestConn>,
    // サーバーが送るパケットの DCID -> クライアント SCID
    cid_map: HashMap<ConnectionId, ConnectionId>,
    // Short header の DCID 照合に使う長さの集合
    short_lengths: BTreeSet<usize>,
    // 受信バッファ
    recv_buf: Vec<u8>,
    // 送信バッファ
    send_buf: Vec<u8>,
}

struct TestConn {
    conn: Connection,
    h3_conn: Http3Connection,
    control_streams_bound: bool,
}

impl MultiConnClient {
    /// サーバーへ接続するテスト用クライアントを作成する
    ///
    /// UDP ソケットは OS にローカルポートを割り当てさせる
    /// (同一ポートへの 2 ソケット bind は EADDRINUSE で失敗するため固定しない)。
    pub async fn new(server_addr: SocketAddr) -> Result<Self> {
        let local_addr: SocketAddr = if server_addr.is_ipv4() {
            "127.0.0.1:0".parse().expect("test must succeed")
        } else {
            "[::1]:0".parse().expect("test must succeed")
        };
        let socket = UdpSocket::bind(local_addr)
            .await
            .map_err(|e| Error::Internal(format!("failed to bind socket: {}", e)))?;
        let local_addr = socket
            .local_addr()
            .map_err(|e| Error::Internal(format!("failed to get local address: {}", e)))?;

        // テスト用のため証明書検証なしの TLS コンテキスト
        let tls_ctx = TlsContext::new_client_with_options(&[b"h3"], false)?;

        let mut short_lengths = BTreeSet::new();
        short_lengths.insert(CLIENT_SCID_LEN);

        Ok(Self {
            socket,
            local_addr,
            server_addr,
            _tls_ctx: tls_ctx,
            conns: HashMap::new(),
            cid_map: HashMap::new(),
            short_lengths,
            recv_buf: vec![0u8; 65535],
            send_buf: vec![0u8; 1350],
        })
    }

    /// ローカルアドレスを取得する
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// 新しい QUIC 接続を追加する
    ///
    /// 戻り値は接続を識別するクライアント SCID。
    /// パケットは `pump` を呼ぶまで送信されない。
    pub fn add_connection(&mut self) -> Result<ConnectionId> {
        // dcid: クライアントが Initial の DCID に使う CID (サーバーの original_dcid になる)
        // scid: クライアントの SCID (サーバーが送るパケットの DCID になる)
        let dcid = ConnectionId::random(CLIENT_SCID_LEN)
            .ok_or(Error::Internal("failed to generate dcid".to_string()))?;
        let scid = ConnectionId::random(CLIENT_SCID_LEN)
            .ok_or(Error::Internal("failed to generate scid".to_string()))?;

        let tls_session = self._tls_ctx.create_session()?;

        let params = TransportParams::new().into_raw();
        let h3_settings = Http3Settings::new().into_raw();
        let ts = timestamp();

        let conn = Connection::client_new(
            &dcid,
            &scid,
            self.local_addr,
            self.server_addr,
            "localhost",
            tls_session,
            &params,
            ts,
        )?;
        let h3_conn = Http3Connection::client_new(&h3_settings)?;

        self.cid_map.insert(scid.clone(), scid.clone());
        self.conns.insert(
            scid.clone(),
            TestConn {
                conn,
                h3_conn,
                control_streams_bound: false,
            },
        );

        Ok(scid)
    }

    /// ハンドシェイクが完了するまでポンプを回す
    pub async fn handshake(&mut self, key: &ConnectionId, timeout: Duration) -> Result<()> {
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            if self
                .conns
                .get(key)
                .is_some_and(|conn| conn.conn.is_handshake_completed())
            {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(Error::Internal("handshake timeout".to_string()));
            }
            self.pump(Duration::from_millis(20)).await?;
        }
    }

    /// 全接続の送信データをフラッシュし、受信を 1 回処理する
    pub async fn pump(&mut self, timeout: Duration) -> Result<()> {
        let ts = timestamp();
        self.flush_all(ts).await?;

        // 受信を 1 回処理 (タイムアウトまで待つ)
        match tokio::time::timeout(timeout, self.socket.recv_from(&mut self.recv_buf)).await {
            Ok(Ok((len, from))) => {
                if from == self.server_addr {
                    let data = self.recv_buf[..len].to_vec();
                    let ts = timestamp();
                    self.dispatch(&data, ts);
                }
            }
            Ok(Err(e)) => {
                return Err(Error::Internal(format!("recv error: {}", e)));
            }
            Err(_) => {
                // タイムアウト: 受信なし
            }
        }

        // 期限切れタイマーを処理する
        // closing / draining 期間中の接続はタイマー処理を飛ばす
        // (テストクライアントは閉じた接続の送信を継続する必要がない)
        let ts = timestamp();
        let keys: Vec<ConnectionId> = self.conns.keys().cloned().collect();
        for key in keys {
            let expired = self
                .conns
                .get(&key)
                .is_some_and(|conn| conn.conn.get_expiry() <= ts);
            if !expired {
                continue;
            }
            if let Some(conn) = self.conns.get_mut(&key)
                && !conn.conn.is_in_closing_period()
                && !conn.conn.is_in_draining_period()
            {
                conn.conn.handle_expiry(ts)?;
            }
        }

        // ハンドシェイク完了直後の接続にコントロールストリームをバインドする
        for conn in self.conns.values_mut() {
            if conn.conn.is_handshake_completed() && !conn.control_streams_bound {
                let ctrl_stream_id = conn.conn.open_uni_stream()?;
                conn.h3_conn.bind_control_stream(ctrl_stream_id)?;

                let qpack_enc_stream_id = conn.conn.open_uni_stream()?;
                let qpack_dec_stream_id = conn.conn.open_uni_stream()?;
                conn.h3_conn
                    .bind_qpack_streams(qpack_enc_stream_id, qpack_dec_stream_id)?;

                conn.control_streams_bound = true;
            }
        }

        Ok(())
    }

    /// HTTP/3 リクエストを送信する
    pub fn send_request(
        &mut self,
        key: &ConnectionId,
        method: &str,
        path: &str,
    ) -> Result<StreamId> {
        let conn = self
            .conns
            .get_mut(key)
            .ok_or(Error::Internal("connection not found".to_string()))?;

        let stream_id = conn.conn.open_bidi_stream()?;
        let headers = vec![
            Header::method(method),
            Header::scheme("https"),
            Header::authority("localhost"),
            Header::path(path),
        ];
        conn.h3_conn.submit_request(stream_id, &headers)?;

        Ok(stream_id)
    }

    /// レスポンス (ステータスとボディ) を受信する
    ///
    /// ヘッダー受信完了でボディが無い場合、または StreamEnd で完了とみなす。
    pub async fn recv_response(
        &mut self,
        key: &ConnectionId,
        timeout: Duration,
    ) -> Result<(u16, Vec<u8>)> {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut status: Option<u16> = None;
        let mut body = Vec::new();

        loop {
            while let Some(event) = self
                .conns
                .get_mut(key)
                .and_then(|conn| conn.h3_conn.poll_event())
            {
                match event {
                    Http3Event::Header { header, .. } => {
                        if header.name == b":status"
                            && let Some(value) = header.value_str()
                            && let Ok(code) = value.parse::<u16>()
                        {
                            status = Some(code);
                        }
                    }
                    Http3Event::Data { data, .. } => {
                        body.extend_from_slice(&data);
                    }
                    Http3Event::HeadersEnd { fin: true, .. } => {
                        return Ok((status.unwrap_or(0), body));
                    }
                    Http3Event::StreamEnd { .. } => {
                        return Ok((status.unwrap_or(0), body));
                    }
                    _ => {}
                }
            }

            if tokio::time::Instant::now() >= deadline {
                return Err(Error::Internal("response timeout".to_string()));
            }
            self.pump(Duration::from_millis(20)).await?;
        }
    }

    /// QUIC 双方向ストリームを開く (HTTP/3 を介さない)
    pub fn open_stream(&mut self, key: &ConnectionId) -> Result<StreamId> {
        let conn = self
            .conns
            .get_mut(key)
            .ok_or(Error::Internal("connection not found".to_string()))?;
        conn.conn.open_bidi_stream()
    }

    /// QUIC ストリームに生データを書き込む
    ///
    /// HTTP/3 を介さず、不正なフレームなどをストリームに流すために使う。
    pub async fn send_raw_stream_data(
        &mut self,
        key: &ConnectionId,
        stream_id: StreamId,
        data: &[u8],
        fin: bool,
    ) -> Result<()> {
        let ts = timestamp();
        let (written, _) = {
            let conn = self
                .conns
                .get_mut(key)
                .ok_or(Error::Internal("connection not found".to_string()))?;
            conn.conn
                .write_stream(&mut self.send_buf, stream_id, data, fin, ts)?
        };

        if written > 0 {
            self.socket
                .send_to(&self.send_buf[..written], self.server_addr)
                .await
                .map_err(|e| Error::Internal(format!("send error: {}", e)))?;
        }

        Ok(())
    }

    /// QUIC の CONNECTION_CLOSE を送信する
    ///
    /// テストでサーバー側の終了状態 (Terminal) への移行経路を発火させるために使う。
    pub async fn send_connection_close(
        &mut self,
        key: &ConnectionId,
        error_code: u64,
    ) -> Result<()> {
        let ts = timestamp();
        let written = {
            let conn = self
                .conns
                .get_mut(key)
                .ok_or(Error::Internal("connection not found".to_string()))?;
            conn.conn
                .write_connection_close(&mut self.send_buf, error_code, b"", ts)?
        };

        if written > 0 {
            self.socket
                .send_to(&self.send_buf[..written], self.server_addr)
                .await
                .map_err(|e| Error::Internal(format!("send error: {}", e)))?;
        }

        Ok(())
    }

    /// 接続が閉じられた (closing / draining) かどうか
    ///
    /// サーバーが CONNECTION_CLOSE を送信したことをクライアント側から確認するために使う。
    pub fn is_connection_closed(&self, key: &ConnectionId) -> bool {
        self.conns.get(key).is_some_and(|conn| {
            conn.conn.is_in_closing_period() || conn.conn.is_in_draining_period()
        })
    }

    /// 全接続の送信データをフラッシュする
    async fn flush_all(&mut self, ts: u64) -> Result<()> {
        let keys: Vec<ConnectionId> = self.conns.keys().cloned().collect();

        for key in keys {
            // HTTP/3 ストリームデータを書き込んで送信する
            {
                let Some(conn) = self.conns.get_mut(&key) else {
                    continue;
                };
                if conn.conn.is_handshake_completed()
                    && conn.control_streams_bound
                    && !conn.conn.is_in_draining_period()
                {
                    let packets = write_h3_streams(conn, &mut self.send_buf, ts)?;
                    for pkt in packets {
                        self.socket
                            .send_to(&pkt, self.server_addr)
                            .await
                            .map_err(|e| Error::Internal(format!("send error: {}", e)))?;
                    }
                }
            }

            // QUIC パケットを送信する
            // draining 期間中の接続はパケットを送信しない
            loop {
                let (written, _) = match self.conns.get_mut(&key) {
                    Some(conn) if !conn.conn.is_in_draining_period() => {
                        conn.conn.write_pkt(&mut self.send_buf, ts)?
                    }
                    Some(_) => break,
                    None => break,
                };
                if written == 0 {
                    break;
                }
                self.socket
                    .send_to(&self.send_buf[..written], self.server_addr)
                    .await
                    .map_err(|e| Error::Internal(format!("send error: {}", e)))?;
            }

            // 発行済み CID をルーティングテーブルに登録する
            // 発行は write_pkt の中で行われるため、送信後に回収する
            if let Some(conn) = self.conns.get_mut(&key) {
                for cid in conn.conn.poll_issued_cids() {
                    self.short_lengths.insert(cid.len());
                    self.cid_map.insert(cid, key.clone());
                }
            }
        }

        Ok(())
    }

    /// 受信パケットを DCID で接続に振り分けて処理する
    fn dispatch(&mut self, data: &[u8], ts: u64) {
        let Some(key) = self.resolve_key(data) else {
            return;
        };
        let Some(conn) = self.conns.get_mut(&key) else {
            return;
        };

        let pkt_info = PacketInfo::default();
        // テストクライアントはパケット単位のエラーで停止しない
        if conn
            .conn
            .read_pkt(&self.local_addr, &self.server_addr, &pkt_info, data, ts)
            .is_err()
        {
            return;
        }

        // 受信したストリームデータを HTTP/3 に渡す
        while let Some(stream_data) = conn.conn.poll_stream_data() {
            let Ok(consumed) = conn.h3_conn.read_stream(
                stream_data.stream_id,
                &stream_data.data,
                stream_data.fin,
                ts,
            ) else {
                break;
            };
            if consumed > 0 {
                if conn
                    .conn
                    .extend_max_stream_offset(stream_data.stream_id, consumed as u64)
                    .is_err()
                {
                    break;
                }
                conn.conn.extend_max_offset(consumed as u64);
            }
        }
    }

    /// 受信パケットの DCID からクライアント SCID を解決する
    fn resolve_key(&self, data: &[u8]) -> Option<ConnectionId> {
        if data.is_empty() {
            return None;
        }

        if data[0] & 0x80 != 0 {
            // Long header (サーバーの Initial / Handshake): DCID 長はヘッダーに含まれる
            if data.len() < 6 {
                return None;
            }
            let dcid_len = data[5] as usize;
            if data.len() < 6 + dcid_len {
                return None;
            }
            let dcid = ConnectionId::new(&data[6..6 + dcid_len])?;
            return self.cid_map.get(&dcid).cloned();
        }

        // Short header: クライアントが発行した CID の長さで照合する
        for len in self.short_lengths.iter().rev() {
            let len = *len;
            if data.len() < 1 + len {
                continue;
            }
            if let Some(dcid) = ConnectionId::new(&data[1..1 + len])
                && let Some(key) = self.cid_map.get(&dcid)
            {
                return Some(key.clone());
            }
        }

        None
    }
}

/// HTTP/3 ストリームデータを書き込み、パケットを収集する
///
/// `Client::flush` の実装と同様に、nghttp3 の書き込みデータを QUIC ストリームに
/// 渡してパケットを収集する。
fn write_h3_streams(conn: &mut TestConn, send_buf: &mut [u8], ts: u64) -> Result<Vec<Vec<u8>>> {
    use nghttp3_sys::nghttp3_vec;

    let mut packets = Vec::new();

    let mut vecs = [nghttp3_vec {
        base: std::ptr::null_mut(),
        len: 0,
    }; 16];

    while let Ok((stream_id, fin, count)) = conn.h3_conn.write_stream(&mut vecs) {
        if count == 0 && !fin {
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

        if h3_data.is_empty() && !fin {
            continue;
        }

        match conn
            .conn
            .write_stream(send_buf, stream_id, &h3_data, fin, ts)
        {
            Ok((pkt_written, data_written)) => {
                if pkt_written > 0 {
                    packets.push(send_buf[..pkt_written].to_vec());
                }
                if let Some(dw) = data_written
                    && (dw > 0 || fin)
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
            Err(e) => return Err(e),
        }
    }

    Ok(packets)
}
