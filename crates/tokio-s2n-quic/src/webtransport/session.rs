//! WebTransport セッションとストリーム型

use bytes::Bytes;
use s2n_quic::connection::BidirectionalStreamAcceptor;
use s2n_quic::stream::{ReceiveStream, SendStream};
use shiguredo_http3::webtransport::capsule::Capsule;
use shiguredo_http3::webtransport::stream::StreamHeader;
use tokio::sync::mpsc;

/// WebTransport セッション
pub struct WtSession {
    /// セッション ID
    session_id: u64,
    /// 双方向ストリームアクセプター
    bidi_acceptor: BidirectionalStreamAcceptor,
    /// 接続ハンドル
    handle: s2n_quic::connection::Handle,
    /// CONNECT ストリームの送信端 (WT_CLOSE_SESSION 送信 + FIN 送出用)
    connect_send: SendStream,
    /// WT 単方向ストリーム受信チャネル
    uni_rx: mpsc::Receiver<WtRecvStream>,
}

impl WtSession {
    /// 新しいセッションを作成する
    pub(crate) fn new(
        session_id: u64,
        bidi_acceptor: BidirectionalStreamAcceptor,
        handle: s2n_quic::connection::Handle,
        connect_send: SendStream,
        uni_rx: mpsc::Receiver<WtRecvStream>,
    ) -> Self {
        Self {
            session_id,
            bidi_acceptor,
            handle,
            connect_send,
            uni_rx,
        }
    }

    /// セッション ID を取得する
    pub fn session_id(&self) -> u64 {
        self.session_id
    }

    /// 双方向ストリームを受け付ける
    ///
    /// 受信した QUIC 双方向ストリームから WT_STREAM ヘッダー (0x41 + session_id) を解析する
    /// (draft-ietf-webtrans-http3-15 Section 4.2)
    pub async fn accept_bi_stream(&mut self) -> crate::Result<WtBiStream> {
        let stream: s2n_quic::stream::BidirectionalStream = self
            .bidi_acceptor
            .accept_bidirectional_stream()
            .await
            .map_err(crate::Error::transport)?
            .ok_or(crate::Error::StreamClosed)?;

        let stream_id: u64 = stream.id();
        let (mut recv, send) = stream.split();

        // WT_STREAM ヘッダー (0x41 + session_id) をデコード
        let mut header_buf: Vec<u8> = Vec::new();
        let pending = loop {
            let data = recv
                .receive()
                .await
                .map_err(crate::Error::transport)?
                .ok_or(crate::Error::StreamClosed)?;
            header_buf.extend_from_slice(&data);
            if let Some((_, consumed)) = StreamHeader::decode_bidirectional(&header_buf) {
                break header_buf[consumed..].to_vec();
            }
        };

        Ok(WtBiStream {
            stream_id,
            recv,
            send,
            pending,
        })
    }

    /// 新しい双方向ストリームを開く
    ///
    /// WT_STREAM ヘッダー (0x41 + session_id) を先頭に送信する
    /// (draft-ietf-webtrans-http3-15 Section 4.2)
    pub async fn open_bi_stream(&mut self) -> crate::Result<WtBiStream> {
        let stream = self
            .handle
            .open_bidirectional_stream()
            .await
            .map_err(crate::Error::transport)?;
        let stream_id: u64 = stream.id();
        let (recv, mut send) = stream.split();

        // WT_STREAM ヘッダー (0x41 + session_id) を送信
        let mut header = Vec::new();
        // session_id は CONNECT ストリーム ID なので必ず client-initiated bidi
        StreamHeader::new(self.session_id)
            .expect("session_id must be a client-initiated bidi stream id")
            .encode_bidirectional(&mut header);
        send.send(Bytes::from(header))
            .await
            .map_err(crate::Error::transport)?;

        Ok(WtBiStream {
            stream_id,
            recv,
            send,
            pending: Vec::new(),
        })
    }

    /// 単方向ストリームを受け付ける
    ///
    /// uni_task が WT 単方向ストリーム (0x54) をルーティングしたものを返す
    pub async fn accept_uni_stream(&mut self) -> crate::Result<WtRecvStream> {
        self.uni_rx.recv().await.ok_or(crate::Error::StreamClosed)
    }

    /// 新しい単方向ストリームを開く
    ///
    /// WT 単方向ストリームヘッダー (0x54 + session_id) を先頭に送信する
    /// (draft-ietf-webtrans-http3-15 Section 4.3)
    pub async fn open_uni_stream(&mut self) -> crate::Result<WtSendStream> {
        let stream = self
            .handle
            .open_send_stream()
            .await
            .map_err(crate::Error::transport)?;
        let stream_id: u64 = stream.id();
        let mut send = stream;

        // WT 単方向ストリームヘッダー (0x54 + session_id) を送信
        let mut header = Vec::new();
        // session_id は CONNECT ストリーム ID なので必ず client-initiated bidi
        StreamHeader::new(self.session_id)
            .expect("session_id must be a client-initiated bidi stream id")
            .encode_unidirectional(&mut header);
        send.send(Bytes::from(header))
            .await
            .map_err(crate::Error::transport)?;

        Ok(WtSendStream { stream_id, send })
    }

