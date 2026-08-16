//! HTTP/3 サーバー実装

use std::collections::{BTreeSet, HashMap};
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use nghttp3_sys::nghttp3_settings;
use ngtcp2_sys::ngtcp2_transport_params;
use shiguredo_ngtcp2::{
    Connection, ConnectionErrorKind, ConnectionId, Error, Header, Http3Connection, Http3Event,
    Http3Settings, PacketInfo, QuicVersion, Result, StreamId, TlsContext, TransportParams,
};

use crate::conn::{
    SERVER_SCID_LEN, feed_stream_data_to_h3, parse_new_connection_packet, resolve_dcid,
    send_connection_close,
};
use crate::{Socket, timestamp};

/// HTTP/3 サーバー
pub struct Server {
    socket: Socket,
    local_addr: SocketAddr,
    // TLS コンテキスト
    tls_ctx: TlsContext,
    // トランスポートパラメータ (新規接続ごとに original_dcid を付与して使用)
    transport_params: ngtcp2_transport_params,
    // HTTP/3 設定
    h3_settings: nghttp3_settings,
    // 接続マップ (サーバー SCID -> 接続)
    connections: HashMap<ConnectionId, ServerConnection>,
    // DCID -> 接続キーのルーティングマップ (RFC 9000 Section 5.2)
    //
    // 1 つの接続は複数の CID を持つ (クライアント初回 Initial の DCID、
    // サーバーが発行した SCID、NEW_CONNECTION_ID で発行した CID)。
    // 到着パケットは DCID で接続に振り分ける。
    cid_map: HashMap<ConnectionId, ConnectionId>,
    // Short header パケットの DCID 照合に使う長さの集合
    //
    // Short header は DCID 長を運ばないため (RFC 9000 Section 17.3)、
    // サーバーが発行した CID の長さで照合する。
    short_cid_lengths: BTreeSet<usize>,
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
    // クライアントのアドレス
    remote_addr: SocketAddr,
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

        // サーバーが発行する CID はすべて SERVER_SCID_LEN の長さ
        let mut short_cid_lengths = BTreeSet::new();
        short_cid_lengths.insert(SERVER_SCID_LEN);

