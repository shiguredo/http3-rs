//! HTTP/3 クライアント接続

use crate::error::Error;
use crate::event::Event;
use crate::qpack::Header;
use crate::settings::Settings;
use crate::varint::VarInt;

use super::{Connection, H3InitData, Role};

/// HTTP/3 クライアント接続
///
/// クライアント専用の API を提供。
#[derive(Debug)]
pub struct ClientConnection {
    inner: Connection,
}

impl ClientConnection {
    /// 新しいクライアント接続を作成
    pub fn new(settings: Settings) -> Self {
        Self {
            inner: Connection::new(Role::Client, settings),
        }
    }

    /// デフォルト設定でクライアント接続を作成
    pub fn with_default_settings() -> Self {
        Self::new(Settings::default())
    }

    /// 制御ストリーム ID を設定
    ///
    /// 制御ストリームは 1 つのみ許可される (RFC 9114 Section 6.2.1)。
    pub fn set_control_stream_id(&mut self, stream_id: u64) -> Result<(), Error> {
        self.inner.set_control_stream_id(stream_id)
    }

    /// 制御ストリーム・QPACK encoder/decoder ストリームを一括で初期化する
    pub fn init_h3_streams(
        &mut self,
        control_stream_id: u64,
        encoder_stream_id: u64,
        decoder_stream_id: u64,
    ) -> Result<H3InitData, Error> {
        self.inner
            .init_h3_streams(control_stream_id, encoder_stream_id, decoder_stream_id)
    }

    /// QUIC からストリームデータを受信
    pub fn feed_stream(&mut self, stream_id: u64, data: &[u8], fin: bool) -> Result<(), Error> {
        self.inner.feed_stream(stream_id, data, fin)
    }

    /// イベントを取得
    pub fn poll_event(&mut self) -> Result<Option<Event>, Error> {
        self.inner.poll_event()
    }

    /// イベントキューの全イベントを取り出す
    ///
    /// キュー内の全イベントを `Vec` として返し、キューを空にする。
    pub fn drain_events(&mut self) -> Result<Vec<Event>, Error> {
        self.inner.drain_events()
    }

    /// 送信可能なストリームを取得
    pub fn writable_streams(&self) -> impl Iterator<Item = u64> + '_ {
        self.inner.writable_streams()
    }

    /// ストリームの送信データを取得
    pub fn get_stream_data(&mut self, stream_id: u64) -> Option<(&[u8], bool)> {
        self.inner.get_stream_data(stream_id)
    }

    /// ストリームデータを取得して内部バッファから消費する
    ///
    /// ストリームの送信バッファにある全データを 1 回の呼び出しで返す。
    /// ループで繰り返し呼ぶ必要はない。
    pub fn take_stream_data(&mut self, stream_id: u64) -> Option<(Vec<u8>, bool)> {
        self.inner.take_stream_data(stream_id)
    }

    /// ストリームの送信データを消費
    pub fn consume_stream_data(&mut self, stream_id: u64, len: usize) {
        self.inner.consume_stream_data(stream_id, len);
    }

    /// リクエストを送信
    ///
    /// 新しいストリームを作成し、ヘッダーを送信。
    /// ストリーム ID を返す。
    pub fn send_request(&mut self, headers: &[Header], fin: bool) -> Result<u64, Error> {
        self.inner.send_request(headers, fin)
    }

    /// ボディを送信
    pub fn send_body(&mut self, stream_id: u64, data: &[u8], fin: bool) -> Result<(), Error> {
        self.inner.send_body(stream_id, data, fin)
    }

    /// ピア設定を取得
    pub fn peer_settings(&self) -> Option<&Settings> {
        self.inner.peer_settings()
    }

    /// ローカル設定を取得
    pub fn local_settings(&self) -> &Settings {
        self.inner.local_settings()
    }

    /// GOAWAY を送信
    pub fn send_goaway(&mut self, id: VarInt) -> Result<(), Error> {
        self.inner.send_goaway(id)
    }

    /// ストリームをリセット (RESET_STREAM)
    pub fn stream_reset(
        &mut self,
        stream_id: u64,
        error_code: u64,
        final_size: u64,
    ) -> Result<(), Error> {
        self.inner.stream_reset(stream_id, error_code, final_size)
    }

    /// ストリームの送信停止を要求 (STOP_SENDING)
    pub fn stop_sending(&mut self, stream_id: u64, error_code: u64) -> Result<(), Error> {
        self.inner.stop_sending(stream_id, error_code)
    }

    /// QUIC DATAGRAM フレームのペイロードを受信
    ///
    /// (draft-ietf-webtrans-http3-15 Section 4.5)
    pub fn feed_datagram(&mut self, data: &[u8]) -> Result<(), Error> {
        self.inner.feed_datagram(data)
    }

    /// WebTransport データグラムを送信用にエンコードする
    ///
    /// (draft-ietf-webtrans-http3-15 Section 4.5)
    pub fn send_datagram(&self, session_id: u64, payload: &[u8]) -> Result<Vec<u8>, Error> {
        self.inner.send_datagram(session_id, payload)
    }

    /// QUIC transport parameter に基づく WebTransport 前提条件を注入する
    ///
    /// WebTransport CONNECT 送信前に呼び出す必要がある。
    /// (draft-ietf-webtrans-http3-15 Section 3.1)
    pub fn set_webtransport_transport_verified(
        &mut self,
        max_datagram_frame_size_nonzero: bool,
        reset_stream_at_supported: bool,
    ) -> Result<(), Error> {
        self.inner.set_webtransport_transport_verified(
            max_datagram_frame_size_nonzero,
            reset_stream_at_supported,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_connection() {
        let mut client = ClientConnection::with_default_settings();
        client.set_control_stream_id(2).expect("test must succeed");

        let headers = vec![
            Header::new(b":method", b"GET").expect("test must succeed"),
            Header::new(b":path", b"/").expect("test must succeed"),
            Header::new(b":scheme", b"https").expect("test must succeed"),
            Header::new(b":authority", b"example.com").expect("test must succeed"),
        ];

        let stream_id = client
            .send_request(&headers, true)
            .expect("test must succeed");
        assert_eq!(stream_id, 0);
    }
}
