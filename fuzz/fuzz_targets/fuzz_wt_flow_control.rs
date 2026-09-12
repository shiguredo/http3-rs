#![no_main]

//! WebTransport フロー制御の状態機械を fuzz する
//!
//! 実接続 (`ServerConnection`) 上に WebTransport セッションを確立し、
//! ピアの広告カプセルと送信試行を任意順に与えて panic しないことを検証する。
//! 旧 `webtransport::Session` を対象にしていたものを、本番経路である
//! `Connection` の公開 API に対する fuzz に置き換えた。
//! (draft-ietf-webtrans-http3-16 Section 5.6)

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use shiguredo_http3::qpack::Encoder;
use shiguredo_http3::webtransport::{Capsule, Settings as WtSettings};
use shiguredo_http3::{ClientConnection, Header, ServerConnection, Settings, VarInt};

/// セッション確立後に適用する操作
#[derive(Debug, Arbitrary)]
enum FlowControlOp {
    /// ピアの WT_MAX_DATA を広告する
    MaxData { maximum: u64 },
    /// ピアの WT_MAX_STREAMS を広告する
    MaxStreams {
        bidirectional: bool,
        maximum: u64,
    },
    /// データ送信を試行して計上する
    SendData { bytes: u64 },
    /// ストリーム開設を試行して計上する
    OpenStream { bidirectional: bool },
    /// 受信データの消費を通知する
    ConsumeData { bytes: u64 },
    /// 送信待ちカプセルを取り出す
    TakeCapsules,
    /// 任意の生カプセルバイト列を CONNECT ストリームへ流す
    RawCapsule { data: Vec<u8> },
    /// 送信可能かどうかを問い合わせる (状態を変えない)
    Query { bytes: u64 },
}

fn vi(value: u64) -> VarInt {
    VarInt::new(value).expect("2^62-1 以下の値は VarInt として有効")
}

/// QPACK エンコードされた HEADERS フレームを構築する
fn build_headers_frame(headers: &[Header]) -> Vec<u8> {
    let mut encoder = Encoder::new();
    let mut qpack_buf = vec![0u8; 4096];
    let qpack_len = encoder
        .encode(&mut qpack_buf, headers, 0)
        .expect("fuzz 用ヘッダーのエンコードは成功する");
    qpack_buf.truncate(qpack_len);

    let mut frame = Vec::new();
    shiguredo_http3::varint::encode_into_vec(&mut frame, VarInt::from_static(0x01));
    shiguredo_http3::varint::encode_into_vec(
        &mut frame,
        VarInt::new(qpack_len as u64).expect("QPACK 長は VarInt 範囲内"),
    );
    frame.extend_from_slice(&qpack_buf);
    frame
}

/// WebTransport セッションを確立済みのサーバーを構築する
///
/// クライアント SETTINGS と CONNECT を流し込み、200 OK を返すところまで進める。
fn established_server() -> ServerConnection {
    let client_wt = WtSettings::new()
        .webtransport_max_sessions_draft07(vi(100))
        .wt_initial_max_streams_uni(vi(100))
        .wt_initial_max_streams_bidi(vi(100))
        .wt_initial_max_data(vi(8 * 1024 * 1024));
    let client_settings = Settings::new().enable_webtransport_client(client_wt);
    let mut client = ClientConnection::new(client_settings);
    client
        .set_control_stream_id(2)
        .expect("制御ストリーム ID の設定は成功する");
    let (client_ctrl, _) = client
        .take_stream_data(2)
        .expect("制御ストリームの初期データが取れる");

    let server_wt = WtSettings::new()
        .wt_enabled(vi(1))
        .webtransport_max_sessions_draft07(vi(100))
        .wt_initial_max_streams_uni(vi(100))
        .wt_initial_max_streams_bidi(vi(100))
        .wt_initial_max_data(vi(8 * 1024 * 1024));
    let server_settings = Settings::new().enable_webtransport_server(server_wt);
    let mut server = ServerConnection::new(server_settings);
    server
        .set_control_stream_id(3)
        .expect("制御ストリーム ID の設定は成功する");
    server
        .set_webtransport_transport_verified(true, true)
        .expect("transport parameter の注入は成功する");
    let _ = server.feed_stream(2, &client_ctrl, false);
    let _ = server.drain_events();

    let headers = vec![
        Header::new(b":method", b"CONNECT").expect("有効なヘッダー"),
        Header::new(b":protocol", b"webtransport").expect("有効なヘッダー"),
        Header::new(b":scheme", b"https").expect("有効なヘッダー"),
        Header::new(b":authority", b"example.com").expect("有効なヘッダー"),
        Header::new(b":path", b"/wt").expect("有効なヘッダー"),
    ];
    let frame = build_headers_frame(&headers);
    let _ = server.feed_stream(0, &frame, false);
    let _ = server.drain_events();
    let response = vec![Header::new(b":status", b"200").expect("有効なヘッダー")];
    let _ = server.send_response(0, &response, false);
    let _ = server.drain_events();
    server
}

fuzz_target!(|ops: Vec<FlowControlOp>| {
    // 操作列を上限付きで実行する (1 入力あたりの実行時間を抑える)
    let ops = &ops[..ops.len().min(128)];
    if ops.is_empty() {
        return;
    }

    let mut server = established_server();
    let stream_id = 0u64;
    // 確立直後の初期カプセルは消費しておく
    let _ = server.take_wt_flow_control_capsules(stream_id);

    for op in ops {
        match op {
            FlowControlOp::MaxData { maximum } => {
                let Ok(maximum) = VarInt::new(*maximum) else {
                    continue;
                };
                let capsule = Capsule::MaxData {
                    maximum: maximum.get(),
                };
                let mut data = Vec::new();
                capsule.encode_as_data_frame(&mut data)
                    .expect("テスト用カプセルのエンコードは成功する");
                let _ = server.feed_stream(stream_id, &data, false);
                let _ = server.drain_events();
            }
            FlowControlOp::MaxStreams {
                bidirectional,
                maximum,
            } => {
                let Ok(maximum) = VarInt::new(*maximum) else {
                    continue;
                };
                let capsule = Capsule::MaxStreams {
                    bidirectional: *bidirectional,
                    maximum: maximum.get(),
                };
                let mut data = Vec::new();
                capsule.encode_as_data_frame(&mut data)
                    .expect("テスト用カプセルのエンコードは成功する");
                let _ = server.feed_stream(stream_id, &data, false);
                let _ = server.drain_events();
            }
            FlowControlOp::SendData { bytes } => {
                let _ = server.wt_data_sent(stream_id, *bytes);
            }
            FlowControlOp::OpenStream { bidirectional } => {
                let _ = server.wt_stream_opened(stream_id, *bidirectional);
            }
            FlowControlOp::ConsumeData { bytes } => {
                server.wt_data_consumed(stream_id, *bytes);
            }
            FlowControlOp::TakeCapsules => {
                let _ = server.take_wt_flow_control_capsules(stream_id);
            }
            FlowControlOp::RawCapsule { data } => {
                let _ = server.feed_stream(stream_id, data, false);
                let _ = server.drain_events();
            }
            FlowControlOp::Query { bytes } => {
                let _ = server.can_send_wt_data(stream_id, *bytes);
                let _ = server.can_open_wt_bidi_stream(stream_id);
                let _ = server.can_open_wt_uni_stream(stream_id);
                let _ = server.wt_remote_max_data(stream_id);
                let _ = server.wt_session_flow_control_enabled(stream_id);
            }
        }
    }
});
