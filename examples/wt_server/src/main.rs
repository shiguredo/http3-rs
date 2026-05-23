/// WebTransport サーバーサンプル
///
/// draft-ietf-webtrans-http3 の draft-02 / 07 / 14 / 15 に対応する。
/// クライアントの SETTINGS から draft バージョンを自動判定し、適切な応答を返す
/// (`shiguredo_http3::Settings::webtransport_draft_pattern()`)。
///
/// `--reject-connect` で全セッションを 404 拒否し、`WtSessionRequest::reject()` の動作を確認できる。
///
/// 接続後はエコーサーバーとして動作する:
/// - 双方向ストリーム: 受信データをそのまま返す
/// - 単方向ストリーム: 受信データをログに出力し、単方向ストリームで返す
///
/// 使い方:
///   cargo run -p wt_server -- --listen 127.0.0.1:4443
mod error;
mod tls;
mod webtransport;

use s2n_quic::Server;

use crate::error::Error;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = parse_args();

    if let Err(e) = run_server(&args.listen, args.reject_connect).await {
        tracing::error!("Server error: {e}");
    }
}

/// WebTransport サーバーを起動する
///
/// HTTP/3 over QUIC (ALPN: h3) で WebTransport セッションを受け付ける。
async fn run_server(listen: &str, reject_connect: bool) -> Result<(), Error> {
    tracing::info!("Loading TLS certificate...");
    let tls = tls::generate_tls_server()?;
    tracing::info!("TLS certificate ready");

    // WebTransport は QUIC DATAGRAM (RFC 9221) サポートを要求する
    let datagram = s2n_quic::provider::datagram::default::Endpoint::builder()
        .with_send_capacity(16)
        .map_err(|e| Error::Other(format!("datagram configuration failed: {e}")))?
        .with_recv_capacity(16)
        .map_err(|e| Error::Other(format!("datagram configuration failed: {e}")))?
        .build()
        .map_err(|e| Error::Other(format!("datagram configuration failed: {e}")))?;

    let mut server = Server::builder()
        .with_tls(tls)
        .map_err(|e| Error::Other(format!("TLS configuration failed: {e}")))?
        .with_io(listen)
        .map_err(|e| Error::Other(format!("I/O binding failed: {e}")))?
        .with_datagram(datagram)
        .map_err(|e| Error::Other(format!("datagram provider failed: {e}")))?
        .start()
        .map_err(|e| Error::Other(format!("server start failed: {e}")))?;

    tracing::info!("WebTransport server listening on {listen}");

    // WebTransport 用の HTTP/3 設定
    // draft-15: SETTINGS_WT_INITIAL_MAX_STREAMS で初期ストリーム上限を通知する
    // 動的な WT_MAX_STREAMS カプセルによる上限更新は未実装のため、初期値を大きく設定する
    let v = |value: u64| {
        shiguredo_http3::VarInt::new(value).expect("WT settings value must fit VarInt")
    };
    let wt_settings = shiguredo_http3::webtransport::Settings::new()
        .wt_enabled(shiguredo_http3::VarInt::from_static(1))
        .wt_initial_max_streams_uni(v(1000))
        .wt_initial_max_streams_bidi(v(1000))
        .enable_webtransport_draft02(true)
        .webtransport_max_sessions_draft07(v(100));
    let h3_settings = shiguredo_http3::Settings::default().enable_webtransport_server(wt_settings);

    loop {
        tokio::select! {
            result = server.accept() => {
                let Some(connection) = result else { break };
                let remote = connection
                    .remote_addr()
                    .map(|a| a.to_string())
                    .unwrap_or_else(|_| "unknown".to_string());

                // ALPN を確認する
                let alpn = match connection.application_protocol() {
                    Ok(alpn) => alpn,
                    Err(e) => {
                        tracing::warn!("[{remote}] Failed to get ALPN: {e}");
                        continue;
                    }
                };

                match &alpn[..] {
                    b"h3" => {
                        tracing::info!("[{remote}] New WebTransport connection");
                        let settings = h3_settings;
                        let reject = reject_connect;
                        tokio::spawn(async move {
                            match handle_connection(connection, settings, remote.clone(), reject)
                                .await
                            {
                                Ok(()) => tracing::info!("[{remote}] Connection closed"),
                                Err(e) => tracing::error!("[{remote}] Connection error: {e}"),
                            }
                        });
                    }
                    other => {
                        let alpn_str = String::from_utf8_lossy(other);
                        tracing::warn!("[{remote}] Unsupported ALPN: {alpn_str} (expected h3)");
                    }
                }
            }
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("Shutting down...");
                break;
            }
        }
    }

    Ok(())
}

