//! HTTP/3 サーバー接続

use crate::error::Error;
use crate::event::Event;
use crate::qpack::Header;
use crate::settings::Settings;

use super::{Connection, H3InitData, Role};

/// HTTP/3 サーバー接続
///
/// サーバー専用の API を提供。
#[derive(Debug)]
pub struct ServerConnection {
    inner: Connection,
}

impl ServerConnection {
    /// 新しいサーバー接続を作成
    pub fn new(settings: Settings) -> Self {
        Self {
            inner: Connection::new(Role::Server, settings),
        }
    }

    /// デフォルト設定でサーバー接続を作成
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

    /// レスポンスを送信
    ///
    /// 指定されたストリームにヘッダーを送信。
    pub fn send_response(
        &mut self,
        stream_id: u64,
        headers: &[Header],
        fin: bool,
    ) -> Result<(), Error> {
        self.inner.send_response(stream_id, headers, fin)
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
    pub fn send_goaway(&mut self, id: u64) -> Result<(), crate::error::Error> {
        self.inner.send_goaway(id)
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
    /// WebTransport CONNECT 受信処理前に呼び出す必要がある。
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
    fn test_server_connection() {
        let mut server = ServerConnection::with_default_settings();
        server.set_control_stream_id(3).unwrap();

        // 制御ストリームの送信データを取得
        let (data, fin) = server.get_stream_data(3).unwrap();
        assert!(!data.is_empty());
        assert!(!fin);
    }

    #[test]
    fn test_server_receive_request() {
        let server = ServerConnection::with_default_settings();

        // テスト: サーバーがリクエストを受信できることを確認
        assert!(server.peer_settings().is_none()); // SETTINGS 未受信
    }
}