        Ok(Self {
            socket,
            local_addr,
            tls_ctx,
            transport_params,
            h3_settings,
            connections: HashMap::new(),
            cid_map: HashMap::new(),
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
    /// * `handler` - リクエストハンドラ
    ///   - 引数: (クライアントアドレス, HTTP/3 イベント)
    ///   - 戻り値: レスポンスヘッダーとボディ (None で応答しない)
    ///
    /// # エラー処理
    ///
    /// パケット処理・ストリーム処理のエラーは接続単位で処理され、サーバーループは
    /// 継続する。そのためこのメソッドはエラーを返さない (後方互換のため戻り値型は
    /// Result のまま)。
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
                            self.handle_recv(&data, from, &mut handler).await;
                        }
                        Err(e) => {
                            eprintln!("[tokio-ngtcp2 server] recv error: {}", e);
                            continue;
                        }
                    }
                }

                // タイムアウト
                _ = tokio::time::sleep(timer_duration) => {
                    self.handle_timeouts().await;
                }
            }

            // 送信データをフラッシュ
            self.flush_all().await;

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
    ///
    /// 到着パケットを DCID で既存・新規の接続に振り分ける (RFC 9000 Section 5.2)。
    /// エラーは接続単位で処理し、サーバーループは継続する。
    async fn handle_recv<F>(&mut self, data: &[u8], from: SocketAddr, handler: &mut F)
    where
        F: FnMut(SocketAddr, Http3Event) -> Option<(Vec<Header>, Vec<u8>)>,
    {
        // DCID で既存接続を検索
        if let Some(conn_key) = resolve_dcid(&self.cid_map, &self.short_cid_lengths, data) {
            self.handle_existing_connection(&conn_key, data, from, handler)
                .await;
            return;
        }

        // DCID 未登録のパケット: Long header なら新規接続、Short header なら破棄する
        // (Short header は必ずサーバーが発行した CID を DCID に持つため、
        // 未登録の DCID は未知の接続へのパケットである)
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
        F: FnMut(SocketAddr, Http3Event) -> Option<(Vec<Header>, Vec<u8>)>,
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

        // ハンドシェイク完了後、コントロールストリームをバインド
        let bind_result = match self.connections.get_mut(conn_key) {
            Some(conn) if conn.conn.is_handshake_completed() && !conn.control_streams_bound => {
                bind_server_control_streams(conn)
            }
            _ => Ok(()),
        };
        if let Err(e) = bind_result {
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

        // HTTP/3 イベントを処理
        loop {
            let event = match self.connections.get_mut(conn_key) {
                Some(conn) => conn.h3_conn.poll_event(),
                None => return,
            };
            let Some(event) = event else {
                break;
            };

            let stream_id = match &event {
                Http3Event::HeadersEnd { stream_id, .. } => Some(*stream_id),
                _ => None,
            };

            if let Some((headers, body)) = handler(from, event)
                && let Some(sid) = stream_id
            {
                let submit_result = match self.connections.get_mut(conn_key) {
                    Some(conn) => {
                        if body.is_empty() {
                            conn.h3_conn.submit_response(sid, &headers)
                        } else {
                            conn.h3_conn.submit_response_with_body(sid, &headers, body)
                        }
                    }
                    None => return,
                };
                if let Err(e) = submit_result {
                    self.handle_conn_error(conn_key, e).await;
                    return;
                }
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
                eprintln!("[tokio-ngtcp2 server] failed to generate scid");
                return;
            }
        };

        let ts = timestamp();

        // TLS セッションを作成
        let tls_session = match self.tls_ctx.create_session() {
            Ok(session) => session,
            Err(e) => {
                eprintln!("[tokio-ngtcp2 server] failed to create TLS session: {}", e);
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
            // bind 時に渡されたトランスポートパラメータを基に、
            // original_dcid (クライアントからの最初の Initial パケットの DCID) を付与する
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
                    eprintln!("[tokio-ngtcp2 server] failed to create connection: {}", e);
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

        // HTTP/3 接続を作成
        let h3_conn = match Http3Connection::server_new(&self.h3_settings) {
            Ok(h3_conn) => h3_conn,
            Err(e) => {
                eprintln!(
                    "[tokio-ngtcp2 server] failed to create HTTP/3 connection: {}",
                    e
                );
                return;
            }
        };

        // ルーティングテーブルに登録する
        // - original_dcid: クライアントが Initial の再送に使う DCID
        // - server_scid: ハンドシェイク後にクライアントが DCID に使う CID
        self.cid_map
            .insert(info.original_dcid.clone(), server_scid.clone());
        self.cid_map
            .insert(server_scid.clone(), server_scid.clone());
        self.connections.insert(
            server_scid,
            ServerConnection {
                conn,
                h3_conn,
                control_streams_bound: false,
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
                // 接続を黙って破棄する。
                // closing / draining 状態にならないため、明示的に除去する
                // (除去しないと compute_timer_duration が 1ms ビジーループになる)
                self.remove_connection(conn_key);
            }
            ConnectionErrorKind::Terminal => {
                // closing / draining 状態に移行済み。remove_closed_connections が除去する
            }
            ConnectionErrorKind::TransportClose | ConnectionErrorKind::ApplicationClose => {
                eprintln!("[tokio-ngtcp2 server] closing connection: {}", err);
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
                // 内部エラー。プロトコル違反ではないため CONNECTION_CLOSE は送らず、
                // 接続を破棄してサーバーは継続する
                eprintln!("[tokio-ngtcp2 server] connection error: {}", err);
                self.remove_connection(conn_key);
            }
        }
    }

    /// 全接続のタイムアウトを処理
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

    /// 全接続の送信データをフラッシュ
    async fn flush_all(&mut self) {
        let ts = timestamp();

        let keys: Vec<ConnectionId> = self.connections.keys().cloned().collect();
        for conn_key in keys {
            let Some(remote) = self.connections.get(&conn_key).map(|c| c.remote_addr) else {
                continue;
            };

            // HTTP/3 ストリームデータを一つずつ書き込み、即座に送信
            if let Err(e) = self.write_and_send_h3_streams(&conn_key, remote, ts).await {
                self.handle_conn_error(&conn_key, e).await;
                continue;
            }

            // 残りの QUIC パケットを送信
            while let Some(conn) = self.connections.get_mut(&conn_key) {
                match conn.conn.write_pkt(&mut self.send_buf, ts) {
                    Ok((0, _)) => break,
                    Ok((written, _)) => {
                        if let Err(e) = self.socket.send_to(&self.send_buf[..written], remote).await
                        {
                            eprintln!("[tokio-ngtcp2 server] send error: {}", e);
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

    /// HTTP/3 ストリームデータを書き込み、即座に送信
    ///
    /// ngtcp2 examples に従い、NGTCP2_WRITE_STREAM_FLAG_MORE を使用して
    /// 複数のストリームデータを 1 つのパケットにまとめる。
    async fn write_and_send_h3_streams(
        &mut self,
        conn_key: &ConnectionId,
        remote: SocketAddr,
        ts: u64,
    ) -> Result<()> {
        use nghttp3_sys::nghttp3_vec;

        let ready = match self.connections.get(conn_key) {
            Some(conn) => conn.conn.is_handshake_completed() && conn.control_streams_bound,
            None => return Ok(()),
        };
        if !ready {
            return Ok(());
        }

        let mut vecs = [nghttp3_vec {
            base: std::ptr::null_mut(),
            len: 0,
        }; 16];

        loop {
            // HTTP/3 から書き込むべきデータを取得
            let (stream_id, fin, count) = match self.connections.get_mut(conn_key) {
                Some(conn) => match conn.h3_conn.write_stream(&mut vecs) {
                    Ok(output) => output,
                    Err(e) => return Err(e),
                },
                None => return Ok(()),
            };
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

            // データが空でも FIN が立っている場合は送信する必要がある
            if h3_data.is_empty() && !fin {
                continue;
            }

            // QUIC ストリームに書き込み
            // ngtcp2 examples に従い、特定のエラーをハンドリング
            let result = {
                let conn = match self.connections.get_mut(conn_key) {
                    Some(conn) => conn,
                    None => return Ok(()),
                };
                conn.conn
                    .write_stream(&mut self.send_buf, stream_id, &h3_data, fin, ts)
            };

            match result {
                Ok((pkt_written, data_written)) => {
                    // パケットを即座に送信
                    if pkt_written > 0
                        && let Err(e) = self
                            .socket
                            .send_to(&self.send_buf[..pkt_written], remote)
                            .await
                    {
                        return Err(Error::Internal(format!("send error: {}", e)));
                    }

                    // nghttp3 に書き込んだ量を通知
                    if let Some(dw) = data_written
                        && (dw > 0 || fin)
                    {
                        let conn = match self.connections.get_mut(conn_key) {
                            Some(conn) => conn,
                            None => return Ok(()),
                        };
                        conn.h3_conn.add_write_offset(stream_id, dw)?;
                    }
                }
                Err(Error::StreamDataBlocked(_)) => {
                    // ngtcp2 examples: nghttp3_conn_block_stream を呼び出して続行
                    let conn = match self.connections.get_mut(conn_key) {
                        Some(conn) => conn,
                        None => return Ok(()),
                    };
                    conn.h3_conn.block_stream(stream_id);
                    continue;
                }
                Err(Error::StreamShutWr(_)) => {
                    // ngtcp2 examples: nghttp3_conn_shutdown_stream_write を呼び出して続行
                    let conn = match self.connections.get_mut(conn_key) {
                        Some(conn) => conn,
                        None => return Ok(()),
                    };
                    conn.h3_conn.shutdown_stream_write(stream_id);
                    continue;
                }
                Err(e) => return Err(e),
            }
        }

        Ok(())
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

    /// 全接続のコネクション ID 一覧を取得
    ///
    /// `send_response_by_conn_id` で接続を指定するために使用する。
    pub fn get_conn_ids(&self) -> Vec<ConnectionId> {
        self.connections.keys().cloned().collect()
    }

    /// レスポンスを送信 (クライアントアドレス指定)
    ///
    /// 旧 API。同一アドレスから複数の接続が張られている場合は接続を一意に
    /// 特定できないためエラーを返す。複数接続を扱う場合は
    /// `send_response_by_conn_id` を使うこと。
    pub fn send_response(
        &mut self,
        client_addr: SocketAddr,
        stream_id: StreamId,
        headers: &[Header],
    ) -> Result<()> {
        let mut keys = self
            .connections
            .iter()
            .filter(|(_, conn)| conn.remote_addr == client_addr)
            .map(|(key, _)| key.clone());
        let Some(conn_key) = keys.next() else {
            return Err(Error::Internal(format!(
                "connection not found: {}",
                client_addr
            )));
        };
        if keys.next().is_some() {
            return Err(Error::Internal(
                "multiple connections from the same address; use send_response_by_conn_id"
                    .to_string(),
            ));
        }

        self.submit_response_for(&conn_key, stream_id, headers)
    }

    /// レスポンスを送信 (コネクション ID 指定)
    ///
    /// 同一アドレスから複数の接続が張られている場合も、コネクション ID で
    /// 接続を一意に特定してレスポンスを送信できる。
    pub fn send_response_by_conn_id(
        &mut self,
        conn_id: &ConnectionId,
        stream_id: StreamId,
        headers: &[Header],
    ) -> Result<()> {
        self.submit_response_for(conn_id, stream_id, headers)
    }

    /// 指定した接続にレスポンスを送信する
    fn submit_response_for(
        &mut self,
        conn_key: &ConnectionId,
        stream_id: StreamId,
        headers: &[Header],
    ) -> Result<()> {
        let conn = self
            .connections
            .get_mut(conn_key)
            .ok_or(Error::Internal(format!(
                "connection not found: {}",
                conn_key
            )))?;

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
