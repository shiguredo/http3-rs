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
    /// CONNECT ストリームの送信端 (CLOSE_WEBTRANSPORT_SESSION 送信用)
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
        // issue 0059 Phase 1: BytesMut で蓄積し、ヘッダ消費後の残留は split_off で zero-copy 切り出し
        let mut header_buf = bytes::BytesMut::new();
        let pending: Bytes = loop {
            let data = recv
                .receive()
                .await
                .map_err(crate::Error::transport)?
                .ok_or(crate::Error::StreamClosed)?;
            header_buf.extend_from_slice(&data);
            if let Some((_, consumed)) = StreamHeader::decode_bidirectional(&header_buf) {
                bytes::Buf::advance(&mut header_buf, consumed);
                break header_buf.freeze();
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
            pending: Bytes::new(),
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
    /// CLOSE_WEBTRANSPORT_SESSION カプセルを CONNECT ストリームに送信する
    /// (draft-ietf-webtrans-http3-15 Section 5.6)
    pub async fn close(&mut self, code: u32, reason: &str) -> crate::Result<()> {
        let capsule = Capsule::CloseSession {
            error_code: code,
            message: reason.to_string(),
        };
        let mut buf = Vec::new();
        capsule.encode(&mut buf);
        self.connect_send
            .send(Bytes::from(buf))
            .await
            .map_err(crate::Error::transport)
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
    /// issue 0059 Phase 1: Bytes 化して to_vec() を排除
    pending: Bytes,
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
    pub async fn recv(&mut self) -> crate::Result<Bytes> {
        if !self.pending.is_empty() {
            return Ok(std::mem::take(&mut self.pending));
        }
        let received: Result<Option<Bytes>, _> = self.recv.receive().await;
        match received {
            Ok(Some(data)) => Ok(data),
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
    /// issue 0059 Phase 1: Bytes 化して to_vec() を排除
    pending: Bytes,
}

impl WtRecvStream {
    /// 新しい受信ストリームを作成する
    pub(crate) fn new(stream_id: u64, recv: ReceiveStream, pending: Bytes) -> Self {
        Self {
            stream_id,
            recv,
            pending,
        }
    }

    /// データを受信する
    ///
    /// ヘッダー解析後の残留データがある場合はそれを先に返す
    pub async fn recv(&mut self) -> crate::Result<Bytes> {
        if !self.pending.is_empty() {
            return Ok(std::mem::take(&mut self.pending));
        }
        let received: Result<Option<Bytes>, _> = self.recv.receive().await;
        match received {
            Ok(Some(data)) => Ok(data),
            Ok(None) => Err(crate::Error::StreamClosed),
            Err(e) => Err(crate::Error::transport(e)),
        }
    }

    /// ストリーム ID を取得する
    pub fn stream_id(&self) -> u64 {
        self.stream_id
    }
}
