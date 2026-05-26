//! ConnectionState - HTTP/3 接続の内部状態管理
//!
//! std::sync::Mutex で保護される。await を跨がない。

use shiguredo_http3::{
    ClientConnection, Event, H3InitData, Header, ServerConnection, Settings as H3Settings,
};

/// サーバー側 HTTP/3 接続状態
pub(crate) struct ServerConnectionState {
    /// HTTP/3 接続 (Sans I/O)
    pub(crate) h3_conn: ServerConnection,
    /// 制御ストリーム ID (送信側)
    pub(crate) control_stream_id: Option<u64>,
    /// QPACK エンコーダーストリーム ID (送信側)
    pub(crate) encoder_stream_id: Option<u64>,
    /// QPACK デコーダーストリーム ID (送信側)
    pub(crate) decoder_stream_id: Option<u64>,
}

impl ServerConnectionState {
    /// 新しいサーバー接続状態を作成する
    pub(crate) fn new(settings: H3Settings) -> Self {
        Self {
            h3_conn: ServerConnection::new(settings),
            control_stream_id: None,
            encoder_stream_id: None,
            decoder_stream_id: None,
        }
    }

    /// 制御ストリーム・QPACK ストリームを一括で初期化する
    pub(crate) fn init_h3_streams(
        &mut self,
        control_stream_id: u64,
        encoder_stream_id: u64,
        decoder_stream_id: u64,
    ) -> crate::Result<H3InitData> {
        self.control_stream_id = Some(control_stream_id);
        self.encoder_stream_id = Some(encoder_stream_id);
        self.decoder_stream_id = Some(decoder_stream_id);
        Ok(self
            .h3_conn
            .init_h3_streams(control_stream_id, encoder_stream_id, decoder_stream_id)?)
    }

    /// QPACK ストリームの送信待ちデータをドレインする
    ///
    /// エンコーダーストリーム / デコーダーストリームに蓄積されたデータを
    /// (stream_id, data) のペアで返す。
    pub(crate) fn drain_qpack_data(&mut self) -> Vec<(u64, Vec<u8>)> {
        let mut result = Vec::new();
        for stream_id in [self.encoder_stream_id, self.decoder_stream_id]
            .into_iter()
            .flatten()
        {
            if let Some((data, _fin)) = self.h3_conn.take_stream_data(stream_id) {
                result.push((stream_id, data));
            }
        }
        result
    }

    /// ストリームデータを処理してイベントを返す
    pub(crate) fn process_stream_data(
        &mut self,
        stream_id: u64,
        data: &[u8],
        fin: bool,
    ) -> crate::Result<Vec<Event>> {
        self.h3_conn.feed_stream(stream_id, data, fin)?;
        Ok(self.h3_conn.drain_events()?)
    }

    /// ストリームデータをフィードするだけでイベントをドレインしない
    ///
    /// QPACK エンコーダーストリームなど、イベント生成とは独立して処理する
    /// 単方向ストリームに使用する。イベントは `drain_events` で後から取得する。
    pub(crate) fn feed_stream_only(
        &mut self,
        stream_id: u64,
        data: &[u8],
        fin: bool,
    ) -> crate::Result<()> {
        self.h3_conn.feed_stream(stream_id, data, fin)?;
        Ok(())
    }

    /// 内部キューの全イベントをドレインして返す
    pub(crate) fn drain_events(&mut self) -> crate::Result<Vec<Event>> {
        Ok(self.h3_conn.drain_events()?)
    }

    /// レスポンスを準備する
    pub(crate) fn prepare_response(
        &mut self,
        stream_id: u64,
        headers: &[Header],
        body: &[u8],
    ) -> crate::Result<()> {
        self.h3_conn.send_response(stream_id, headers, false)?;
        self.h3_conn.send_body(stream_id, body, true)?;
        Ok(())
    }

    /// ストリームの送信データを取得する
    pub(crate) fn get_stream_data(&mut self, stream_id: u64) -> Option<(Vec<u8>, bool)> {
        self.h3_conn.take_stream_data(stream_id)
    }
}

/// クライアント側 HTTP/3 接続状態
pub(crate) struct ClientConnectionState {
    /// HTTP/3 接続 (Sans I/O)
    pub(crate) h3_conn: ClientConnection,
    /// 制御ストリーム ID (送信側)
    pub(crate) control_stream_id: Option<u64>,
    /// QPACK エンコーダーストリーム ID (送信側)
    pub(crate) encoder_stream_id: Option<u64>,
    /// QPACK デコーダーストリーム ID (送信側)
    pub(crate) decoder_stream_id: Option<u64>,
}

impl ClientConnectionState {
    /// 新しいクライアント接続状態を作成する
    pub(crate) fn new(settings: H3Settings) -> Self {
        Self {
            h3_conn: ClientConnection::new(settings),
            control_stream_id: None,
            encoder_stream_id: None,
            decoder_stream_id: None,
        }
    }

    /// 制御ストリーム・QPACK ストリームを一括で初期化する
    pub(crate) fn init_h3_streams(
        &mut self,
        control_stream_id: u64,
        encoder_stream_id: u64,
        decoder_stream_id: u64,
    ) -> crate::Result<H3InitData> {
        self.control_stream_id = Some(control_stream_id);
        self.encoder_stream_id = Some(encoder_stream_id);
        self.decoder_stream_id = Some(decoder_stream_id);
        Ok(self
            .h3_conn
            .init_h3_streams(control_stream_id, encoder_stream_id, decoder_stream_id)?)
    }

    /// QPACK ストリームの送信待ちデータをドレインする
    ///
    /// エンコーダーストリーム / デコーダーストリームに蓄積されたデータを
    /// (stream_id, data) のペアで返す。
    pub(crate) fn drain_qpack_data(&mut self) -> Vec<(u64, Vec<u8>)> {
        let mut result = Vec::new();
        for stream_id in [self.encoder_stream_id, self.decoder_stream_id]
            .into_iter()
            .flatten()
        {
            if let Some((data, _fin)) = self.h3_conn.take_stream_data(stream_id) {
                result.push((stream_id, data));
            }
        }
        result
    }

    /// リクエストを送信する
    pub(crate) fn send_request(&mut self, headers: &[Header], fin: bool) -> crate::Result<u64> {
        let stream_id = self.h3_conn.send_request(headers, fin)?;
        Ok(stream_id)
    }

    /// ストリームデータを処理してイベントを返す
    pub(crate) fn process_stream_data(
        &mut self,
        stream_id: u64,
        data: &[u8],
        fin: bool,
    ) -> crate::Result<Vec<Event>> {
        self.h3_conn.feed_stream(stream_id, data, fin)?;
        Ok(self.h3_conn.drain_events()?)
    }

    /// ストリームデータをフィードするだけでイベントをドレインしない
    ///
    /// QPACK デコーダーストリームなど、イベント生成とは独立して処理する
    /// 単方向ストリームに使用する。イベントは `drain_events` で後から取得する。
    pub(crate) fn feed_stream_only(
        &mut self,
        stream_id: u64,
        data: &[u8],
        fin: bool,
    ) -> crate::Result<()> {
        self.h3_conn.feed_stream(stream_id, data, fin)?;
        Ok(())
    }

    /// 内部キューの全イベントをドレインして返す
    pub(crate) fn drain_events(&mut self) -> crate::Result<Vec<Event>> {
        Ok(self.h3_conn.drain_events()?)
    }

    /// ストリームの送信データを取得する
    pub(crate) fn get_stream_data(&mut self, stream_id: u64) -> Option<(Vec<u8>, bool)> {
        self.h3_conn.take_stream_data(stream_id)
    }
}