/// WebTransport 接続を処理する
///
/// HTTP/3 ハンドシェイク後、WebTransport セッションを確立してエコー処理を行う。
async fn handle_connection(
    connection: s2n_quic::Connection,
    h3_settings: shiguredo_http3::Settings,
    remote: String,
    reject_connect: bool,
) -> Result<(), Error> {
    tracing::info!("[{remote}] Starting WebTransport handshake...");

    let request = webtransport::WtSessionRequest::from_connection(connection, h3_settings).await?;

    let path = request.path().to_string();
    let authority = request.authority().to_string();
    let draft = request.draft();
    tracing::info!(
        "[{remote}] WebTransport CONNECT: authority={authority}, path={path}, draft={draft:?}"
    );

    if reject_connect {
        tracing::warn!("[{remote}] Rejecting WebTransport session (404, --reject-connect demo)");
        request.reject(404).await?;
        return Ok(());
    }

    tracing::info!("[{remote}] Accepting WebTransport session...");
    let session = request.accept().await?;
    tracing::info!(
        "[{remote}] WebTransport session established (session_id={}, draft={:?})",
        session.session_id(),
        session.draft()
    );

    // セッションを分解して bidi と uni を独立に扱う
    // tokio::select! での二重可変借用を回避するため
    let parts = session.into_parts();
    let mut bidi_acceptor = parts.bidi_acceptor;
    let mut uni_rx = parts.uni_rx;
    let session_id = parts.session_id;
    let mut handle = parts.handle;

    // DATAGRAM エコータスク: 受信した DATAGRAM をそのまま送り返す
    let datagram_handle = handle.clone();
    let datagram_remote = remote.clone();
    let _datagram_task = tokio::spawn(async move {
        handle_datagram_echo(&datagram_handle, session_id, &datagram_remote).await;
    });
    // draft-14 以降: WT_MAX_STREAMS カプセルを送信してストリーム上限を通知する
    // Safari は draft-07 の SETTINGS で接続しつつ draft-14 のカプセルベースフロー制御を使うため、
    // セッション確立直後に WT_MAX_STREAMS を送る必要がある
    let mut connect_send = parts._connect_send;
    {
        use shiguredo_http3::webtransport::capsule::Capsule;

        // 各カプセルを個別の H3 DATA フレームで包む
        let capsules = [
            Capsule::MaxStreams {
                bidirectional: true,
                maximum: 100,
            },
            Capsule::MaxStreams {
                bidirectional: false,
                maximum: 100,
            },
            Capsule::MaxData {
                maximum: 8 * 1024 * 1024,
            },
        ];

        let mut buf = Vec::new();
        for capsule in &capsules {
            let mut capsule_bytes = Vec::new();
            capsule.encode(&mut capsule_bytes);

            // H3 DATA フレーム: type=0x00, length=varint, payload
            buf.push(0x00);
            let payload_len = shiguredo_http3::VarInt::new(capsule_bytes.len() as u64)
                .expect("capsule payload length fits in VarInt");
            let len_size = payload_len.encoded_len();
            let len_start = buf.len();
            buf.resize(len_start + len_size, 0);
            shiguredo_http3::varint::encode(&mut buf[len_start..], payload_len).unwrap();
            buf.extend_from_slice(&capsule_bytes);
        }

        tracing::info!(
            "[{remote}] Sending WT flow control capsules (max_streams_bidi=100, max_streams_uni=100, max_data=8MB): {:02x?}",
            buf
        );
        connect_send
            .send(bytes::Bytes::from(buf))
            .await
            .map_err(|e| Error::Other(format!("failed to send WT_MAX_STREAMS: {e}")))?;
    }

    // CONNECT ストリームの受信データを読んでログに出す
    let connect_remote = remote.clone();
    let _connect_recv_task = tokio::spawn(async move {
        let mut recv = parts._connect_recv;
        loop {
            match recv.receive().await {
                Ok(Some(data)) => {
                    tracing::info!(
                        "[{connect_remote}] CONNECT stream received {} bytes: {:02x?}",
                        data.len(),
                        data.as_ref()
                    );
                }
                Ok(None) => {
                    tracing::info!("[{connect_remote}] CONNECT stream closed (FIN)");
                    break;
                }
                Err(e) => {
                    tracing::info!("[{connect_remote}] CONNECT stream error: {e}");
                    break;
                }
            }
        }
    });

    // エコーサーバーとして双方向ストリームと単方向ストリームを並行処理する
    tracing::info!("[{remote}] Waiting for streams...");
    loop {
        tokio::select! {
            // 双方向ストリーム: 受信データをそのままエコーする
            result = bidi_acceptor.accept_bidirectional_stream() => {
                match result {
                    Ok(Some(stream)) => {
                        let stream_id: u64 = stream.id();
                        tracing::info!(
                            "[{remote}] Accepted bidi stream {} (0x{:x})",
                            stream_id,
                            stream_id
                        );

                        // WT bidi ストリームヘッダーを読む
                        let (mut recv, send) = stream.split();
                        let mut header_buf: Vec<u8> = Vec::new();
                        let pending = loop {
                            let data = match recv.receive().await {
                                Ok(Some(data)) => data,
                                Ok(None) => {
                                    tracing::warn!(
                                        "[{remote}] Bidi stream {stream_id}: closed before header"
                                    );
                                    break Vec::new();
                                }
                                Err(e) => {
                                    tracing::error!(
                                        "[{remote}] Bidi stream {stream_id}: header read error: {e}"
                                    );
                                    break Vec::new();
                                }
                            };
                            header_buf.extend_from_slice(&data);
                            match shiguredo_http3::webtransport::stream::StreamHeader::decode_bidirectional_checked(&header_buf) {
                                Ok((_, consumed)) => break header_buf[consumed..].to_vec(),
                                Err(shiguredo_http3::webtransport::stream::StreamHeaderDecodeError::BufferTooShort) => continue,
                                Err(e) => {
                                    tracing::error!(
                                        "[{remote}] Bidi stream {stream_id}: header decode error: {e:?}"
                                    );
                                    break Vec::new();
                                }
                            }
                        };
                        let wt_recv = webtransport::WtRecvStream::new(stream_id, recv, pending);
                        let wt_send = webtransport::WtSendStream::new(stream_id, send);
                        let bi_stream = webtransport::WtBiStream::from_parts(wt_send, wt_recv);

                        let r = remote.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handle_bidi_echo(bi_stream, &r).await {
                                tracing::error!(
                                    "[{r}] Bidi stream {stream_id} error: {e}"
                                );
                            }
                        });
                    }
                    Ok(None) => {
                        tracing::info!("[{remote}] Bidi stream acceptor closed");
                        break;
                    }
                    Err(e) => {
                        tracing::info!("[{remote}] Bidi stream accept error: {e}");
                        break;
                    }
                }
            }
            // 単方向ストリーム: 受信データをログに出力し、単方向ストリームで返す
            result = uni_rx.recv() => {
                match result {
                    Some(uni_recv) => {
                        let stream_id = uni_recv.stream_id();
                        tracing::info!(
                            "[{remote}] Accepted uni recv stream {} (0x{:x})",
                            stream_id,
                            stream_id
                        );
                        let r = remote.clone();
                        // 新しい単方向送信ストリームを開く
                        match handle.open_send_stream().await {
                            Ok(send) => {
                                let send_stream_id: u64 = send.id();
                                let mut send = send;
                                let mut header = Vec::new();
                                shiguredo_http3::webtransport::stream::StreamHeader::new(session_id)
                                    .expect("session_id must be a client-initiated bidirectional stream id")
                                    .encode_unidirectional(&mut header);
                                if let Err(e) = send.send(bytes::Bytes::from(header)).await {
                                    tracing::error!(
                                        "[{remote}] Failed to send WT header on uni stream: {e}"
                                    );
                                    break;
                                }
                                let uni_send = webtransport::WtSendStream::new(send_stream_id, send);
                                tracing::info!(
                                    "[{r}] Opened uni send stream {} (0x{:x}) for echo",
                                    send_stream_id,
                                    send_stream_id
                                );
                                tokio::spawn(async move {
                                    if let Err(e) = handle_uni_echo(uni_recv, uni_send, &r).await {
                                        tracing::error!(
                                            "[{r}] Uni stream {stream_id} error: {e}"
                                        );
                                    }
                                });
                            }
                            Err(e) => {
                                tracing::error!(
                                    "[{remote}] Failed to open uni send stream: {e}"
                                );
                                break;
                            }
                        }
                    }
                    None => {
                        tracing::info!("[{remote}] Uni stream channel closed");
                        break;
                    }
                }
            }
        }
    }

    tracing::info!("[{remote}] Session processing finished");
    Ok(())
}