    /// セッションをクローズする
    ///
    /// WT_CLOSE_SESSION カプセルを H3 DATA フレームに包んで CONNECT ストリームに送信し、
    /// 直後に FIN を送出する
    /// (draft-ietf-webtrans-http3-16 Section 6: WT_CLOSE_SESSION を送信したエンドポイントは
    /// CONNECT ストリームに即座に FIN を送らなければならない)
    pub async fn close(&mut self, code: u32, reason: &str) -> crate::Result<()> {
        let capsule = Capsule::CloseSession {
            error_code: code,
            message: reason.to_string(),
        };
        // CONNECT ストリーム上のカプセルは HTTP/3 DATA フレーム (0x00 + varint 長 + ペイロード)
        // として送出する必要がある (RFC 9297 Section 3.1 / RFC 9114 Section 7.2.1)。
        let mut buf = Vec::new();
        capsule.encode_as_data_frame(&mut buf);
        self.connect_send
            .send(Bytes::from(buf))
            .await
            .map_err(crate::Error::transport)?;
        // FIN 送出
        self.connect_send.finish().map_err(crate::Error::transport)
    }
}

/// WebTransport 双方向ストリーム
pub struct WtBiStream {
    /// ストリーム ID
    stream_id: u64,
    /// 受信ストリーム
    recv: ReceiveStream,
    /// 送信ストリーム
    send: SendStream,
    /// ヘッダー解析後の残留データ
    pending: Vec<u8>,
}

impl WtBiStream {
    /// データを送信する
    pub async fn send(&mut self, data: &[u8]) -> crate::Result<()> {
        self.send
            .send(Bytes::copy_from_slice(data))
            .await
            .map_err(crate::Error::transport)
    }

    /// データを受信する
    ///
    /// ヘッダー解析後の残留データがある場合はそれを先に返す
    pub async fn recv(&mut self) -> crate::Result<Vec<u8>> {
        if !self.pending.is_empty() {
            return Ok(std::mem::take(&mut self.pending));
        }
        let received: Result<Option<Bytes>, _> = self.recv.receive().await;
        match received {
            Ok(Some(data)) => Ok(data.to_vec()),
            Ok(None) => Err(crate::Error::StreamClosed),
            Err(e) => Err(crate::Error::transport(e)),
        }
    }

    /// ストリームを終了する
    pub fn finish(&mut self) -> crate::Result<()> {
        self.send.finish().map_err(crate::Error::transport)
    }

    /// ストリーム ID を取得する
    pub fn stream_id(&self) -> u64 {
        self.stream_id
    }
}

/// WebTransport 送信ストリーム
pub struct WtSendStream {
    /// ストリーム ID
    stream_id: u64,
    /// 送信ストリーム
    send: SendStream,
}

impl WtSendStream {
    /// データを送信する
    pub async fn send(&mut self, data: &[u8]) -> crate::Result<()> {
        self.send
            .send(Bytes::copy_from_slice(data))
            .await
            .map_err(crate::Error::transport)
    }

    /// ストリームを終了する
    pub fn finish(&mut self) -> crate::Result<()> {
        self.send.finish().map_err(crate::Error::transport)
    }

    /// ストリーム ID を取得する
    pub fn stream_id(&self) -> u64 {
        self.stream_id
    }
}

/// WebTransport 受信ストリーム
pub struct WtRecvStream {
    /// ストリーム ID
    stream_id: u64,
    /// 受信ストリーム
    recv: ReceiveStream,
    /// ヘッダー解析後の残留データ
    pending: Vec<u8>,
}

impl WtRecvStream {
    /// 新しい受信ストリームを作成する
    pub(crate) fn new(stream_id: u64, recv: ReceiveStream, pending: Vec<u8>) -> Self {
        Self {
            stream_id,
            recv,
            pending,
        }
    }

    /// データを受信する
    ///
    /// ヘッダー解析後の残留データがある場合はそれを先に返す
    pub async fn recv(&mut self) -> crate::Result<Vec<u8>> {
        if !self.pending.is_empty() {
            return Ok(std::mem::take(&mut self.pending));
        }
        let received: Result<Option<Bytes>, _> = self.recv.receive().await;
        match received {
            Ok(Some(data)) => Ok(data.to_vec()),
            Ok(None) => Err(crate::Error::StreamClosed),
            Err(e) => Err(crate::Error::transport(e)),
        }
    }

    /// ストリーム ID を取得する
    pub fn stream_id(&self) -> u64 {
        self.stream_id
    }
}
