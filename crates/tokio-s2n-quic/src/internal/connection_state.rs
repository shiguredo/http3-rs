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

    /// WebTransport CONNECT ストリームのリセット (RESET_STREAM 受信) を通知しイベントを返す
    ///
    /// s2n-quic の `stream::Error` は Final Size を公開しないため常に 0 を渡す。
    /// sans-I/O 層は CONNECT ストリームのリセットで `terminate_wt_session` を呼び
    /// `SessionClosed` イベントを発火するのみで `final_size` は使用しない
    /// (draft-ietf-webtrans-http3-16 Section 6)。
    pub(crate) fn connect_stream_reset(
        &mut self,
        stream_id: u64,
        error_code: u64,
    ) -> crate::Result<Vec<Event>> {
        self.h3_conn.stream_reset(stream_id, error_code, 0)?;
        Ok(self.h3_conn.drain_events()?)
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

    /// WebTransport CONNECT ストリームのリセット (RESET_STREAM 受信) を通知しイベントを返す
    ///
    /// s2n-quic の `stream::Error` は Final Size を公開しないため常に 0 を渡す。
    /// sans-I/O 層は CONNECT ストリームのリセットで `terminate_wt_session` を呼び
    /// `SessionClosed` イベントを発火するのみで `final_size` は使用しない
    /// (draft-ietf-webtrans-http3-16 Section 6)。
    pub(crate) fn connect_stream_reset(
        &mut self,
        stream_id: u64,
        error_code: u64,
    ) -> crate::Result<Vec<Event>> {
        self.h3_conn.stream_reset(stream_id, error_code, 0)?;
        Ok(self.h3_conn.drain_events()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shiguredo_http3::Event;

    /// クライアント側の初期化データを検証する
    #[test]
    fn test_client_init_h3_streams() {
        let mut state = ClientConnectionState::new(H3Settings::default());
        let init = state
            .init_h3_streams(2, 6, 10)
            .expect("テスト用の初期化に成功すること");

        assert_eq!(init.control_stream_id, 2);
        assert_eq!(init.encoder_stream_id, 6);
        assert_eq!(init.decoder_stream_id, 10);

        // 制御ストリームの初期データはストリームタイプ 0x00 (制御ストリーム) で始まる
        assert_eq!(init.control_data[0], 0x00);
        // QPACK エンコーダーストリームはストリームタイプ 0x02 で始まる
        assert_eq!(init.encoder_data[0], 0x02);
        // QPACK デコーダーストリームはストリームタイプ 0x03 で始まる
        assert_eq!(init.decoder_data[0], 0x03);
    }

    /// サーバー側の初期化データを検証する
    #[test]
    fn test_server_init_h3_streams() {
        let mut state = ServerConnectionState::new(H3Settings::default());
        let init = state
            .init_h3_streams(3, 7, 11)
            .expect("テスト用の初期化に成功すること");

        assert_eq!(init.control_stream_id, 3);
        assert_eq!(init.control_data[0], 0x00);
        assert_eq!(init.encoder_data[0], 0x02);
        assert_eq!(init.decoder_data[0], 0x03);
    }

    /// QPACK ストリームの送信待ちデータをドレインする
    #[test]
    fn test_drain_qpack_data_after_init_is_empty() {
        // init_h3_streams が初期データを take_stream_data で取り切るため、
        // 初期化直後の QPACK ドレインは空であること (漏れがあるとデータが残る)
        let mut state = ClientConnectionState::new(H3Settings::default());
        state
            .init_h3_streams(2, 6, 10)
            .expect("テスト用の初期化に成功すること");

        let qpack = state.drain_qpack_data();
        assert!(
            qpack.is_empty(),
            "初期データは init_h3_streams で取り出され、ドレインに残らないこと: {qpack:?}"
        );
    }

    /// ピアの SETTINGS 受信で SettingsReceived イベントが生成される
    #[test]
    fn test_client_settings_received() {
        let mut state = ClientConnectionState::new(H3Settings::default());
        state
            .init_h3_streams(2, 6, 10)
            .expect("テスト用の初期化に成功すること");

        // サーバー開始の単方向ストリーム (ID 3): ストリームタイプ 0x00 + SETTINGS
        // (SETTINGS フレーム: type=4, length=0: [0x04, 0x00])
        let data = [0x00, 0x04, 0x00];
        let events = state
            .process_stream_data(3, &data, false)
            .expect("テスト用の SETTINGS 処理に成功すること");
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::SettingsReceived { .. })),
            "SettingsReceived イベントが生成されること: {events:?}"
        );
    }

    /// QPACK エンコーダーストリームは feed_stream_only で処理し、イベント化しない
    #[test]
    fn test_feed_stream_only_does_not_generate_events() {
        let mut state = ClientConnectionState::new(H3Settings::default());
        state
            .init_h3_streams(2, 6, 10)
            .expect("テスト用の初期化に成功すること");

        // サーバー開始のエンコーダーストリーム (ID 7): ストリームタイプ 0x02
        state
            .feed_stream_only(7, &[0x02], false)
            .expect("テスト用の feed に成功すること");
        assert!(
            state
                .drain_events()
                .expect("イベントドレインに成功すること")
                .is_empty(),
            "エンコーダーストリームからはイベントが生成されないこと"
        );
    }

    /// クライアント: リクエスト送信でリクエストデータと FIN が生成される
    #[test]
    fn test_client_send_request_generates_qpack_data() {
        let mut state = ClientConnectionState::new(H3Settings::default());
        state
            .init_h3_streams(2, 6, 10)
            .expect("テスト用の初期化に成功すること");

        let headers = vec![
            Header::new(b":method", b"GET").expect("テスト用のヘッダーに成功すること"),
            Header::new(b":scheme", b"https").expect("テスト用のヘッダーに成功すること"),
            Header::new(b":authority", b"example.com").expect("テスト用のヘッダーに成功すること"),
            Header::new(b":path", b"/").expect("テスト用のヘッダーに成功すること"),
        ];
        let stream_id = state
            .send_request(&headers, true)
            .expect("テスト用のリクエスト送信に成功すること");

        // リクエスト本体も取得できる (取得後に FIN が交付される)
        let (_, fin) = state
            .get_stream_data(stream_id)
            .expect("送信データがあること");
        assert!(!fin, "データ取得時点では FIN ではないこと");
        let (_, fin) = state
            .get_stream_data(stream_id)
            .expect("追加要求で FIN が交付されること");
        assert!(fin, "追加取得で FIN が交付されること");
    }

    /// レスポンス準備でサーバーがヘッダー・ボディをリクエストストリームに出力する
    #[test]
    fn test_server_prepare_response_generates_stream_data() {
        let mut state = ServerConnectionState::new(H3Settings::default());
        state
            .init_h3_streams(3, 7, 11)
            .expect("テスト用の初期化に成功すること");

        // クライアントからのリクエストを直接 feed する (リクエストストリーム ID 0)
        let request = vec![
            Header::new(b":method", b"GET").expect("テスト用のヘッダーに成功すること"),
            Header::new(b":scheme", b"https").expect("テスト用のヘッダーに成功すること"),
            Header::new(b":authority", b"example.com").expect("テスト用のヘッダーに成功すること"),
            Header::new(b":path", b"/").expect("テスト用のヘッダーに成功すること"),
        ];
        let mut client = ClientConnectionState::new(H3Settings::default());
        client
            .init_h3_streams(2, 6, 10)
            .expect("テスト用の初期化に成功すること");
        let stream_id = client
            .send_request(&request, true)
            .expect("テスト用のリクエスト送信に成功すること");

        // リクエストデータ (HEADERS + FIN 交付) をサーバーへ feed する
        let mut request_data = Vec::new();
        while let Some((data, fin)) = client.get_stream_data(stream_id) {
            request_data.extend_from_slice(&data);
            if fin {
                break;
            }
            // FIN 交付の追加呼び出し (データ消費後の (空, fin=true))
        }
        state
            .process_stream_data(stream_id, &request_data, true)
            .expect("テスト用のリクエスト feed に成功すること");

        let response =
            vec![Header::new(b":status", b"200").expect("テスト用のヘッダーに成功すること")];
        state
            .prepare_response(stream_id, &response, b"hello")
            .expect("テスト用のレスポンス準備に成功すること");

        let (data, fin) = state
            .get_stream_data(stream_id)
            .expect("送信データがあること");
        assert!(!data.is_empty(), "レスポンスデータが生成されること");
        assert!(!fin, "データ取得時点では FIN ではないこと");
        let (_, fin) = state
            .get_stream_data(stream_id)
            .expect("追加要求で FIN が交付されること");
        assert!(fin, "追加取得で FIN が交付されること");
    }

    /// ストリームデータの feed エラーが透過される
    #[test]
    fn test_process_stream_data_error_is_forwarded() {
        let mut state = ClientConnectionState::new(H3Settings::default());
        state
            .init_h3_streams(2, 6, 10)
            .expect("テスト用の初期化に成功すること");

        // サーバー開始の制御ストリーム (ID 3) に DATA フレーム (type=0, length=0) を
        // feed すると H3_FRAME_UNEXPECTED 接続エラーになる (RFC 9114 Section 6.2.1:
        // 制御ストリームでは DATA フレーム禁止)
        let err = state.process_stream_data(3, &[0x00, 0x00, 0x00], false);
        assert!(
            err.is_err(),
            "制御ストリームへの DATA フレームはエラーになること: {err:?}"
        );
    }
}