/// 双方向ストリームのエコー処理
///
/// 受信したデータをそのまま送り返す。
async fn handle_bidi_echo(
    mut bi_stream: webtransport::WtBiStream,
    remote: &str,
) -> Result<(), Error> {
    let stream_id = bi_stream.stream_id();
    let mut total_bytes: u64 = 0;

    loop {
        match bi_stream.recv().await {
            Ok(data) => {
                total_bytes += data.len() as u64;
                tracing::info!(
                    "[{remote}] Bidi stream {stream_id}: received {} bytes (total: {total_bytes})",
                    data.len()
                );
                tracing::debug!(
                    "[{remote}] Bidi stream {stream_id}: data: {:02x?}",
                    &data[..std::cmp::min(data.len(), 64)]
                );
                bi_stream.send(&data).await.map_err(|e| {
                    tracing::error!("[{remote}] Bidi stream {stream_id}: send error: {e}");
                    e
                })?;
                tracing::debug!(
                    "[{remote}] Bidi stream {stream_id}: echoed {} bytes",
                    data.len()
                );
            }
            Err(webtransport::Error::StreamClosed) => {
                tracing::info!(
                    "[{remote}] Bidi stream {stream_id}: closed (total received: {total_bytes} bytes)"
                );
                let _ = bi_stream.finish();
                break;
            }
            Err(e) => {
                tracing::error!("[{remote}] Bidi stream {stream_id}: receive error: {e}");
                return Err(e.into());
            }
        }
    }
    Ok(())
}

