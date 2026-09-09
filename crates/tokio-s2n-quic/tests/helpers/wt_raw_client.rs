//! 生の s2n-quic クライアントで WebTransport ハンドシェイクを行い、
//! CONNECT ストリームへ任意のバイト列を送れるテスト用クライアント
//!
//! `WtClient` は CONNECT ストリームを公開しないため、WT_CLOSE_SESSION 後の
//! 追加 DATA 注入など CONNECT ストリームを直接操作するテストで使用する。

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use s2n_quic::client::Connect;
use s2n_quic::stream::{ReceiveStream, SendStream};
use shiguredo_http3::webtransport::{ConnectRequest, DraftVersion};
use shiguredo_http3::{
    ClientConnection, Event, Limits, Settings, VarInt, WebTransportEvent, webtransport,
};
use tokio::task::JoinHandle;

/// テスト用の WebTransport 設定 (draft-15)
pub fn build_wt_settings() -> webtransport::Settings {
    let v =
        |value: u64| VarInt::new(value).expect("WT settings のバリューが VarInt 範囲内であること");
    webtransport::Settings::new()
        .wt_enabled(VarInt::from_static(1))
        .wt_initial_max_streams_bidi(v(100))
        .wt_initial_max_streams_uni(v(100))
        .wt_initial_max_data(v(1_048_576))
}

/// 生の WebTransport クライアント
pub struct RawWtClient {
    /// CONNECT ストリームの送信端
    pub send: SendStream,
    /// CONNECT ストリームの受信端
    pub recv: ReceiveStream,
    /// 接続ハンドル (生存保持用)
    _handle: s2n_quic::connection::Handle,
    /// 制御・QPACK ストリーム (生存保持用)
    _control: SendStream,
    _encoder: SendStream,
    _decoder: SendStream,
    /// sans-I/O 接続状態 (生存保持用)
    _h3: Arc<Mutex<ClientConnection>>,
    /// サーバーの単方向ストリーム受信タスク
    _uni_task: JoinHandle<()>,
}

