//! HTTP/3 イベント
//!
//! Connection から返されるイベントを定義。

/// WebTransport セッション終了時にリセットすべきストリームの情報
///
/// (draft-ietf-webtrans-http3-15 Section 3.1 / 5.4)
///
/// `reliable_size` は draft-ietf-quic-reliable-stream-reset の `RESET_STREAM_AT`
/// に渡す reliable size。WebTransport データストリームの場合は stream header
/// (stream type / signal value + session_id varint) のバイト数以上である
/// 必要がある。`reset_stream_at` transport parameter がネゴシエートされていない
/// 経路 (draft-02/07) では呼び出し側が通常の `RESET_STREAM` にフォールバック
/// することを想定し、本フィールドは 0 でも構わない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WtStreamReset {
    /// QUIC ストリーム ID
    pub stream_id: u64,
    /// `RESET_STREAM_AT` の reliable size
    pub reliable_size: u64,
}

/// HTTP/3 イベント
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// SETTINGS フレーム受信
    SettingsReceived {
        /// ピアから受信した H3 設定
        settings: crate::settings::Settings,
        /// ピアから受信した WebTransport 設定
        wt_settings: Option<crate::webtransport::settings::Settings>,
    },
    /// ヘッダー受信開始
    HeadersBegin {
        /// ストリーム ID
        stream_id: u64,
    },
    /// 個別ヘッダー受信
    Header {
        /// ストリーム ID
        stream_id: u64,
        /// ヘッダー名
        name: Vec<u8>,
        /// ヘッダー値
        value: Vec<u8>,
    },
    /// ヘッダー受信完了
    HeadersEnd {
        /// ストリーム ID
        stream_id: u64,
    },
    /// データ受信
    Data {
        /// ストリーム ID
        stream_id: u64,
        /// データ
        data: Vec<u8>,
    },
    /// ストリーム終了
    StreamEnd {
        /// ストリーム ID
        stream_id: u64,
    },
    /// ストリームリセット (RESET_STREAM 受信)
    StreamReset {
        /// ストリーム ID
        stream_id: u64,
        /// エラーコード
        error_code: u64,
    },
    /// 送信停止要求 (STOP_SENDING 受信)
    StopSending {
        /// ストリーム ID
        stream_id: u64,
        /// エラーコード
        error_code: u64,
    },
    /// GOAWAY 受信
    GoawayReceived {
        /// GOAWAY で指定された ID
        id: u64,
    },
    /// WebTransport 双方向ストリーム開始
    /// (draft-ietf-webtrans-http3-15 Section 4.3)
    WebTransportBidiStreamOpen {
        /// QUIC ストリーム ID
        stream_id: u64,
        /// WebTransport セッション ID
        session_id: u64,
    },
    /// WebTransport 双方向ストリームデータ受信
    WebTransportBidiStreamData {
        /// QUIC ストリーム ID
        stream_id: u64,
        /// データ
        data: Vec<u8>,
    },
    /// WebTransport 双方向ストリーム終了 (FIN 受信)
    WebTransportBidiStreamEnd {
        /// QUIC ストリーム ID
        stream_id: u64,
    },
    /// WebTransport 単方向ストリーム開始
    /// (draft-ietf-webtrans-http3-15 Section 4.2)
    WebTransportUniStreamOpen {
        /// QUIC ストリーム ID
        stream_id: u64,
        /// WebTransport セッション ID
        session_id: u64,
    },
    /// WebTransport 単方向ストリームデータ受信
    WebTransportUniStreamData {
        /// QUIC ストリーム ID
        stream_id: u64,
        /// データ
        data: Vec<u8>,
    },
    /// WebTransport 単方向ストリーム終了 (FIN 受信)
    WebTransportUniStreamEnd {
        /// QUIC ストリーム ID
        stream_id: u64,
    },
    /// WebTransport セッション終了
    /// (draft-ietf-webtrans-http3-15 Section 6)
    ///
    /// セッションが終了した。`reset_streams` に含まれる全ストリームに対して
    /// `error_code` を使用して `RESET_STREAM_AT` (reliable_size を伴う) と
    /// STOP_SENDING を送信すること。`reset_stream_at` transport parameter が
    /// ネゴシエートされていない経路では通常の `RESET_STREAM` にフォールバックする。
    WebTransportSessionClosed {
        /// セッション ID (CONNECT ストリーム ID)
        session_id: u64,
        /// リセットすべきストリーム情報の一覧 (stream_id と reliable_size)
        reset_streams: Vec<WtStreamReset>,
        /// RESET_STREAM / STOP_SENDING に使用するエラーコード
        /// (WT_SESSION_GONE / WT_ALPN_ERROR 等)
        error_code: u64,
        /// WT_CLOSE_SESSION カプセルのアプリケーションエラーコード
        /// (draft-ietf-webtrans-http3-15 Section 6)
        /// WT_CLOSE_SESSION なしの終了 (FIN / RESET_STREAM) の場合は 0
        close_error_code: u32,
        /// WT_CLOSE_SESSION カプセルのエラーメッセージ
        /// (draft-ietf-webtrans-http3-15 Section 6)
        /// WT_CLOSE_SESSION なしの終了の場合は空文字列
        close_message: String,
    },
    /// WebTransport セッション確立
    /// (draft-ietf-webtrans-http3-15 Section 3)
    ///
    /// CONNECT ストリームに 200 OK が返された。
    /// バッファリングされていたストリーム/データグラムがあれば配送される。
    WebTransportSessionEstablished {
        /// セッション ID (CONNECT ストリーム ID)
        session_id: u64,
        /// フロー制御が有効かどうか (Section 5.1)
        ///
        /// 両端が SETTINGS でフロー制御を宣言した場合に `true`。
        /// `true` の場合、接続層がストリーム数/データ量の超過を検知し、
        /// WT_MAX_STREAMS / WT_MAX_DATA カプセルの生成を行う。
        flow_control_enabled: bool,
    },
    /// WebTransport セッション draining
    /// (draft-ietf-webtrans-http3-15 Section 6)
    ///
    /// WT_DRAIN_SESSION カプセルを受信した。
    /// 新規ストリームやデータグラムの送信を停止すること。
    /// セッションは即座に終了しないが、グレースフルシャットダウンを開始する。
    WebTransportSessionDraining {
        /// セッション ID (CONNECT ストリーム ID)
        session_id: u64,
    },
    /// WebTransport フロー制御カプセル受信
    /// (draft-ietf-webtrans-http3-15 Section 5.6)
    ///
    /// CONNECT ストリーム上でフロー制御カプセルを受信した。
    /// 上位層は `webtransport::Session::process_capsule` に渡すこと。
    WebTransportCapsule {
        /// WebTransport セッション ID
        session_id: u64,
        /// 受信した Capsule
        capsule: crate::webtransport::Capsule,
    },
    /// WebTransport データグラム受信
    /// (draft-ietf-webtrans-http3-15 Section 4.5)
    ///
    /// QUIC DATAGRAM フレームから WebTransport データグラムを受信した。
    WebTransportDatagram {
        /// WebTransport セッション ID
        session_id: u64,
        /// データグラムペイロード
        payload: Vec<u8>,
    },
    /// WebTransport データストリームのリセット受信
    /// (draft-ietf-webtrans-http3-15 Section 4.4)
    ///
    /// WebTransport セッションに属するデータストリームに対して RESET_STREAM を
    /// 受信した。アプリケーション層はセッション ID と application error code を
    /// 元にアプリへ通知すること。
    WebTransportStreamReset {
        /// WebTransport セッション ID
        session_id: u64,
        /// QUIC ストリーム ID
        stream_id: u64,
        /// QUIC application error code
        error_code: u64,
        /// QUIC RESET_STREAM の final size (RFC 9000 Section 19.4)
        ///
        /// `reset_stream_at` transport parameter がネゴシエートされている場合、
        /// この値は stream header (stream type / signal value + session_id varint)
        /// のバイト数以上であることが期待される (draft-ietf-webtrans-http3-15
        /// Section 5.4)。
        final_size: u64,
    },
    /// WebTransport データストリームへの STOP_SENDING 受信
    /// (draft-ietf-webtrans-http3-15 Section 4.4)
    WebTransportStreamStopSending {
        /// WebTransport セッション ID
        session_id: u64,
        /// QUIC ストリーム ID
        stream_id: u64,
        /// QUIC application error code
        error_code: u64,
    },
    /// WebTransport バッファリング拒否
    /// (draft-ietf-webtrans-http3-15 Section 4.6)
    ///
    /// バッファリング上限を超えたため、`error_code` を使用して
    /// RESET_STREAM / STOP_SENDING を送信すること。
    WebTransportBufferedStreamRejected {
        /// 拒否されたストリーム ID
        stream_id: u64,
        /// RESET_STREAM / STOP_SENDING に使用するエラーコード (WT_BUFFERED_STREAM_REJECTED)
        error_code: u64,
    },
    /// 接続エラー
    ConnectionError {
        /// エラーコード
        error_code: u64,
        /// エラー理由
        reason: String,
    },
}