/// 単方向ストリームのエコー処理
///
/// 受信ストリームのデータを読み取り、送信ストリームで返す。
async fn handle_uni_echo(
    mut uni_recv: webtransport::WtRecvStream,
    mut uni_send: webtransport::WtSendStream,
    remote: &str,
) -> Result<(), Error> {
    let recv_id = uni_recv.stream_id();
    let send_id = uni_send.stream_id();
    let mut total_bytes: u64 = 0;

    loop {
        match uni_recv.recv().await {
            Ok(data) => {
                total_bytes += data.len() as u64;
                tracing::info!(
                    "[{remote}] Uni stream {recv_id} -> {send_id}: received {} bytes (total: {total_bytes})",
                    data.len()
                );
                tracing::debug!(
                    "[{remote}] Uni stream {recv_id}: data: {:02x?}",
                    &data[..std::cmp::min(data.len(), 64)]
                );
                uni_send.send(&data).await.map_err(|e| {
                    tracing::error!("[{remote}] Uni stream {send_id}: send error: {e}");
                    e
                })?;
                tracing::debug!(
                    "[{remote}] Uni stream {recv_id} -> {send_id}: echoed {} bytes",
                    data.len()
                );
            }
            Err(webtransport::Error::StreamClosed) => {
                tracing::info!(
                    "[{remote}] Uni stream {recv_id}: closed (total received: {total_bytes} bytes)"
                );
                let _ = uni_send.finish();
                break;
            }
            Err(e) => {
                tracing::error!("[{remote}] Uni stream {recv_id}: receive error: {e}");
                return Err(e.into());
            }
        }
    }
    Ok(())
}

