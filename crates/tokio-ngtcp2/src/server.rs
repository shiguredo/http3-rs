//! HTTP/3 サーバー実装

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use nghttp3_sys::nghttp3_settings;
use ngtcp2_sys::ngtcp2_transport_params;
use shiguredo_ngtcp2::{
    Connection, ConnectionId, Error, Header, Http3Connection, Http3Event, Http3Settings,
    PacketInfo, Result, StreamId, TlsContext, TransportParams,
};

use crate::{Socket, timestamp};

/// HTTP/3 サーバー
pub struct Server {
    socket: Socket,
    local_addr: SocketAddr,
    // TLS コンテキスト
    tls_ctx: TlsContext,
    // トランスポートパラメータ (将来の拡張用に保持)
    #[expect(dead_code)]
    transport_params: ngtcp2_transport_params,
    // HTTP/3 設定
    h3_settings: nghttp3_settings,
    // 接続マップ (クライアントアドレス -> 接続)
    connections: HashMap<SocketAddr, ServerConnection>,
    // 受信バッファ
    recv_buf: Vec<u8>,
    // 送信バッファ
    send_buf: Vec<u8>,
}

/// サーバー側の接続
struct ServerConnection {
    // QUIC 接続
    conn: Connection,
    // HTTP/3 接続
    h3_conn: Http3Connection,
    // コントロールストリームをバインド済みか
    control_streams_bound: bool,
}

// SAFETY: Server の全フィールドは Send/Sync を実装している
// (Connection, Http3Connection, TlsContext は unsafe impl Send/Sync 済み)
unsafe impl Send for Server {}
unsafe impl Sync for Server {}