impl Event {
    /// ストリーム ID を取得 (存在する場合)
    pub fn stream_id(&self) -> Option<u64> {
        match self {
            Self::HeadersBegin { stream_id }
            | Self::Header { stream_id, .. }
            | Self::HeadersEnd { stream_id }
            | Self::Data { stream_id, .. }
            | Self::StreamEnd { stream_id }
            | Self::StreamReset { stream_id, .. }
            | Self::StopSending { stream_id, .. }
            | Self::WebTransportBidiStreamOpen { stream_id, .. }
            | Self::WebTransportBidiStreamData { stream_id, .. }
            | Self::WebTransportBidiStreamEnd { stream_id }
            | Self::WebTransportUniStreamOpen { stream_id, .. }
            | Self::WebTransportUniStreamData { stream_id, .. }
            | Self::WebTransportUniStreamEnd { stream_id }
            | Self::WebTransportStreamReset { stream_id, .. }
            | Self::WebTransportStreamStopSending { stream_id, .. }
            | Self::WebTransportBufferedStreamRejected { stream_id, .. } => Some(*stream_id),
            Self::WebTransportSessionClosed { session_id, .. }
            | Self::WebTransportSessionEstablished { session_id, .. }
            | Self::WebTransportSessionDraining { session_id }
            | Self::WebTransportCapsule { session_id, .. }
            | Self::WebTransportDatagram { session_id, .. } => Some(*session_id),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_stream_id() {
        let event = Event::HeadersBegin { stream_id: 4 };
        assert_eq!(event.stream_id(), Some(4));

        let event = Event::SettingsReceived {
            settings: crate::settings::Settings::new(),
            wt_settings: None,
        };
        assert_eq!(event.stream_id(), None);
    }
}