/// DATAGRAM エコー処理
///
/// QUIC DATAGRAM フレームを受信し、同じ内容をそのまま送り返す。
/// HTTP Datagram フォーマット (RFC 9297): Quarter Stream ID (varint) + Payload
async fn handle_datagram_echo(
    handle: &s2n_quic::connection::Handle,
    session_id: u64,
    remote: &str,
) {
    use s2n_quic::provider::datagram::default::{Receiver, Sender};

    let quarter_stream_id = session_id / 4;
    let mut total_datagrams: u64 = 0;

    loop {
        // 受信を非同期で待機する
        let datagram = std::future::poll_fn(|cx| {
            match handle.datagram_mut(|recv: &mut Receiver| recv.poll_recv_datagram(cx)) {
                Ok(poll) => poll.map(|r| r.ok()),
                Err(_) => std::task::Poll::Ready(None),
            }
        })
        .await;

        let Some(raw) = datagram else {
            tracing::info!("[{remote}] Datagram receiver closed");
            break;
        };

        // Quarter Stream ID をデコードして、このセッション宛かを確認する
        let Some((decoded, consumed)) = shiguredo_http3::webtransport::Datagram::decode(&raw)
        else {
            tracing::warn!(
                "[{remote}] Datagram decode failed ({} bytes): {:02x?}",
                raw.len(),
                &raw[..std::cmp::min(raw.len(), 32)]
            );
            continue;
        };

        if decoded.quarter_stream_id() != quarter_stream_id {
            tracing::debug!(
                "[{remote}] Datagram for different session (qsi={}, expected={})",
                decoded.quarter_stream_id(),
                quarter_stream_id
            );
            continue;
        }

        total_datagrams += 1;
        tracing::info!(
            "[{remote}] Datagram received: {} bytes payload (total: {total_datagrams}, consumed: {consumed})",
            decoded.payload.len()
        );
        tracing::debug!(
            "[{remote}] Datagram payload: {:02x?}",
            &decoded.payload[..std::cmp::min(decoded.payload.len(), 64)]
        );

        // エコー: 受信したデータグラムをそのまま送り返す
        // raw にはすでに Quarter Stream ID + Payload が含まれているのでそのまま使う
        let echo_data = raw.clone();
        match handle.datagram_mut(|sender: &mut Sender| sender.send_datagram(echo_data)) {
            Ok(Ok(())) => {
                tracing::debug!("[{remote}] Datagram echoed: {} bytes", raw.len());
            }
            Ok(Err(e)) => {
                tracing::warn!("[{remote}] Datagram send failed: {e}");
            }
            Err(e) => {
                tracing::error!("[{remote}] Datagram send error: {e}");
                break;
            }
        }
    }

    tracing::info!("[{remote}] Datagram echo finished (total: {total_datagrams})");
}

/// CLI 引数
struct Args {
    listen: String,
    /// `WtSessionRequest::reject()` のデモ用: 全 CONNECT を 404 で拒否する
    reject_connect: bool,
}

fn parse_args() -> Args {
    let mut args = noargs::raw_args();
    args.metadata_mut().app_name = "wt_server";
    args.metadata_mut().app_description = "WebTransport echo server";

    if noargs::VERSION_FLAG.take(&mut args).is_present() {
        println!("wt_server {}", env!("CARGO_PKG_VERSION"));
        std::process::exit(0);
    }
    noargs::HELP_FLAG.take_help(&mut args);

    let listen: String = noargs::opt("listen")
        .short('l')
        .ty("ADDR")
        .doc("Listen address")
        .default("127.0.0.1:4443")
        .take(&mut args)
        .then(|o| Ok::<_, std::convert::Infallible>(o.value().to_string()))
        .unwrap();

    let reject_connect: bool = noargs::flag("reject-connect")
        .doc("Reject every WebTransport CONNECT with 404 (for testing WtSessionRequest::reject)")
        .take(&mut args)
        .is_present();

    if let Ok(Some(help)) = args.finish() {
        print!("{help}");
        std::process::exit(0);
    }

    Args {
        listen,
        reject_connect,
    }
}