impl Server {
    /// 新しいサーバーを作成
    ///
    /// # Arguments
    ///
    /// * `addr` - リッスンアドレス
    /// * `cert_path` - 証明書ファイルのパス
    /// * `key_path` - 秘密鍵ファイルのパス
    /// * `transport_params` - QUIC トランスポートパラメータ (None でデフォルト)
    /// * `h3_settings` - HTTP/3 設定 (None でデフォルト)
    pub async fn bind(
        addr: SocketAddr,
        cert_path: &Path,
        key_path: &Path,
        transport_params: Option<ngtcp2_transport_params>,
        h3_settings: Option<nghttp3_settings>,
    ) -> Result<Self> {
        let socket = Socket::bind(addr)
            .await
            .map_err(|e| Error::Internal(format!("failed to bind socket: {}", e)))?;

        let local_addr = socket.local_addr();

        // TLS コンテキストを作成
        let tls_ctx = TlsContext::new_server(cert_path, key_path, &[b"h3"])?;

        // トランスポートパラメータ
        let transport_params =
            transport_params.unwrap_or_else(|| TransportParams::new().into_raw());

        // HTTP/3 設定
        let h3_settings = h3_settings.unwrap_or_else(|| Http3Settings::new().into_raw());

        Ok(Self {
            socket,
            local_addr,
            tls_ctx,
            transport_params,
            h3_settings,
            connections: HashMap::new(),
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
    /// * `handler` - リクエストハンドラ
    ///   - 引数: (クライアントアドレス, HTTP/3 イベント)
    ///   - 戻り値: レスポンスヘッダーとボディ (None で応答しない)
    pub async fn run<F>(&mut self, mut handler: F) -> Result<()>
    where
        F: FnMut(SocketAddr, Http3Event) -> Option<(Vec<Header>, Vec<u8>)>,
    {
        loop {
            // 次のタイムアウトを計算
            let timer_duration = self.compute_timer_duration();

            tokio::select! {
                // 受信データを処理
                result = self.socket.recv_from(&mut self.recv_buf) => {
                    match result {
                        Ok((len, from)) => {
                            let data = self.recv_buf[..len].to_vec();
                            self.handle_recv(&data, from, &mut handler).await?;
                        }
                        Err(e) => {
                            eprintln!("[tokio-ngtcp2 server] recv error: {}", e);
                            continue;
                        }
                    }
                }

                // タイムアウト
                _ = tokio::time::sleep(timer_duration) => {
                    self.handle_timeouts().await?;
                }
            }

            // 送信データをフラッシュ
            self.flush_all().await?;

            // クローズした接続を削除
            self.remove_closed_connections();
        }
    }

    /// タイムアウト時間を計算
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

    /// 受信データを処理
    async fn handle_recv<F>(&mut self, data: &[u8], from: SocketAddr, handler: &mut F) -> Result<()>
    where
        F: FnMut(SocketAddr, Http3Event) -> Option<(Vec<Header>, Vec<u8>)>,
    {
        let ts = timestamp();
        let pkt_info = PacketInfo::default();

        // 既存の接続を検索
        if let Some(conn) = self.connections.get_mut(&from) {
            // QUIC パケットを処理
            conn.conn
                .read_pkt(&self.local_addr, &from, &pkt_info, data, ts)?;

            // ハンドシェイク完了後、コントロールストリームをバインド
            if conn.conn.is_handshake_completed() && !conn.control_streams_bound {
                bind_server_control_streams(conn)?;
            }

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

            // HTTP/3 イベントを処理
            while let Some(event) = conn.h3_conn.poll_event() {
                let stream_id = match &event {
                    Http3Event::HeadersEnd { stream_id, .. } => Some(*stream_id),
                    _ => None,
                };

                if let Some((headers, body)) = handler(from, event)
                    && let Some(sid) = stream_id
                {
                    eprintln!(
                        "[tokio-ngtcp2 server] submit_response: stream_id = {}, body.len = {}",
                        sid,
                        body.len()
                    );
                    if body.is_empty() {
                        conn.h3_conn.submit_response(sid, &headers)?;
                    } else {
                        conn.h3_conn
                            .submit_response_with_body(sid, &headers, body)?;
                    }
                    eprintln!("[tokio-ngtcp2 server] submit_response done");
                }
            }

            return Ok(());
        }

        // 新しい接続を作成
        // パケットヘッダーを解析して DCID を取得
        // (簡易実装: 最初の 17 バイトを DCID として使用)
        if data.len() < 17 {
            return Ok(());
        }

        // Long Header の場合の DCID 取得 (RFC 9000 Section 17.2)
        let first_byte = data[0];
        if first_byte & 0x80 == 0 {
            // Short Header: 新しい接続には使用しない
            return Ok(());
        }

        // QUIC バージョンを読み取る (bytes 1-4, ビッグエンディアン)
        if data.len() < 5 {
            return Ok(());
        }
        let _version = u32::from_be_bytes([data[1], data[2], data[3], data[4]]);
        // DCID Length (offset 5)
        if data.len() < 6 {
            return Ok(());
        }
        let dcid_len = data[5] as usize;
        if data.len() < 6 + dcid_len {
            return Ok(());
        }
        let original_dcid_bytes = &data[6..6 + dcid_len];
        let original_dcid = match ConnectionId::new(original_dcid_bytes) {
            Some(cid) => cid,
            None => return Ok(()),
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
            None => return Ok(()),
        };

        // サーバー側の SCID を生成
        let server_scid = ConnectionId::random(16)
            .ok_or(Error::Internal("failed to generate scid".to_string()))?;

        // TLS セッションを作成
        let tls_session = self.tls_ctx.create_session()?;

        // サーバー用のトランスポートパラメータを作成
        // original_dcid はクライアントからの最初の Initial パケットの DCID
        let params = TransportParams::new()
            .with_original_dcid(&original_dcid)
            .into_raw();

        // QUIC 接続を作成
        // server_new の引数:
        // - dcid: クライアントの SCID (サーバーがクライアントに送るパケットの DCID になる)
        // - scid: サーバーの SCID
        let mut conn = match Connection::server_new(
            &client_scid,
            &server_scid,
            self.local_addr,
            from,
            tls_session,
            &params,
            ts,
        ) {
            Ok(c) => c,
            Err(e) => return Err(e),
        };

        // 受信パケットを処理
        conn.read_pkt(&self.local_addr, &from, &pkt_info, data, ts)?;

        // HTTP/3 接続を作成
        let h3_conn = Http3Connection::server_new(&self.h3_settings)?;

        let server_conn = ServerConnection {
            conn,
            h3_conn,
            control_streams_bound: false,
        };

        self.connections.insert(from, server_conn);

        Ok(())
    }

    /// 全接続のタイムアウトを処理
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

    /// 全接続の送信データをフラッシュ
    async fn flush_all(&mut self) -> Result<()> {
        let ts = timestamp();

        let addrs: Vec<SocketAddr> = self.connections.keys().copied().collect();
        eprintln!(
            "[tokio-ngtcp2 server] flush_all: {} connections",
            addrs.len()
        );

        for addr in addrs {
            // 接続を一時的に取り出す
            let mut conn = match self.connections.remove(&addr) {
                Some(c) => c,
                None => continue,
            };

            eprintln!(
                "[tokio-ngtcp2 server] flush_all: control_streams_bound = {}",
                conn.control_streams_bound
            );

            // HTTP/3 ストリームデータを一つずつ書き込み、即座に送信
            self.write_and_send_h3_streams(&mut conn, addr, ts).await?;

            // 残りの QUIC パケットを送信
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

            // 接続を戻す
            self.connections.insert(addr, conn);
        }

        Ok(())
    }

    /// HTTP/3 ストリームデータを書き込み、即座に送信
    ///
    /// ngtcp2 examples に従い、NGTCP2_WRITE_STREAM_FLAG_MORE を使用して
    /// 複数のストリームデータを 1 つのパケットにまとめる。
    async fn write_and_send_h3_streams(
        &mut self,
        conn: &mut ServerConnection,
        addr: SocketAddr,
        ts: u64,
    ) -> Result<()> {
        use nghttp3_sys::nghttp3_vec;

        if !conn.conn.is_handshake_completed() || !conn.control_streams_bound {
            return Ok(());
        }

        let mut vecs = [nghttp3_vec {
            base: std::ptr::null_mut(),
            len: 0,
        }; 16];

        while let Ok((stream_id, fin, count)) = conn.h3_conn.write_stream(&mut vecs) {
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

            // データが空でも FIN が立っている場合は送信する必要がある
            if h3_data.is_empty() && !fin {
                continue;
            }

            // QUIC ストリームに書き込み
            // ngtcp2 examples に従い、特定のエラーをハンドリング
            let result = conn
                .conn
                .write_stream(&mut self.send_buf, stream_id, &h3_data, fin, ts);

            match result {
                Ok((pkt_written, data_written)) => {
                    // パケットを即座に送信
                    if pkt_written > 0 {
                        self.socket
                            .send_to(&self.send_buf[..pkt_written], addr)
                            .await
                            .map_err(|e| Error::Internal(format!("send error: {}", e)))?;
                    }

                    // nghttp3 に書き込んだ量を通知
                    if let Some(dw) = data_written
                        && dw > 0
                    {
                        conn.h3_conn.add_write_offset(stream_id, dw)?;
                    }
                }
                Err(Error::StreamDataBlocked(_)) => {
                    // ngtcp2 examples: nghttp3_conn_block_stream を呼び出して続行
                    conn.h3_conn.block_stream(stream_id);
                    continue;
                }
                Err(Error::StreamShutWr(_)) => {
                    // ngtcp2 examples: nghttp3_conn_shutdown_stream_write を呼び出して続行
                    conn.h3_conn.shutdown_stream_write(stream_id);
                    continue;
                }
                Err(e) => return Err(e),
            }
        }

        Ok(())
    }

    /// クローズした接続を削除
    fn remove_closed_connections(&mut self) {
        self.connections.retain(|_, conn| {
            !conn.conn.is_in_closing_period() && !conn.conn.is_in_draining_period()
        });
    }

    /// レスポンスを送信
    pub fn send_response(
        &mut self,
        client_addr: SocketAddr,
        stream_id: StreamId,
        headers: &[Header],
    ) -> Result<()> {
        let conn = self
            .connections
            .get_mut(&client_addr)
            .ok_or(Error::Internal("connection not found".to_string()))?;

        conn.h3_conn.submit_response(stream_id, headers)?;

        Ok(())
    }
}

/// サーバー側のコントロールストリームをバインド
fn bind_server_control_streams(conn: &mut ServerConnection) -> Result<()> {
    // コントロールストリーム
    let ctrl_stream_id = conn.conn.open_uni_stream()?;
    conn.h3_conn.bind_control_stream(ctrl_stream_id)?;

    // QPACK エンコーダストリーム
    let qpack_enc_stream_id = conn.conn.open_uni_stream()?;

    // QPACK デコーダストリーム
    let qpack_dec_stream_id = conn.conn.open_uni_stream()?;

    conn.h3_conn
        .bind_qpack_streams(qpack_enc_stream_id, qpack_dec_stream_id)?;

    conn.control_streams_bound = true;
    Ok(())
}