impl RawWtClient {
    /// 接続して WebTransport の CONNECT リクエストを送る
    pub async fn connect(server_addr: SocketAddr, ca_cert_pem: &str) -> Self {
        let client = s2n_quic::Client::builder()
            .with_tls(ca_cert_pem)
            .expect("クライアント TLS の構築に成功すること")
            .with_io("0.0.0.0:0")
            .expect("クライアント IO の構築に成功すること")
            .start()
            .expect("クライアントの起動に成功すること");
        let mut connection = client
            .connect(Connect::new(server_addr).with_server_name("localhost"))
            .await
            .expect("接続に成功すること");

        let mut control = connection
            .open_send_stream()
            .await
            .expect("制御ストリームのオープンに成功すること");
        let mut encoder = connection
            .open_send_stream()
            .await
            .expect("エンコーダーストリームのオープンに成功すること");
        let mut decoder = connection
            .open_send_stream()
            .await
            .expect("デコーダーストリームのオープンに成功すること");

        let settings = Settings::from_limits(&Limits::default())
            .expect("Limits::default() は VarInt 範囲内")
            .enable_webtransport_client(build_wt_settings());
        let h3 = Arc::new(Mutex::new(ClientConnection::new(settings)));
        let init = h3
            .lock()
            .expect("mutex should not be poisoned")
            .init_h3_streams(control.id(), encoder.id(), decoder.id())
            .expect("H3 ストリームの初期化に成功すること");
        h3.lock()
            .expect("mutex should not be poisoned")
            .set_webtransport_transport_verified(true, true)
            .expect("WebTransport 前提条件の設定に成功すること");

        control
            .send(Bytes::from(init.control_data))
            .await
            .expect("制御ストリームの送信に成功すること");
        encoder
            .send(Bytes::from(init.encoder_data))
            .await
            .expect("エンコーダーストリームの送信に成功すること");
        decoder
            .send(Bytes::from(init.decoder_data))
            .await
            .expect("デコーダーストリームの送信に成功すること");

        let (mut handle, acceptor) = connection.split();
        let (_bidi, mut uni_acceptor) = acceptor.split();

        // サーバーの単方向ストリーム (制御 / QPACK) を sans-I/O 層へ流す
        let h3_for_uni = Arc::clone(&h3);
        let uni_task = tokio::spawn(async move {
            while let Ok(Some(mut recv)) = uni_acceptor.accept_receive_stream().await {
                let h3 = Arc::clone(&h3_for_uni);
                tokio::spawn(async move {
                    let stream_id = recv.id();
                    while let Ok(Some(data)) = recv.receive().await {
                        if h3
                            .lock()
                            .expect("mutex should not be poisoned")
                            .feed_stream(stream_id, &data, false)
                            .is_err()
                        {
                            return;
                        }
                    }
                    let _ = h3
                        .lock()
                        .expect("mutex should not be poisoned")
                        .feed_stream(stream_id, &[], true);
                });
            }
        });

        // サーバー SETTINGS の受信を待つ (WebTransport CONNECT の送信に必要)
        let settings_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            if h3
                .lock()
                .expect("mutex should not be poisoned")
                .peer_settings()
                .is_some()
            {
                break;
            }
            assert!(
                tokio::time::Instant::now() < settings_deadline,
                "サーバー SETTINGS の受信がタイムアウトした"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        let stream = handle
            .open_bidirectional_stream()
            .await
            .expect("CONNECT ストリームのオープンに成功すること");
        let connect_stream_id = stream.id();
        let (mut recv, mut send) = stream.split();
        h3.lock()
            .expect("mutex should not be poisoned")
            .feed_stream(connect_stream_id, &[], false)
            .expect("CONNECT ストリームの登録に成功すること");

        let request =
            ConnectRequest::new("https", "localhost", "/").draft_version(DraftVersion::Draft15);
        let headers = request
            .to_headers()
            .expect("CONNECT リクエストのヘッダー生成に成功すること");
        let stream_id = h3
            .lock()
            .expect("mutex should not be poisoned")
            .send_request(&headers, false)
            .expect("CONNECT リクエストのエンコードに成功すること");
        assert_eq!(
            stream_id, connect_stream_id,
            "CONNECT ストリーム ID が一致すること"
        );
        let mut data = Vec::new();
        while let Some((chunk, _fin)) = h3
            .lock()
            .expect("mutex should not be poisoned")
            .take_stream_data(stream_id)
        {
            data.extend_from_slice(&chunk);
        }
        send.send(Bytes::from(data))
            .await
            .expect("CONNECT リクエストの送信に成功すること");

        // サーバーの 200 レスポンスを処理してセッション確立を待つ。
        // 確立前に CONNECT ストリームへデータを注入すると、サーバーの
        // ハンドシェイク中に処理されてしまうため。
        loop {
            let established = h3
                .lock()
                .expect("mutex should not be poisoned")
                .drain_events()
                .expect("イベントドレインに成功すること")
                .iter()
                .any(|event| {
                    matches!(
                        event,
                        Event::WebTransport(WebTransportEvent::SessionEstablished { .. })
                    )
                });
            if established {
                break;
            }
            match tokio::time::timeout(Duration::from_secs(5), recv.receive()).await {
                Ok(Ok(Some(data))) => {
                    h3.lock()
                        .expect("mutex should not be poisoned")
                        .feed_stream(connect_stream_id, &data, false)
                        .expect("CONNECT レスポンスの feed に成功すること");
                }
                Ok(Ok(None)) => panic!("セッション確立前に FIN を受信した"),
                Ok(Err(e)) => panic!("セッション確立前にエラーを受信した: {e}"),
                Err(_) => panic!("セッション確立のタイムアウト"),
            }
        }

        Self {
            send,
            recv,
            _handle: handle,
            _control: control,
            _encoder: encoder,
            _decoder: decoder,
            _h3: h3,
            _uni_task: uni_task,
        }
    }
}
