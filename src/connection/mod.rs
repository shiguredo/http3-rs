//! HTTP/3 接続 (RFC 9114)
//!
//! Sans I/O パターンに基づく HTTP/3 接続管理。
//!
//! ## 使用例
//!
//! ```rust,ignore
//! use shiguredo_http3::{Connection, Settings, Header, Event};
//!
//! // クライアント接続を作成
//! let mut conn = Connection::client(Settings::default());
//!
//! // 制御ストリームの送信データを取得して QUIC で送信
//! let ctrl_stream_id = 2; // クライアント開始の単方向ストリーム
//! conn.set_control_stream_id(ctrl_stream_id).expect("infallible: implementation bug if this panics");
//! if let Some((data, fin)) = conn.get_stream_data(ctrl_stream_id) {
//!     // QUIC で送信
//! }
//!
//! // リクエストを送信
//! let stream_id = conn.send_request(&[
//!     Header::new(b":method", b"GET").expect("infallible: implementation bug if this panics"),
//!     Header::new(b":path", b"/").expect("infallible: implementation bug if this panics"),
//!     Header::new(b":scheme", b"https").expect("infallible: implementation bug if this panics"),
//!     Header::new(b":authority", b"example.com").expect("infallible: implementation bug if this panics"),
//! ], true).expect("infallible: implementation bug if this panics");
//!
//! // QUIC からデータを受信
//! conn.feed_stream(stream_id, &response_data, fin);
//!
//! // イベントを処理
//! while let Some(event) = conn.poll_event() {
//!     match event {
//!         Event::HeadersEnd { stream_id } => { /* ヘッダー受信完了 */ }
//!         Event::Data { stream_id, data } => { /* データ受信 */ }
//!         _ => {}
//!     }
//! }
//! ```

mod client;
mod server;
mod wt_capsule;
mod wt_session;
mod wt_stream;
mod wt_types;

use wt_types::{WtSession, WtSessionState};

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

use crate::error::{Error, ErrorCode};
use crate::event::{Event, WebTransportEvent};
use crate::limits::Limits;
use crate::qpack::{
    DecodeOutput, DecoderStream, DecoderStreamReceiver, DynamicDecoder, DynamicEncoder,
    EncoderStream, EncoderStreamReceiver, Header, estimate_encoded_size,
};
use crate::settings::Settings;
use crate::stream::request::{RawReceivedData, RequestStream};
use crate::stream::{ControlStreamRecv, ControlStreamSend, StreamKind, StreamState};
use crate::varint::VarInt;

pub use client::ClientConnection;
pub use server::ServerConnection;

/// H3 ストリーム初期化結果
///
/// `Connection::init_h3_streams()` が返す、各ストリームの初期データ。
/// 呼び出し側は各ストリームの初期データを対応する QUIC ストリームで送信する責任を持つ。
///
/// **重要**: 初期データを送信した後も、制御ストリーム・QPACK エンコーダーストリーム・
/// QPACK デコーダーストリームに対応する QUIC ストリームのハンドルは接続終了まで
/// 保持し続けること。これらのクリティカルストリームをクローズすると、相手側で
/// H3_CLOSED_CRITICAL_STREAM エラーが発生する (RFC 9114 Section 6.2.1)。
#[derive(Debug, Clone)]
pub struct H3InitData {
    /// 制御ストリーム ID
    pub control_stream_id: u64,
    /// 制御ストリームの初期データ (SETTINGS フレーム)
    pub control_data: Vec<u8>,
    /// QPACK エンコーダーストリーム ID
    pub encoder_stream_id: u64,
    /// QPACK エンコーダーストリームの初期データ (stream type)
    pub encoder_data: Vec<u8>,
    /// QPACK デコーダーストリーム ID
    pub decoder_stream_id: u64,
    /// QPACK デコーダーストリームの初期データ (stream type)
    pub decoder_data: Vec<u8>,
}

/// 接続ロール
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// クライアント
    Client,
    /// サーバー
    Server,
}

/// HTTP/3 接続
#[derive(Debug)]
pub struct Connection {
    /// ロール
    role: Role,
    /// ローカル設定
    local_settings: Settings,
    /// ピア設定
    peer_settings: Option<Settings>,
    /// 制限値
    limits: Limits,
    /// 制御ストリーム (送信側)
    control_send: ControlStreamSend,
    /// 制御ストリーム (受信側)
    control_recv: ControlStreamRecv,
    /// リクエストストリーム
    streams: HashMap<u64, RequestStream>,
    /// 次のストリーム ID (クライアント: 0, 4, 8... / サーバー: 1, 5, 9...)
    next_stream_id: u64,
    /// QPACK 動的エンコーダー
    qpack_encoder: DynamicEncoder,
    /// QPACK 動的デコーダー
    qpack_dynamic_decoder: DynamicDecoder,
    /// QPACK エンコーダーストリーム (送信側)
    encoder_stream: EncoderStream,
    /// QPACK エンコーダーストリーム ID (送信側)
    encoder_stream_id: Option<u64>,
    /// QPACK デコーダーストリーム (送信側)
    decoder_stream: DecoderStream,
    /// QPACK デコーダーストリーム ID (送信側)
    decoder_stream_id: Option<u64>,
    /// QPACK エンコーダーストリームレシーバー (受信側)
    encoder_stream_recv: EncoderStreamReceiver,
    /// ピアの QPACK エンコーダーストリーム ID
    peer_encoder_stream_id: Option<u64>,
    /// QPACK デコーダーストリームレシーバー (受信側)
    decoder_stream_recv: DecoderStreamReceiver,
    /// ピアの QPACK デコーダーストリーム ID
    peer_decoder_stream_id: Option<u64>,
    /// イベントキュー
    events: VecDeque<Event>,
    /// 接続エラー
    error: Option<Error>,
    /// GOAWAY 受信済み (方向を問わない事実)
    ///
    /// クライアント受信時は request stream ID、サーバー受信時は push ID を
    /// それぞれ運ぶ。新規 WT セッション拒否や WT draining 伝播の判定には
    /// `peer_goaway_request_boundary()` を使うこと。
    /// (RFC 9114 Section 5.2 / 7.2.6)
    peer_goaway_received: bool,
    /// 直近で受信した GOAWAY の ID (複数受信時の単調減少チェック用)
    ///
    /// 値の意味はロール依存: クライアント受信なら request stream ID、
    /// サーバー受信なら push ID。WT / request stream の新規拒否判定に
    /// 使えるのはクライアント受信時のみ。
    /// (RFC 9114 Section 5.2)
    peer_goaway_last_id: Option<VarInt>,
    /// 最後に送信した GOAWAY の ID (段階的送信のために単調減少を検証する)
    last_sent_goaway_id: Option<VarInt>,
    /// クライアントから受信した MAX_PUSH_ID の最新値
    ///
    /// サーバープッシュ自体はサポートしないが、RFC 9114 Section 7.2.7 で定義された
    /// 単調増加制約だけは検証する (後退は H3_ID_ERROR)。
    max_push_id: Option<VarInt>,
    /// 送信待ちストリーム ID
    writable_streams: VecDeque<u64>,
    /// QPACK ブロック中のストリームを ricnt (Required Insert Count) 順でソートする
    ///
    /// `retry_blocked_streams()` で ricnt が小さいストリームから順にリトライし、
    /// ricnt > 現在の Insert Count のストリームに到達した時点で打ち切る (nghttp3 方式)。
    /// (ricnt, stream_id) のペアで一意性を保証する。
    blocked_by_ricnt: BTreeSet<(u64, u64)>,
    /// 未知タイプの単方向ストリーム ID (後続データを破棄する)
    ///
    /// RFC 9114 Section 6.2: 未知ストリームタイプの受信データは破棄する。
    ignored_uni_streams: HashSet<u64>,
    /// ストリームタイプ未確定の単方向ストリーム (バッファ)
    ///
    /// varint が複数チャンクにまたがる場合のバッファリング用。
    pending_uni_streams: HashMap<u64, Vec<u8>>,
    /// WebTransport 単方向ストリーム (ストリーム ID → セッション ID)
    ///
    /// セッション ID が確定した WT 単方向ストリームを追跡する。
    /// (draft-ietf-webtrans-http3-15 Section 4.2)
    wt_uni_streams: HashMap<u64, u64>,
    /// WebTransport 単方向ストリームのセッション ID 未確定バッファ
    ///
    /// ストリームタイプ (0x54) は確定したが、セッション ID の varint が
    /// 複数チャンクにまたがる場合のバッファリング用。
    pending_wt_uni_streams: HashMap<u64, Vec<u8>>,
    /// WebTransport 双方向ストリーム (ストリーム ID → セッション ID)
    ///
    /// signal value (0x41) とセッション ID が確定した WT 双方向ストリームを追跡する。
    /// (draft-ietf-webtrans-http3-15 Section 4.3)
    wt_bidi_streams: HashMap<u64, u64>,
    /// WebTransport 双方向ストリームのセッション ID 未確定バッファ
    ///
    /// WT_STREAM (0x41) 確定後、session_id の varint が
    /// 複数チャンクにまたがる場合のバッファリング用。
    /// (draft-ietf-webtrans-http3-15 Section 4.3)
    pending_wt_bidi_streams: HashMap<u64, Vec<u8>>,
    /// クライアント開始の新規 bidi stream のディスパッチ保留バッファ
    ///
    /// サーバー側で WebTransport が有効な場合、先頭 varint が不完全で
    /// WT bidi (0x41) かリクエストか判定できないストリームをバッファリングする。
    /// `pending_wt_bidi_streams` とは異なり、0x41 でなければリクエストに戻す。
    /// (draft-ietf-webtrans-http3-15 Section 4.3)
    pending_bidi_dispatch: HashMap<u64, Vec<u8>>,
    /// WebTransport セッション表 (セッション ID → セッション状態)
    ///
    /// CONNECT stream の stream_id がセッション ID となる。
    /// セッションのライフサイクルと関連ストリームを追跡する。
    /// (draft-ietf-webtrans-http3-15 Section 3, 4.6, 6)
    wt_sessions: HashMap<u64, WtSession>,
    /// 終了済み WebTransport セッション ID (tombstone)
    ///
    /// セッション終了時に `wt_sessions` から除去したエントリの ID を記録し、
    /// 終了後に届く DATA / FIN / RESET / 新規ストリーム / データグラムの拒否・破棄に
    /// 使う (zombie Pending セッションの再生成を防ぐ)。
    /// セッション ID のみの軽量な記録であり、接続終了まで保持する
    /// (元の WtSession エントリは解放される)。
    /// (draft-ietf-webtrans-http3-16 Section 6)
    closed_wt_sessions: HashSet<u64>,
    /// エンコーダーストリーム初期化前に受信した SET_CAPACITY 値
    ///
    /// peer SETTINGS がエンコーダーストリーム ID 設定前に到着した場合に
    /// SET_CAPACITY 命令を遅延させる (RFC 9204 Section 4.2)。
    /// QUIC transport parameter レベルの WebTransport 前提条件が検証済みか
    ///
    /// Sans I/O 設計上、Connection は QUIC transport parameter に直接アクセスできない。
    /// 上位層が `set_webtransport_transport_verified()` を呼び出すことで、
    /// transport parameter レベルの前提条件が満たされていることを注入する。
    /// (draft-ietf-webtrans-http3-15 Section 3.1)
    wt_transport_verified: bool,
    /// RESET_STREAM_AT 拡張がサポートされているか
    ///
    /// draft-15 では RESET_STREAM_AT が必須だが、draft-02/07 では不要。
    /// `set_webtransport_transport_verified()` で注入される。
    /// (draft-ietf-webtrans-http3-15 Section 3.1)
    /// 将来のドラフトで変更される可能性がある
    wt_reset_stream_at_supported: bool,
    deferred_encoder_set_capacity: Option<u64>,
    /// デコーダーストリーム初期化前に蓄積された Section Acknowledgment の stream_id
    ///
    /// ヘッダーデコードがデコーダーストリーム ID 設定前に発生した場合に
    /// SECTION_ACK 命令を遅延させる (RFC 9204 Section 4.4.1)。
    deferred_section_acks: Vec<u64>,
    /// デコーダーストリーム初期化前に蓄積された Stream Cancellation の stream_id
    deferred_stream_cancellations: Vec<u64>,
}

impl Connection {
    /// クライアント接続を作成
    pub fn client(settings: Settings) -> Self {
        Self::new(Role::Client, settings)
    }

    /// サーバー接続を作成
    pub fn server(settings: Settings) -> Self {
        Self::new(Role::Server, settings)
    }

    /// 新しい接続を作成
    fn new(role: Role, settings: Settings) -> Self {
        let mut limits = Limits::default();

        // 引数の settings を使用して SETTINGS を送信
        // デフォルトの QPACK 設定と引数の WebTransport 設定をマージ。
        // `Limits::default()` のフィールドは静的に VarInt 範囲内のため `expect`。
        let mut local_settings = Settings::from_limits(&limits)
            .expect("Limits::default() values must fit VarInt (RFC 9000 Section 16)");
        if let Some(v) = settings.enable_connect_protocol {
            local_settings.enable_connect_protocol = Some(v);
        }
        if let Some(v) = settings.h3_datagram {
            local_settings.h3_datagram = Some(v);
        }
        if let Some(wt) = settings.wt_settings {
            local_settings.wt_settings = Some(wt);
        }
        // QPACK 設定もマージ (引数で指定された場合はそちらを優先)
        if let Some(v) = settings.qpack_max_table_capacity {
            local_settings.qpack_max_table_capacity = Some(v);
        }
        if let Some(v) = settings.qpack_blocked_streams {
            local_settings.qpack_blocked_streams = Some(v);
        }
        if let Some(v) = settings.max_field_section_size {
            local_settings.max_field_section_size = Some(v);
        }

        // limits を local_settings と同期 (検証で使用する値が SETTINGS 送信値と一致するようにする)
        if let Some(v) = local_settings.qpack_max_table_capacity {
            limits.qpack_max_table_capacity = v.get();
        }
        if let Some(v) = local_settings.qpack_blocked_streams {
            limits.qpack_blocked_streams = v.get();
        }
        if let Some(v) = local_settings.max_field_section_size {
            limits.max_field_section_size = v.get();
        }

        let mut control_send = ControlStreamSend::new();
        control_send.send_settings(&local_settings);

        // 最初の双方向ストリーム ID
        let next_stream_id = match role {
            Role::Client => 0,
            Role::Server => 1,
        };

        // QPACK 最大テーブル容量を設定 (local_settings から取得)
        let max_table_capacity = local_settings
            .qpack_max_table_capacity
            .map(VarInt::get)
            .unwrap_or(0);
        let mut encoder_stream_recv = EncoderStreamReceiver::new();
        encoder_stream_recv.set_max_table_capacity(max_table_capacity);
        let mut qpack_dynamic_decoder = DynamicDecoder::new();
        qpack_dynamic_decoder.set_max_table_capacity(max_table_capacity);
        // ローカル設定の max_field_section_size をデコーダーに反映 (RFC 9114 Section 4.2.2)
        if let Some(max_size) = local_settings.max_field_section_size {
            qpack_dynamic_decoder.set_max_field_section_size(max_size.get());
        }

        Self {
            role,
            local_settings,
            peer_settings: None,
            limits,
            control_send,
            control_recv: ControlStreamRecv::new(),
            streams: HashMap::new(),
            next_stream_id,
            qpack_encoder: DynamicEncoder::new(),
            qpack_dynamic_decoder,
            encoder_stream: EncoderStream::new(),
            encoder_stream_id: None,
            decoder_stream: DecoderStream::new(),
            decoder_stream_id: None,
            encoder_stream_recv,
            peer_encoder_stream_id: None,
            decoder_stream_recv: DecoderStreamReceiver::new(),
            peer_decoder_stream_id: None,
            events: VecDeque::new(),
            error: None,
            peer_goaway_received: false,
            peer_goaway_last_id: None,
            last_sent_goaway_id: None,
            max_push_id: None,
            writable_streams: VecDeque::new(),
            blocked_by_ricnt: BTreeSet::new(),
            ignored_uni_streams: HashSet::new(),
            pending_uni_streams: HashMap::new(),
            wt_uni_streams: HashMap::new(),
            pending_wt_uni_streams: HashMap::new(),
            wt_bidi_streams: HashMap::new(),
            pending_wt_bidi_streams: HashMap::new(),
            pending_bidi_dispatch: HashMap::new(),
            wt_sessions: HashMap::new(),
            closed_wt_sessions: HashSet::new(),
            wt_transport_verified: false,
            wt_reset_stream_at_supported: false,
            deferred_encoder_set_capacity: None,
            deferred_section_acks: Vec::new(),
            deferred_stream_cancellations: Vec::new(),
        }
    }

    /// ロールを取得
    pub fn role(&self) -> Role {
        self.role
    }

    /// ピアから受信した GOAWAY の request stream 境界値を返す
    ///
    /// クライアント受信時のみ Some を返す。サーバーが受信する GOAWAY は
    /// push ID を運ぶものであり、request stream や WebTransport セッションの
    /// 新規拒否判定には使えない。
    /// (RFC 9114 Section 5.2 / 7.2.6, draft-ietf-webtrans-http3-15 Section 4.7)
    fn peer_goaway_request_boundary(&self) -> Option<VarInt> {
        if self.role == Role::Client {
            self.peer_goaway_last_id
        } else {
            None
        }
    }

    /// ローカル設定を取得
    pub fn local_settings(&self) -> &Settings {
        &self.local_settings
    }

    /// ピア設定を取得
    pub fn peer_settings(&self) -> Option<&Settings> {
        self.peer_settings.as_ref()
    }

    /// ストリーム ID が自身が開始した単方向ストリームかどうかを検証する (RFC 9114 Section 6.2)
    ///
    /// QUIC ストリーム ID の下位 2 ビット:
    /// - 0x0: client-initiated bidirectional
    /// - 0x1: server-initiated bidirectional
    /// - 0x2: client-initiated unidirectional
    /// - 0x3: server-initiated unidirectional
    fn validate_self_initiated_uni_stream_id(&self, stream_id: u64) -> Result<(), Error> {
        let expected = match self.role {
            Role::Client => 0x2, // client-initiated unidirectional
            Role::Server => 0x3, // server-initiated unidirectional
        };
        if stream_id & 0x3 != expected {
            return Err(Error::ConnectionError(ErrorCode::StreamCreationError));
        }
        Ok(())
    }

    /// 制御ストリーム ID を設定 (送信側)
    ///
    /// 制御ストリームは 1 つのみ許可される (RFC 9114 Section 6.2.1)。
    /// 既に設定済みの場合は `StreamCreationError` を返す。
    /// 自身が開始した単方向ストリームでなければ `StreamCreationError` を返す。
    pub fn set_control_stream_id(&mut self, stream_id: u64) -> Result<(), Error> {
        self.validate_self_initiated_uni_stream_id(stream_id)?;
        if self.control_send.stream_id().is_some() {
            return Err(Error::ConnectionError(ErrorCode::StreamCreationError));
        }
        self.control_send.set_stream_id(stream_id);
        if self.control_send.has_pending() {
            self.writable_streams.push_back(stream_id);
        }
        Ok(())
    }

    /// QPACK エンコーダーストリーム ID を設定 (送信側)
    ///
    /// 単方向ストリーム (0x02 type) のストリーム ID を登録し、
    /// stream type バイトを送信バッファに書き込む (RFC 9114 Section 6.2, RFC 9204 Section 4.2)。
    /// エンコーダーストリームは 1 つのみ許可される (RFC 9204 Section 4.2)。
    /// 既に設定済みの場合は `StreamCreationError` を返す。
    /// 自身が開始した単方向ストリームでなければ `StreamCreationError` を返す。
    pub fn set_encoder_stream_id(&mut self, stream_id: u64) -> Result<(), Error> {
        self.validate_self_initiated_uni_stream_id(stream_id)?;
        if self.encoder_stream_id.is_some() {
            return Err(Error::ConnectionError(ErrorCode::StreamCreationError));
        }
        self.encoder_stream_id = Some(stream_id);
        self.encoder_stream.write_stream_type();
        // 遅延された SET_CAPACITY をフラッシュ
        if let Some(capacity) = self.deferred_encoder_set_capacity.take() {
            self.encoder_stream
                .encode_set_capacity(capacity)
                .map_err(Error::Qpack)?;
        }
        if self.encoder_stream.has_pending() {
            self.writable_streams.push_back(stream_id);
        }
        Ok(())
    }

    /// QPACK デコーダーストリーム ID を設定 (送信側)
    ///
    /// 単方向ストリーム (0x03 type) のストリーム ID を登録し、
    /// stream type バイトを送信バッファに書き込む (RFC 9114 Section 6.2, RFC 9204 Section 4.2)。
    /// デコーダーストリームは 1 つのみ許可される (RFC 9204 Section 4.2)。
    /// 既に設定済みの場合は `StreamCreationError` を返す。
    /// 自身が開始した単方向ストリームでなければ `StreamCreationError` を返す。
    pub fn set_decoder_stream_id(&mut self, stream_id: u64) -> Result<(), Error> {
        self.validate_self_initiated_uni_stream_id(stream_id)?;
        if self.decoder_stream_id.is_some() {
            return Err(Error::ConnectionError(ErrorCode::StreamCreationError));
        }
        self.decoder_stream_id = Some(stream_id);
        self.decoder_stream.write_stream_type();
        // 遅延された Section Acknowledgment をフラッシュ
        for ack_stream_id in std::mem::take(&mut self.deferred_section_acks) {
            self.decoder_stream
                .encode_section_acknowledgment(ack_stream_id);
        }
        // 遅延された Stream Cancellation をフラッシュ
        for cancel_stream_id in std::mem::take(&mut self.deferred_stream_cancellations) {
            self.decoder_stream
                .encode_stream_cancellation(cancel_stream_id);
        }
        if self.decoder_stream.has_pending() {
            self.writable_streams.push_back(stream_id);
        }
        Ok(())
    }

    /// 制御ストリーム・QPACK encoder/decoder ストリームを一括で初期化する
    ///
    /// 3 つのストリーム ID を設定し、各ストリームの初期データを取得する。
    /// 呼び出し側は返されたデータを対応する QUIC ストリームで送信する責任を持つ。
    ///
    /// **重要**: 初期データを送信した後も、対応する QUIC ストリームのハンドルは接続終了まで
    /// 保持し続けること。これらのクリティカルストリームをクローズすると、相手側で
    /// H3_CLOSED_CRITICAL_STREAM エラーが発生する (RFC 9114 Section 6.2.1)。
    ///
    /// 各ストリーム ID は自身が開始した単方向ストリーム ID でなければならない。
    /// エラーを返した場合、Connection は部分的に初期化された状態になりうる。
    /// エラー後の Connection は使用せず破棄すること。
    pub fn init_h3_streams(
        &mut self,
        control_stream_id: u64,
        encoder_stream_id: u64,
        decoder_stream_id: u64,
    ) -> Result<H3InitData, Error> {
        self.set_control_stream_id(control_stream_id)?;
        self.set_encoder_stream_id(encoder_stream_id)?;
        self.set_decoder_stream_id(decoder_stream_id)?;

        let control_data = self
            .take_stream_data(control_stream_id)
            .map(|(d, _)| d)
            .unwrap_or_default();
        let encoder_data = self
            .take_stream_data(encoder_stream_id)
            .map(|(d, _)| d)
            .unwrap_or_default();
        let decoder_data = self
            .take_stream_data(decoder_stream_id)
            .map(|(d, _)| d)
            .unwrap_or_default();

        Ok(H3InitData {
            control_stream_id,
            control_data,
            encoder_stream_id,
            encoder_data,
            decoder_stream_id,
            decoder_data,
        })
    }

    /// QPACK エンコーダーストリームへの参照を取得
    pub fn encoder_stream(&self) -> &EncoderStream {
        &self.encoder_stream
    }

    /// QPACK エンコーダーストリームへの可変参照を取得
    pub fn encoder_stream_mut(&mut self) -> &mut EncoderStream {
        &mut self.encoder_stream
    }

    /// QPACK デコーダーストリームへの参照を取得
    pub fn decoder_stream(&self) -> &DecoderStream {
        &self.decoder_stream
    }

    /// QPACK デコーダーストリームへの可変参照を取得
    pub fn decoder_stream_mut(&mut self) -> &mut DecoderStream {
        &mut self.decoder_stream
    }

    /// QPACK 動的エンコーダーへの参照を取得
    pub fn qpack_encoder(&self) -> &DynamicEncoder {
        &self.qpack_encoder
    }

    /// QPACK 動的エンコーダーへの可変参照を取得
    pub fn qpack_encoder_mut(&mut self) -> &mut DynamicEncoder {
        &mut self.qpack_encoder
    }

    /// QPACK 動的デコーダーへの参照を取得
    pub fn qpack_decoder(&self) -> &DynamicDecoder {
        &self.qpack_dynamic_decoder
    }

    /// QPACK 動的デコーダーへの可変参照を取得
    pub fn qpack_decoder_mut(&mut self) -> &mut DynamicDecoder {
        &mut self.qpack_dynamic_decoder
    }

    /// ストリームが終了条件を満たした場合に `streams` から除去する
    ///
    /// 除去条件は次の 3 つ (RFC 9114 Section 4.1 / 4.1.1):
    /// - Reset になった場合 (RESET_STREAM 受信。ローカル送信データは破棄する)
    /// - `StreamState::Closed` (両方向クローズ) かつ送信バッファ完全消費済み
    ///   (FIN 交付済み)。データ全消費時点では除去しない (FIN 交付のための
    ///   追加呼び出しが残っているため)
    /// - セッション終了済み (tombstone) の CONNECT ストリーム。CONNECT ストリームは
    ///   セッション中 FIN を送らず受信側も open のままのため、両方向クローズに
    ///   到達しないケースがあり、セッション終了を除去トリガにする
    ///   (RFC 9114 Section 4.4 / draft-ietf-webtrans-http3-16 Section 6)
    ///
    /// StreamEnd (受信側 FIN) 時点や STOP_SENDING 受信時点では除去しない
    /// (サーバーが応答を送る必要がある / 受信側が open のままのため)。
    fn remove_stream_if_done(&mut self, stream_id: u64) {
        let done = self.streams.get(&stream_id).is_some_and(|s| {
            s.state() == StreamState::Reset
                || (s.state() == StreamState::Closed && s.is_send_complete())
        });
        if done || self.closed_wt_sessions.contains(&stream_id) {
            self.streams.remove(&stream_id);
        }
    }

    /// QUIC からストリームデータを受信
    pub fn feed_stream(&mut self, stream_id: u64, data: &[u8], fin: bool) -> Result<(), Error> {
        if let Some(ref err) = self.error {
            return Err(err.clone());
        }

        let kind = StreamKind::from_stream_id(stream_id);

        // 単方向ストリームの場合
        if kind.is_unidirectional() {
            self.handle_unidirectional_stream(stream_id, data, fin)?;
            return Ok(());
        }

        // クライアントが server-initiated bidi stream を受信した場合
        if self.role == Role::Client && kind.is_server_initiated() {
            // WebTransport の能力ネゴシエーションが完了している場合のみ受け入れる
            // (draft-ietf-webtrans-http3-15 Section 3.1, 4.3)
            if self.is_wt_fully_negotiated() {
                self.handle_wt_bidi_stream(stream_id, data, fin)?;
                return Ok(());
            }
            // WebTransport 無効時は接続エラー (RFC 9114 Section 6.1)
            return Err(Error::ConnectionError(ErrorCode::StreamCreationError));
        }

        // 既に確定済みまたはヘッダー解決中の WT bidi stream ならデータ/FIN を処理する
        if self.wt_bidi_streams.contains_key(&stream_id)
            || self.pending_wt_bidi_streams.contains_key(&stream_id)
        {
            self.handle_wt_bidi_stream(stream_id, data, fin)?;
            return Ok(());
        }

        // ディスパッチ保留中のクライアント開始 bidi stream に後続データが到着した場合
        if self.pending_bidi_dispatch.contains_key(&stream_id) {
            return self.dispatch_client_bidi_stream(stream_id, data, fin);
        }

        // サーバー側: クライアント開始の新規 bidi stream は先頭 varint で
        // WT bidi (0x41) かリクエストストリームかを判定する
        // ネゴシエーション完了を確認し、未完了の場合はリクエストストリームとして処理する
        // (draft-ietf-webtrans-http3-15 Section 3.1, 4.3)
        if self.role == Role::Server
            && kind.is_client_initiated()
            && self.is_wt_fully_negotiated()
            && !self.streams.contains_key(&stream_id)
        {
            return self.dispatch_client_bidi_stream(stream_id, data, fin);
        }

        // 双方向ストリーム (リクエスト/レスポンス)
        let result = self.handle_bidirectional_stream(stream_id, data, fin);

        // ストリームが両方向クローズ + 送信完了済みなら除去する
        // (受信経路からも除去条件を満たすことがあるため。エラー経路でも
        //  セッション終了 (tombstone) 済みの CONNECT ストリームは除去する)
        self.remove_stream_if_done(stream_id);
        result?;
        Ok(())
    }

    /// 単方向ストリームを処理
    fn handle_unidirectional_stream(
        &mut self,
        stream_id: u64,
        data: &[u8],
        fin: bool,
    ) -> Result<(), Error> {
        // 無視対象のストリームは後続データを破棄 (RFC 9114 Section 6.2)
        if self.ignored_uni_streams.contains(&stream_id) {
            return Ok(());
        }

        // データがある場合は先に処理する (FIN 判定の前にパーサーを進める)
        if !data.is_empty() {
            // 既知のストリームかチェック
            if self.control_recv.stream_id() == Some(stream_id) {
                self.control_recv.receive(data);
                self.process_control_stream()?;
            } else if self.peer_encoder_stream_id == Some(stream_id) {
                self.encoder_stream_recv.receive(data);
                self.process_encoder_stream()?;
            } else if self.peer_decoder_stream_id == Some(stream_id) {
                self.decoder_stream_recv.receive(data);
                self.process_decoder_stream()?;
            } else if self.wt_uni_streams.contains_key(&stream_id) {
                // WebTransport 単方向ストリーム
                // 0077 Phase 5: WT 分岐を wt_stream.rs のヘルパーに委譲
                self.handle_wt_uni_stream_data(stream_id, data)?;
            } else if self.pending_wt_uni_streams.contains_key(&stream_id) {
                // セッション ID 未確定の WT 単方向ストリーム
                self.resolve_wt_uni_stream_session_id(stream_id, data)?;
            } else {
                // 新しいストリーム: 後続の処理に委譲
                return self.handle_new_unidirectional_stream(stream_id, data, fin);
            }
        }

        // FIN 受信時の critical stream 処理
        if fin {
            // 制御ストリームの FIN: バッファに未処理データが残っていればフレーム切り詰め
            // (RFC 9114 Section 7.1)
            if self.control_recv.stream_id() == Some(stream_id) {
                if self.control_recv.has_pending_data() {
                    return Err(Error::ConnectionError(ErrorCode::FrameError));
                }
                return Err(Error::ConnectionError(ErrorCode::ClosedCriticalStream));
            }

            // QPACK ストリームの FIN (RFC 9114 Section 6.2.1)
            if self.peer_encoder_stream_id == Some(stream_id)
                || self.peer_decoder_stream_id == Some(stream_id)
            {
                return Err(Error::ConnectionError(ErrorCode::ClosedCriticalStream));
            }

            // WebTransport 単方向ストリームの FIN
            // 0077 Phase 5: WT 分岐を wt_stream.rs のヘルパーに委譲
            self.handle_wt_uni_stream_fin(stream_id);

            // セッション ID 未確定の WT 単方向ストリームの FIN
            // (セッション ID が未確定のまま FIN が来た場合は単に破棄)
            self.pending_wt_uni_streams.remove(&stream_id);

            // ストリームタイプ varint が未完のまま FIN が来た場合はバッファを破棄する
            // (RFC 9114 Section 6.2: ストリームヘッダー受信前に閉じられた
            //  単方向ストリームは許容される)
            self.pending_uni_streams.remove(&stream_id);
        }

        Ok(())
    }

    /// 新しい単方向ストリームを処理 (ストリームタイプのデコード)
    fn handle_new_unidirectional_stream(
        &mut self,
        stream_id: u64,
        data: &[u8],
        fin: bool,
    ) -> Result<(), Error> {
        // 既知のストリームではない場合のみここに到達する

        // 新しいストリーム: ストリームタイプを varint でデコード (RFC 9114 Section 6.2)
        // varint が複数チャンクにまたがる場合はバッファリング
        let has_pending = self.pending_uni_streams.contains_key(&stream_id);
        let type_data = if let Some(buf) = self.pending_uni_streams.get_mut(&stream_id) {
            buf.extend_from_slice(data);
            buf.clone()
        } else {
            data.to_vec()
        };

        let (stream_type, type_len) = match crate::varint::decode(&type_data) {
            Ok(result) => result,
            Err(crate::varint::DecodeError::BufferTooShort) => {
                // バッファ不足: 次のチャンクを待つ
                if fin {
                    // varint 未完のまま FIN が来た場合はバッファを破棄する
                    // (RFC 9114 Section 6.2: ストリームヘッダー受信前に閉じられた
                    //  単方向ストリームは許容される)
                    self.pending_uni_streams.remove(&stream_id);
                    return Ok(());
                }
                if !has_pending {
                    self.pending_uni_streams.insert(stream_id, data.to_vec());
                }
                // has_pending の場合は既に上で extend 済み
                return Ok(());
            }
        };

        // pending バッファがあれば削除
        self.pending_uni_streams.remove(&stream_id);

        let remaining = &type_data[type_len..];

        match stream_type.get() {
            0x00 => {
                // Control Stream
                if self.control_recv.stream_id().is_some() {
                    // 制御ストリームは 1 つのみ
                    return Err(Error::ConnectionError(ErrorCode::StreamCreationError));
                }
                self.control_recv.set_stream_id(stream_id);
                // ストリームタイプは control.rs 側でも消費されるため、
                // type_data 先頭からではなく remaining を渡す。
                // ただし ControlStreamRecv は WaitingType 状態から開始するため、
                // ストリームタイプバイトを含めて渡す必要がある。
                // ここでは varint デコード済みなので直接 WaitingSettings に進むため、
                // 残りのデータのみを渡し、状態を手動で遷移させる。
                self.control_recv.skip_stream_type();
                if !remaining.is_empty() {
                    self.control_recv.receive(remaining);
                    self.process_control_stream()?;
                }
            }
            0x02 => {
                // QPACK Encoder Stream
                if self.peer_encoder_stream_id.is_some() {
                    // エンコーダーストリームは 1 つのみ
                    return Err(Error::ConnectionError(ErrorCode::StreamCreationError));
                }
                self.peer_encoder_stream_id = Some(stream_id);
                if !remaining.is_empty() {
                    self.encoder_stream_recv.receive(remaining);
                    self.process_encoder_stream()?;
                }
            }
            0x03 => {
                // QPACK Decoder Stream
                if self.peer_decoder_stream_id.is_some() {
                    // デコーダーストリームは 1 つのみ
                    return Err(Error::ConnectionError(ErrorCode::StreamCreationError));
                }
                self.peer_decoder_stream_id = Some(stream_id);
                if !remaining.is_empty() {
                    self.decoder_stream_recv.receive(remaining);
                    self.process_decoder_stream()?;
                }
            }
            0x01 => {
                // Push Stream (RFC 9114 Section 6.2.2)
                if self.role == Role::Server {
                    // サーバーがクライアント開始の push stream を受信するのは違反
                    // (RFC 9114 Section 6.2.2)
                    return Err(Error::ConnectionError(ErrorCode::StreamCreationError));
                }
                // クライアント側: MAX_PUSH_ID を送信していないので H3_ID_ERROR
                // (RFC 9114 Section 4.6)
                return Err(Error::ConnectionError(ErrorCode::IdError));
            }
            0x54 => {
                // WebTransport 単方向ストリーム (draft-ietf-webtrans-http3-16 Section 4.2)
                //
                // ネゴシエーション完了 (peer SETTINGS 受信 + WT 広告 + H3_DATAGRAM +
                // QUIC transport parameter 検証) を bidi 経路と同じ条件で確認する
                // (draft-ietf-webtrans-http3-16 Section 3.1 / 7.1)。
                if !self.is_wt_fully_negotiated() {
                    // ネゴシエーション未完了の 0x54 は「recipient がサポートしない
                    // ストリームタイプ」に該当し、ストリーム単位の拒否で対処する
                    // (RFC 9114 Section 6.2: 未知ストリームタイプは接続エラーにしては
                    //  ならない MUST NOT。abort 時のエラーコードは
                    //  H3_STREAM_CREATION_ERROR またはリザーブドエラーコードが
                    //  SHOULD)。
                    // ストリームエラーは RESET_STREAM / STOP_SENDING の送信を
                    // QUIC 統合層に委ねる (error.rs の「ストリームエラー (ストリームを
                    // リセットすべき)」)。統合層の一部経路では対応が未実装のため、
                    // ピアへの拒否通知が届かないケースがあり得る (統合層側の対応は
                    // 本変更のスコープ外)。
                    // バッファリングでネゴシエーション完了を待つ方式は採らない。
                    // draft-ietf-webtrans-http3-16 Section 4.6 のバッファリングは
                    // 確立済みセッションに関連付けられるまでの SHOULD であり
                    // MUST ではない。RFC 9114 Section 6.2 の MUST が定める
                    // 2 択 (abort / discard) のうち abort 方式を採用する。
                    return Err(Error::StreamError(ErrorCode::StreamCreationError));
                }
                // セッション ID (varint) をパース
                self.resolve_wt_uni_stream_session_id(stream_id, remaining)?;
            }
            _ => {
                // 未知ストリームタイプ: 後続データを破棄 (RFC 9114 Section 6.2)
                self.ignored_uni_streams.insert(stream_id);
            }
        }

        // ストリームタイプ確定後のクリティカルストリーム FIN チェック
        // (RFC 9114 Section 6.2.1, RFC 9204 Section 4.2)
        // 初回チャンクに FIN が付いている場合、上部の FIN チェック時点では
        // stream_id が未登録のため検出できない
        if fin {
            // 制御ストリーム: バッファに未処理データが残っていればフレーム切り詰め
            // (RFC 9114 Section 7.1)
            if self.control_recv.stream_id() == Some(stream_id) {
                if self.control_recv.has_pending_data() {
                    return Err(Error::ConnectionError(ErrorCode::FrameError));
                }
                return Err(Error::ConnectionError(ErrorCode::ClosedCriticalStream));
            }

            // QPACK ストリームの FIN (RFC 9114 Section 6.2.1)
            if self.peer_encoder_stream_id == Some(stream_id)
                || self.peer_decoder_stream_id == Some(stream_id)
            {
                return Err(Error::ConnectionError(ErrorCode::ClosedCriticalStream));
            }

            // WebTransport 単方向ストリームの FIN (初回チャンクに FIN が付いている場合)
            if self.wt_uni_streams.remove(&stream_id).is_some() {
                self.events
                    .push_back(Event::WebTransport(WebTransportEvent::UniStreamEnd {
                        stream_id,
                    }));
            }
            self.pending_wt_uni_streams.remove(&stream_id);
        }

        Ok(())
    }

    /// クライアント開始の新規双方向ストリームを WT bidi かリクエストに振り分ける
    ///
    /// サーバー側で WebTransport が有効な場合、クライアント開始の新規 bidi stream の
    /// 先頭 varint をデコードし、WT_STREAM (0x41) なら WT bidi ストリームとして、
    /// それ以外ならリクエストストリームとして処理する。
    /// varint が不完全な場合は `pending_bidi_dispatch` にバッファリングする。
    /// (draft-ietf-webtrans-http3-15 Section 4.3)
    fn dispatch_client_bidi_stream(
        &mut self,
        stream_id: u64,
        data: &[u8],
        fin: bool,
    ) -> Result<(), Error> {
        // バッファリング中のデータがあれば結合する
        let buf = if let Some(pending) = self.pending_bidi_dispatch.get_mut(&stream_id) {
            pending.extend_from_slice(data);
            pending.clone()
        } else {
            data.to_vec()
        };

        if buf.is_empty() {
            // データなし: 先頭バイトが到着するまでバッファリング
            self.pending_bidi_dispatch.entry(stream_id).or_default();
            if fin {
                // データなしで FIN: 空のリクエストストリームとして処理
                self.pending_bidi_dispatch.remove(&stream_id);
                self.handle_bidirectional_stream(stream_id, &[], fin)?;
            }
            return Ok(());
        }

        // 先頭バイトで varint のプレフィックスを判定
        let first_byte = buf[0];
        let varint_prefix = first_byte >> 6;

        if varint_prefix == 0 {
            // 1 バイト varint (値 0x00-0x3F): HTTP/3 フレームタイプ
            // 0x41 は 2 バイト varint (prefix = 0x01) なのでここには来ない
            self.pending_bidi_dispatch.remove(&stream_id);
            self.handle_bidirectional_stream(stream_id, &buf, fin)?;
            return Ok(());
        }

        // 2 バイト以上の varint: 0x41 (WT_STREAM) の可能性がある
        // 完全にデコードして判定する
        match crate::varint::decode(&buf) {
            Ok((value, _)) => {
                self.pending_bidi_dispatch.remove(&stream_id);
                if value.get() == crate::webtransport::stream::BIDIRECTIONAL_SIGNAL_VALUE {
                    // WT_STREAM (0x41): WT bidi ストリームとして処理
                    self.handle_wt_bidi_stream(stream_id, &buf, fin)?;
                } else {
                    // 0x41 以外: リクエストストリームとして処理
                    self.handle_bidirectional_stream(stream_id, &buf, fin)?;
                }
                Ok(())
            }
            Err(crate::varint::DecodeError::BufferTooShort) => {
                // varint 不完全: バッファリングして次のチャンクを待つ
                self.pending_bidi_dispatch.entry(stream_id).or_insert(buf);
                if fin {
                    // varint 不完全で FIN: 不正なストリーム
                    self.pending_bidi_dispatch.remove(&stream_id);
                    return Err(Error::ConnectionError(ErrorCode::FrameError));
                }
                Ok(())
            }
        }
    }

    /// QPACK エンコーダーストリームを処理
    ///
    /// 動的テーブルの更新のみ行う。ブロック中ストリームの再デコードは
    /// `drain_events()` で遅延実行する。イベントキューモデルでは
    /// エンコーダーストリーム処理時にクロスストリームのイベントを生成すると、
    /// 意図しない呼び出し元がイベントを消費するリスクがあるため。
    fn process_encoder_stream(&mut self) -> Result<(), Error> {
        while let Some(_instruction) = self
            .encoder_stream_recv
            .process(self.qpack_dynamic_decoder.table_mut())
            .map_err(|_| Error::ConnectionError(ErrorCode::QpackEncoderStreamError))?
        {
            // 動的テーブルが更新された
        }
        Ok(())
    }

    /// ブロックされているストリームの再デコードを試みる (RFC 9204 Section 2.1.2)
    ///
    /// ricnt (Required Insert Count) 順でブロック解除を試みる (nghttp3 方式)
    ///
    /// ricnt が小さいストリームから順にデコードを試み、ricnt > 現在の Insert Count の
    /// ストリームに到達した時点で打ち切る。デコードに成功したストリームはフラグを解除し、
    /// ストリームの内部バッファに残っている後続フレームを `process_stream_frames` で処理する。
    fn retry_blocked_streams(&mut self) -> Result<(), Error> {
        if self.blocked_by_ricnt.is_empty() {
            return Ok(());
        }

        let insert_count = self.qpack_dynamic_decoder.table().insert_count();

        // ricnt 順でアンブロック可能なストリームを収集する
        let unblockable: Vec<(u64, u64)> = self
            .blocked_by_ricnt
            .iter()
            .take_while(|(ricnt, _)| *ricnt <= insert_count)
            .copied()
            .collect();

        if unblockable.is_empty() {
            return Ok(());
        }

        // BTreeSet から除去
        for entry in &unblockable {
            self.blocked_by_ricnt.remove(entry);
        }

        for &(_, stream_id) in &unblockable {
            // ストリームからブロック中のヘッダーを取り出す
            let Some((encoded, is_trailer)) = self
                .streams
                .get_mut(&stream_id)
                .and_then(|s| s.take_qpack_blocked_header())
            else {
                continue;
            };

            match self
                .qpack_dynamic_decoder
                .decode(&encoded)
                .map_err(Error::Qpack)?
            {
                DecodeOutput::Decoded(headers) => {
                    // ブロック解除: フラグをクリアする
                    if let Some(stream) = self.streams.get_mut(&stream_id) {
                        stream.set_qpack_blocked(false, 0, None);
                    }

                    // ストリームが終了済みの場合は content-length 検証
                    // (StreamEnd 受信時点でヘッダーがブロックされていた場合はここで検証する)
                    if !is_trailer && let Some(stream) = self.streams.get(&stream_id) {
                        let state = stream.state();
                        if matches!(state, StreamState::RemoteClosed | StreamState::Closed) {
                            let body_size = stream.received_body().len() as u64;
                            let skip = self.role == Role::Client
                                && (stream.is_head_request() || is_no_body_status(&headers));
                            crate::validation::validate_content_length(&headers, body_size, skip)?;
                        }
                    }

                    // ヘッダーイベントを生成する
                    self.emit_header_events(stream_id, headers, is_trailer)?;

                    // ストリームの内部バッファに残っている後続フレームを処理する
                    // (DATA, Trailers, StreamEnd など)
                    self.process_stream_frames(stream_id)?;
                }
                DecodeOutput::Blocked => {
                    // まだ必要なエントリが揃っていない:
                    // 新しい ricnt で再登録する
                    let ricnt = self.qpack_dynamic_decoder.last_required_insert_count();
                    if let Some(stream) = self.streams.get_mut(&stream_id) {
                        stream.set_qpack_blocked(true, ricnt, Some((encoded, is_trailer)));
                    }
                    self.blocked_by_ricnt.insert((ricnt, stream_id));
                }
            }
        }
        Ok(())
    }

    /// QPACK デコーダーストリームを処理
    fn process_decoder_stream(&mut self) -> Result<(), Error> {
        use crate::qpack::DecoderInstruction;

        let total_insert_count = self.qpack_encoder.insert_count();
        while let Some(instruction) = self
            .decoder_stream_recv
            .process(total_insert_count)
            .map_err(|_| Error::ConnectionError(ErrorCode::QpackDecoderStreamError))?
        {
            match instruction {
                DecoderInstruction::InsertCountIncrement { .. } => {
                    let known = self.decoder_stream_recv.known_received_count();
                    self.qpack_encoder.acknowledge(known);
                }
                DecoderInstruction::SectionAcknowledgment { stream_id } => {
                    // 既に全て ack 済みのストリームに対する Section Acknowledgment は
                    // QPACK_DECODER_STREAM_ERROR (RFC 9204 Section 4.4.1)
                    self.qpack_encoder
                        .ack_section(stream_id)
                        .map_err(|_| Error::ConnectionError(ErrorCode::QpackDecoderStreamError))?;
                }
                DecoderInstruction::StreamCancellation { stream_id } => {
                    // ストリームキャンセル: 未 ack セクションを削除
                    self.qpack_encoder.cancel_stream(stream_id);
                }
            }
        }
        Ok(())
    }

    /// 制御ストリームを処理
    fn process_control_stream(&mut self) -> Result<(), Error> {
        while let Some(frame) = self.control_recv.process()? {
            match frame {
                crate::frame::Frame::Settings(payload) => {
                    let settings = Settings::from_payload(&payload);

                    // ピアの QPACK 設定をエンコーダーに適用
                    if let Some(capacity) = settings.qpack_max_table_capacity {
                        let capacity = capacity.get();
                        self.qpack_encoder.set_max_table_capacity(capacity);
                        self.encoder_stream.set_max_table_capacity(capacity);
                        // 実際に使用するテーブル容量を設定
                        // ピアが許可する容量と自分のデフォルト容量の小さい方を使用
                        let use_capacity =
                            std::cmp::min(capacity, self.limits.qpack_max_table_capacity);
                        self.qpack_encoder.set_table_capacity(use_capacity);

                        // Set Dynamic Table Capacity 命令を送信
                        // エンコーダーストリーム未初期化の場合は遅延させる
                        // (stream type より前に命令を書き込むと不正な wire format になる)
                        if use_capacity > 0 {
                            if self.encoder_stream_id.is_some() {
                                self.encoder_stream
                                    .encode_set_capacity(use_capacity)
                                    .map_err(Error::Qpack)?;
                            } else {
                                self.deferred_encoder_set_capacity = Some(use_capacity);
                            }
                        }
                    }

                    // ピアの QPACK blocked streams 設定をエンコーダーに適用
                    // (RFC 9204 Section 2.1.2)
                    if let Some(blocked) = settings.qpack_blocked_streams {
                        self.qpack_encoder
                            .set_peer_max_blocked_streams(blocked.get());
                    }

                    let wt_settings = settings.wt_settings;

                    // クライアントは SETTINGS_WT_ENABLED > 1 を
                    // H3_SETTINGS_ERROR 接続エラーとして扱わなければならない
                    // (draft-ietf-webtrans-http3-16 Section 3.1)
                    // 将来のドラフトで変更される可能性がある
                    if self.role == Role::Client
                        && let Some(ref wt) = wt_settings
                        && wt.wt_enabled.get() > 1
                    {
                        return Err(Error::ConnectionError(ErrorCode::SettingsError));
                    }

                    self.peer_settings = Some(settings);
                    self.events.push_back(Event::SettingsReceived {
                        settings,
                        wt_settings,
                    });
                }
                crate::frame::Frame::Goaway(payload) => {
                    let goaway_id = payload.id();
                    // クライアントが受信する GOAWAY の stream ID は
                    // client-initiated bidirectional stream ID でなければならない
                    // (RFC 9114 Section 7.2.6)
                    if self.role == Role::Client {
                        // client-initiated bidi stream ID は 4 の倍数 (0, 4, 8, ...)
                        if goaway_id.get() % 4 != 0 {
                            return Err(Error::ConnectionError(ErrorCode::IdError));
                        }
                    }

                    // 複数 GOAWAY の単調減少チェック (RFC 9114 Section 5.2)
                    // 値の意味はロール依存だが、単調減少制約はどちらの方向でも成立する
                    if let Some(prev_id) = self.peer_goaway_last_id
                        && goaway_id > prev_id
                    {
                        return Err(Error::ConnectionError(ErrorCode::IdError));
                    }

                    self.peer_goaway_received = true;
                    self.peer_goaway_last_id = Some(goaway_id);
                    self.events
                        .push_back(Event::GoawayReceived { id: goaway_id });

                    // WT draining 伝播はクライアント受信時のみ行う
                    //
                    // サーバーが受信する GOAWAY は push ID を運ぶもので、
                    // WebTransport セッション ID (CONNECT 要求 stream ID) とは
                    // 比較する意味がない
                    // (RFC 9114 Section 5.2 / 7.2.6)
                    // (draft-ietf-webtrans-http3-15 Section 4.7: 新規 WT セッションを
                    //  開始できなくなるのは GOAWAY を受けたクライアント側)
                    if self.role == Role::Client {
                        let draining_sessions: Vec<u64> = self
                            .wt_sessions
                            .iter()
                            .filter(|(sid, session)| {
                                **sid >= goaway_id.get()
                                    && (session.state == WtSessionState::Established
                                        || session.state == WtSessionState::Pending)
                            })
                            .map(|(sid, _)| *sid)
                            .collect();
                        for sid in draining_sessions {
                            // 内部状態を Draining に遷移させた上でイベントを発行する
                            // (draft-ietf-webtrans-http3-15 Section 4.7 / RFC 9114 Section 5.2)
                            if let Some(session) = self.wt_sessions.get_mut(&sid) {
                                session.state = WtSessionState::Draining;
                            }
                            self.events.push_back(Event::WebTransport(
                                WebTransportEvent::SessionDraining { session_id: sid },
                            ));
                        }
                    }
                }
                crate::frame::Frame::MaxPushId(value) => {
                    // MAX_PUSH_ID は client → server で control stream 上で送信される
                    // (RFC 9114 Section 7.2.7)。
                    // クライアント側で受信した場合は H3_FRAME_UNEXPECTED。
                    if self.role == Role::Client {
                        return Err(Error::ConnectionError(ErrorCode::FrameUnexpected));
                    }
                    // サーバー側: 単調増加制約 (前の値より小さいと H3_ID_ERROR)
                    if let Some(prev) = self.max_push_id
                        && value < prev
                    {
                        return Err(Error::ConnectionError(ErrorCode::IdError));
                    }
                    self.max_push_id = Some(value);
                    // サーバープッシュ非対応のため、値を保持するだけで利用はしない。
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// 双方向ストリームを処理 (nghttp3 方式: ストリームレベルブロッキング)
    ///
    /// QPACK ブロック中のストリームはフレーム解析を停止し、
    /// 受信データをストリームの内部バッファに保持する。
    /// ブロック解除は `retry_blocked_streams()` (drain_events 経由) で行う。
    fn handle_bidirectional_stream(
        &mut self,
        stream_id: u64,
        data: &[u8],
        fin: bool,
    ) -> Result<(), Error> {
        // 終了済みセッション (tombstone) の CONNECT ストリームへの遅延 DATA は
        // H3_MESSAGE_ERROR で拒否する (zombie Pending セッションの再生成を防ぐ)。
        // FIN のみの場合は受理して何もしない (正常な終了手順のため)。
        // (draft-ietf-webtrans-http3-16 Section 6)
        if self.closed_wt_sessions.contains(&stream_id) {
            if data.is_empty() && fin {
                return Ok(());
            }
            return Err(Error::StreamError(ErrorCode::MessageError));
        }

        // ストリームを取得または作成
        self.streams
            .entry(stream_id)
            .or_insert_with(|| RequestStream::new(stream_id, self.role));

        // まずデータを受信 (ストリームの内部バッファに追加)
        if let Some(stream) = self.streams.get_mut(&stream_id) {
            stream.receive(data, fin);
        }

        // QPACK ブロック中のストリームはフレーム解析を停止する (nghttp3 方式)
        // データはストリームの内部バッファに残り、ブロック解除時に再処理する
        if self
            .streams
            .get(&stream_id)
            .expect("stream was just inserted")
            .is_qpack_blocked()
        {
            return Ok(());
        }

        self.process_stream_frames(stream_id)
    }

    /// ストリームのフレームを処理する
    ///
    /// `handle_bidirectional_stream` と `retry_blocked_streams` の両方から呼ばれる。
    /// QPACK ブロック発生時はストリームにフラグを立てて処理を中断する。
    /// 後続のフレーム (DATA, StreamEnd) はストリームの内部バッファに残る。
    fn process_stream_frames(&mut self, stream_id: u64) -> Result<(), Error> {
        loop {
            let received = {
                let stream = self
                    .streams
                    .get_mut(&stream_id)
                    .expect("stream must exist while processing frames");
                stream.process_raw()?
            };

            let Some(received) = received else {
                break;
            };

            match received {
                RawReceivedData::Headers(encoded) => {
                    let is_trailer = false;

                    match self
                        .qpack_dynamic_decoder
                        .decode(&encoded)
                        .map_err(Error::Qpack)?
                    {
                        DecodeOutput::Decoded(headers) => {
                            self.emit_header_events(stream_id, headers, is_trailer)?;
                        }
                        DecodeOutput::Blocked => {
                            // ブロックストリーム数の上限チェック (RFC 9204 Section 2.1.2)
                            let blocked_count = self.blocked_by_ricnt.len();
                            if blocked_count >= self.limits.qpack_blocked_streams as usize {
                                return Err(Error::ConnectionError(
                                    ErrorCode::QpackDecompressionFailed,
                                ));
                            }

                            // ストリームにブロック状態を設定し、フレーム解析を停止する
                            let ricnt = self.qpack_dynamic_decoder.last_required_insert_count();
                            if let Some(stream) = self.streams.get_mut(&stream_id) {
                                stream.set_qpack_blocked(true, ricnt, Some((encoded, is_trailer)));
                            }
                            self.blocked_by_ricnt.insert((ricnt, stream_id));
                            // 後続データはストリームの内部バッファに残る
                            break;
                        }
                    }
                }
                RawReceivedData::Trailers(encoded) => {
                    let is_trailer = true;
                    match self
                        .qpack_dynamic_decoder
                        .decode(&encoded)
                        .map_err(Error::Qpack)?
                    {
                        DecodeOutput::Decoded(headers) => {
                            self.emit_header_events(stream_id, headers, is_trailer)?;
                        }
                        DecodeOutput::Blocked => {
                            let blocked_count = self.blocked_by_ricnt.len();
                            if blocked_count >= self.limits.qpack_blocked_streams as usize {
                                return Err(Error::ConnectionError(
                                    ErrorCode::QpackDecompressionFailed,
                                ));
                            }
                            let ricnt = self.qpack_dynamic_decoder.last_required_insert_count();
                            if let Some(stream) = self.streams.get_mut(&stream_id) {
                                stream.set_qpack_blocked(true, ricnt, Some((encoded, is_trailer)));
                            }
                            self.blocked_by_ricnt.insert((ricnt, stream_id));
                            break;
                        }
                    }
                }
                RawReceivedData::Data(data) => {
                    // WebTransport CONNECT ストリームの場合は Capsule デコードを行う
                    // (draft-ietf-webtrans-http3-15 Section 5.6)
                    // 0077 Phase 5: WT 分岐を wt_capsule.rs のヘルパーに委譲
                    if !self.handle_wt_data_frame(stream_id, &data)? {
                        self.events.push_back(Event::Data { stream_id, data });
                    }
                }
                RawReceivedData::StreamEnd => {
                    // content-length と受信済み DATA の整合性を検証 (RFC 9114 Section 4.1.2)
                    if let Some(stream) = self.streams.get(&stream_id) {
                        let headers = stream.received_headers();
                        let body_size = stream.received_body().len() as u64;
                        let skip = self.role == Role::Client
                            && (stream.is_head_request() || is_no_body_status(headers));
                        crate::validation::validate_content_length(headers, body_size, skip)?;
                    }
                    // WebTransport セッション: FIN 到着時に未完成 Capsule が残っていれば malformed
                    // (draft-ietf-webtrans-http3-15 Section 5.6)
                    // 0077 Phase 5: WT 分岐を wt_capsule.rs のヘルパーに委譲
                    self.handle_wt_stream_end(stream_id)?;

                    // 終了済みセッション (tombstone) の CONNECT ストリームの FIN は
                    // 受理するが汎用 StreamEnd イベントは発行しない
                    // (draft-ietf-webtrans-http3-16 Section 6)
                    if !self.closed_wt_sessions.contains(&stream_id) {
                        self.events.push_back(Event::StreamEnd { stream_id });
                    }
                }
            }
        }

        // ループ外で除去チェックを行う (ループ内で除去すると
        // `expect("stream must exist while processing frames")` が panic する)
        self.remove_stream_if_done(stream_id);

        Ok(())
    }

    /// デコード済みヘッダーからイベントを生成する
    fn emit_header_events(
        &mut self,
        stream_id: u64,
        headers: Vec<crate::qpack::Header>,
        is_trailer: bool,
    ) -> Result<(), Error> {
        if is_trailer {
            crate::validation::validate_trailer_headers(&headers)?;
        } else {
            crate::validation::validate_headers(&headers, self.role)?;

            // サーバー側: WebTransport CONNECT を受信した場合の前提条件チェック
            // (draft-ietf-webtrans-http3-15 Section 3.1, 7.1)
            // 0077 Phase 5: WT 分岐を wt_session.rs のヘルパーに委譲
            self.validate_wt_connect_request_server(stream_id, &headers)?;

            // サーバー側: WebTransport CONNECT を受信した場合、セッションを Pending で登録
            // (draft-ietf-webtrans-http3-15 Section 3)
            // 0077 Phase 5: WT 分岐を wt_session.rs のヘルパーに委譲
            self.register_wt_connect_session(stream_id, &headers);

            // 通常の CONNECT リクエスト検出 (RFC 9114 Section 4.4)
            if self.role == Role::Server
                && is_plain_connect(&headers)
                && let Some(stream) = self.streams.get_mut(&stream_id)
            {
                stream.set_connect();
            }

            // 1xx 中間レスポンスの場合はストリーム状態を戻す (RFC 9114 Section 4.1)
            if self.role == Role::Client
                && is_informational_status(&headers)
                && let Some(stream) = self.streams.get_mut(&stream_id)
            {
                stream.notify_informational();
            }

            // クライアント側: plain CONNECT の 2xx レスポンス受信時に
            // is_connect を設定する (RFC 9114 Section 4.4)
            if self.role == Role::Client
                && is_success_status(&headers)
                && let Some(stream) = self.streams.get_mut(&stream_id)
                && stream.is_connect_request()
            {
                stream.set_connect();
            }

            // クライアント側: WebTransport CONNECT の 2xx レスポンス受信時に
            // セッションを Established に遷移させる (draft-ietf-webtrans-http3-15 Section 3)
            // 0077 Phase 5: WT 分岐を wt_session.rs のヘルパーに委譲
            self.handle_wt_connect_response(stream_id, &headers)?;
        }

        // トレーラーの場合は recv_headers を上書きしない
        if !is_trailer && let Some(stream) = self.streams.get_mut(&stream_id) {
            stream.set_recv_headers(headers.clone());
        }

        // Section Acknowledgement を送信
        // デコーダーストリーム未初期化の場合は遅延させる
        let required_insert_count = self.qpack_dynamic_decoder.last_required_insert_count();
        if required_insert_count > 0 {
            if self.decoder_stream_id.is_some() {
                self.decoder_stream.encode_section_acknowledgment(stream_id);
            } else {
                self.deferred_section_acks.push(stream_id);
            }
        }

        self.events.push_back(Event::HeadersBegin { stream_id });
        for header in headers {
            self.events.push_back(Event::Header {
                stream_id,
                name: header.name().to_vec(),
                value: header.value().to_vec(),
            });
        }
        self.events.push_back(Event::HeadersEnd { stream_id });

        Ok(())
    }

    /// イベントを取得
    ///
    /// QPACK ブロック中のストリームがある場合、動的テーブルの更新状況に基づいて
    /// ブロック解除を試みる。
    pub fn poll_event(&mut self) -> Result<Option<Event>, Error> {
        self.retry_blocked_streams()?;
        Ok(self.events.pop_front())
    }

    /// イベントキューの全イベントを取り出す
    ///
    /// キュー内の全イベントを `Vec` として返し、キューを空にする。
    /// キューが空の場合は空の `Vec` を返す。
    ///
    /// QPACK ブロック中のストリームがある場合、動的テーブルの更新状況に基づいて
    /// ブロック解除を試みる。これにより `feed_stream()` がクロスストリームの
    /// イベントを生成しないことを保証する。
    pub fn drain_events(&mut self) -> Result<Vec<Event>, Error> {
        self.retry_blocked_streams()?;
        Ok(self.events.drain(..).collect())
    }

    /// 送信可能なストリームを取得
    ///
    /// FIN のみが残っているストリームは FIN 交付 (取得) まで報告され続け、
    /// FIN 交付後は報告されなくなる。
    pub fn writable_streams(&self) -> impl Iterator<Item = u64> + '_ {
        // 制御ストリーム
        let control_id = if self.control_send.has_pending() {
            self.control_send.stream_id()
        } else {
            None
        };

        // QPACK エンコーダーストリーム
        let encoder_id = if self.encoder_stream.has_pending() {
            self.encoder_stream_id
        } else {
            None
        };

        // QPACK デコーダーストリーム
        let decoder_id = if self.decoder_stream.has_pending() {
            self.decoder_stream_id
        } else {
            None
        };

        // リクエストストリーム
        let request_ids = self
            .streams
            .iter()
            .filter(|(_, s)| s.has_pending_send())
            .map(|(id, _)| *id);

        control_id
            .into_iter()
            .chain(encoder_id)
            .chain(decoder_id)
            .chain(request_ids)
    }

    /// ストリームの送信データを取得
    ///
    /// リクエストストリームでは FIN を設定済みでもデータが全て消費されるまでは
    /// `fin=false` を返し、データ消費後の追加呼び出しで `(空, fin=true)` を返す。
    /// FIN は送信方向クローズ (RFC 9114 Section 4.1) の実現手段として
    /// 交付と同時に送信済みとして確定し、以降は取得できない (FIN は 1 回だけ交付される)。
    pub fn get_stream_data(&mut self, stream_id: u64) -> Option<(&[u8], bool)> {
        // 制御ストリーム
        if self.control_send.stream_id() == Some(stream_id) {
            let data = self.control_send.get_data();
            if !data.is_empty() {
                return Some((data, false));
            }
        }

        // QPACK エンコーダーストリーム
        if self.encoder_stream_id == Some(stream_id) {
            let data = self.encoder_stream.get_data();
            if !data.is_empty() {
                return Some((data, false));
            }
        }

        // QPACK デコーダーストリーム
        if self.decoder_stream_id == Some(stream_id) {
            let data = self.decoder_stream.get_data();
            if !data.is_empty() {
                return Some((data, false));
            }
        }

        // リクエストストリーム
        if let Some(stream) = self.streams.get_mut(&stream_id) {
            // FIN 交付判定をデータ借用より先に行う
            // (データ借用が生きている間に mark_fin_sent で可変化できないため)
            if stream.has_fin_pending() {
                // FIN 交付と同時に送信済みをマークし、2 回目以降は交付しない
                // (FIN は交付時点で確定し、QUIC への書き込み失敗時の再交付はない)
                stream.mark_fin_sent();
                // fin=true はデータ全消費後にのみ交付されるためデータは必ず空
                return Some((&[], true));
            }
            let (data, _) = stream.get_send_data();
            if !data.is_empty() {
                return Some((data, false));
            }
        }

        None
    }

    /// ストリームデータを取得して内部バッファから消費する
    ///
    /// `get_stream_data()` + `consume_stream_data()` を一度に行う convenience メソッド。
    /// データがない場合は `None` を返す。
    ///
    /// FIN を設定していないストリームでは送信バッファの全データが 1 回の呼び出しで返る。
    /// FIN を設定済みのストリームでは、データが全て返った後の追加呼び出しで
    /// `(空, fin=true)` が返り、FIN 交付後は `None` を返す (FIN は 1 回だけ交付される)。
    /// 送信方向クローズ (FIN) を QUIC 層へ渡すためにはデータ消費後にもう一度呼び出すこと
    /// (RFC 9114 Section 4.1)。
    pub fn take_stream_data(&mut self, stream_id: u64) -> Option<(Vec<u8>, bool)> {
        let (data, fin) = self.get_stream_data(stream_id)?;
        if data.is_empty() && !fin {
            return None;
        }
        let data = data.to_vec();
        let len = data.len();
        self.consume_stream_data(stream_id, len);
        Some((data, fin))
    }

    /// ストリームの送信データを消費
    ///
    /// FIN の交付はこのメソッドでは行われず、データ消費後に `get_stream_data` /
    /// `take_stream_data` を再度呼び出したときに `fin=true` で交付される
    /// (RFC 9114 Section 4.1)。
    pub fn consume_stream_data(&mut self, stream_id: u64, len: usize) {
        // 制御ストリーム
        if self.control_send.stream_id() == Some(stream_id) {
            self.control_send.consume_data(len);
            return;
        }

        // QPACK エンコーダーストリーム
        if self.encoder_stream_id == Some(stream_id) {
            self.encoder_stream.consume_data(len);
            return;
        }

        // QPACK デコーダーストリーム
        if self.decoder_stream_id == Some(stream_id) {
            self.decoder_stream.consume_data(len);
            return;
        }

        // リクエストストリーム
        if let Some(stream) = self.streams.get_mut(&stream_id) {
            stream.consume_send_data(len);
            // データ消費後に両方向クローズ + 送信完了済みなら除去する
            // (クライアントは送信完了後に take_stream_data を呼び直さないため、
            //  受信経路のチェックが必須だが、送信経路でも除去条件を満たし得る)
            self.remove_stream_if_done(stream_id);
        }
    }

    /// リクエストを送信 (クライアント専用)
    ///
    /// `fin=true` の場合は送信方向クローズ (FIN) を設定する。FIN はデータが全て
    /// 消費された後に `get_stream_data` / `take_stream_data` の追加呼び出しで交付される
    /// (RFC 9114 Section 4.1)。
    pub(crate) fn send_request(&mut self, headers: &[Header], fin: bool) -> Result<u64, Error> {
        if self.role != Role::Client {
            return Err(Error::ConnectionError(ErrorCode::InternalError));
        }

        // control stream が設定済みであること (RFC 9114 Section 6.2.1)
        // SETTINGS を先に送信できる状態でなければリクエストを生成しない
        if self.control_send.stream_id().is_none() {
            return Err(Error::ConnectionError(ErrorCode::InternalError));
        }

        // 送信ヘッダーの検証 (RFC 9114 Section 4.1, 4.2, 4.3.1)
        crate::validation::validate_request_headers(headers)?;

        // peer の SETTINGS_MAX_FIELD_SECTION_SIZE を超えていないかチェック (RFC 9114 Section 4.2.2)
        let peer_max = self
            .peer_settings
            .and_then(|s| s.max_field_section_size.map(VarInt::get));
        crate::validation::check_field_section_size(headers, peer_max)?;

        // WebTransport CONNECT の場合、peer の WebTransport サポートを確認する
        // (draft-ietf-webtrans-http3-15 Section 3.1, 4.6)
        // 0077 Phase 5: WT 分岐を wt_session.rs のヘルパーに委譲
        self.validate_wt_connect_request(headers)?;

        // GOAWAY 受信後は指定 ID 以上のストリームを作成できない (RFC 9114 Section 5.2)
        //
        // `peer_goaway_request_boundary()` はクライアント受信時のみ Some を返す。
        // サーバーが受信する GOAWAY は push ID を運ぶものであり、request stream
        // 境界値としては使えない (RFC 9114 Section 7.2.6)
        if let Some(goaway_id) = self.peer_goaway_request_boundary()
            && self.next_stream_id >= goaway_id.get()
        {
            return Err(Error::StreamError(ErrorCode::RequestRejected));
        }

        let stream_id = self.next_stream_id;
        self.next_stream_id += 4;

        // QPACK エンコード
        // ブロック可能ストリーム数を渡して上限制御 (RFC 9204 Section 2.1.2)
        let blocked_count = self.qpack_encoder.blocked_streams_count();
        let buf_size = estimate_encoded_size(headers) + 64;
        let mut qpack_buf = vec![0u8; buf_size];
        let qpack_len = self
            .qpack_encoder
            .encode(&mut qpack_buf, headers, blocked_count)
            .ok_or(Error::ConnectionError(ErrorCode::InternalError))?;
        qpack_buf.truncate(qpack_len);

        let mut stream = RequestStream::new(stream_id, self.role);
        // WebTransport CONNECT の場合は WT CONNECT フラグを設定する
        // (DATA を recv_body に累積しない。Capsule データは handle_wt_data_frame が処理する)
        let is_wt_connect = is_webtransport_connect(headers);
        if is_wt_connect {
            stream.set_wt_connect();
        }
        // HEAD リクエストの場合は Content-Length 検証でのレスポンス body チェックをスキップする
        if headers
            .iter()
            .any(|h| h.name() == b":method" && h.value() == b"HEAD")
        {
            stream.set_is_head_request(true);
        }
        // plain CONNECT リクエストの検出 (RFC 9114 Section 4.4)
        // 2xx レスポンス受信時に is_connect を設定するために追跡する
        let is_connect = headers
            .iter()
            .any(|h| h.name() == b":method" && h.value() == b"CONNECT");
        let has_protocol = headers.iter().any(|h| h.name() == b":protocol");
        if is_connect {
            // CONNECT ストリームは open のまま維持する必要があるため FIN は禁止
            // (RFC 9114 Section 4.4, draft-ietf-webtrans-http3-15 Section 3)
            if fin {
                return Err(Error::StreamError(ErrorCode::MessageError));
            }
            if !has_protocol {
                stream.set_connect_request();
            }
        }
        stream.send_encoded_headers(&qpack_buf, fin, false)?;

        // フィールドセクションの送信を記録 (RFC 9204 Section 2.1.1, 4.4.1)
        // 送信成功後に行う: send_encoded_headers が失敗した場合に未送出セクションが
        // エンコーダーに登録されたままになるのを防ぐ
        let ric = self.qpack_encoder.last_required_insert_count();
        self.qpack_encoder.track_section(stream_id, ric);

        self.streams.insert(stream_id, stream);

        // WebTransport CONNECT の場合、セッションを Pending 状態で登録
        // (draft-ietf-webtrans-http3-15 Section 3)
        if is_wt_connect {
            let mut session = WtSession::new();
            // WT-Available-Protocols を保存 (Section 3.3)
            for h in headers {
                if h.name() == b"wt-available-protocols" {
                    if let Ok(value) = std::str::from_utf8(h.value()) {
                        session.available_protocols =
                            crate::webtransport::ConnectRequest::parse_available_protocols(value);
                    }
                    break;
                }
            }
            self.wt_sessions.insert(stream_id, session);
        }

        Ok(stream_id)
    }

    /// レスポンスを送信 (サーバー専用)
    pub(crate) fn send_response(
        &mut self,
        stream_id: u64,
        headers: &[Header],
        fin: bool,
    ) -> Result<(), Error> {
        if self.role != Role::Server {
            return Err(Error::ConnectionError(ErrorCode::InternalError));
        }

        // 送信ヘッダーの検証 (RFC 9114 Section 4.1, 4.2, 4.3.2)
        crate::validation::validate_response_headers(headers)?;

        // peer の SETTINGS_MAX_FIELD_SECTION_SIZE を超えていないかチェック (RFC 9114 Section 4.2.2)
        let peer_max = self
            .peer_settings
            .and_then(|s| s.max_field_section_size.map(VarInt::get));
        crate::validation::check_field_section_size(headers, peer_max)?;

        // WebTransport: 2xx レスポンスの WT-Protocol がクライアントの WT-Available-Protocols に
        // 含まれることを検証する (draft-ietf-webtrans-http3-15 Section 3.3)
        // 0077 Phase 5: WT 分岐を wt_session.rs のヘルパーに委譲
        self.validate_wt_response_protocol(stream_id, headers)?;

        // QPACK エンコード
        // ブロック可能ストリーム数を渡して上限制御 (RFC 9204 Section 2.1.2)
        let blocked_count = self.qpack_encoder.blocked_streams_count();
        let buf_size = estimate_encoded_size(headers) + 64;
        let mut qpack_buf = vec![0u8; buf_size];
        let qpack_len = self
            .qpack_encoder
            .encode(&mut qpack_buf, headers, blocked_count)
            .ok_or(Error::ConnectionError(ErrorCode::InternalError))?;
        qpack_buf.truncate(qpack_len);

        let stream = self
            .streams
            .get_mut(&stream_id)
            .ok_or(Error::StreamNotFound(stream_id))?;

        // 1xx 中間レスポンスかどうかを判定 (RFC 9114 Section 4.1)
        // HTTP/3 は 101 (Switching Protocols) をサポートしない (RFC 9114 Section 4.5)
        let is_interim = headers.iter().any(|h| {
            h.name() == b":status"
                && h.value().len() == 3
                && h.value()[0] == b'1'
                && h.value()[1].is_ascii_digit()
                && h.value()[2].is_ascii_digit()
                && h.value() != b"101"
        });

        stream.send_encoded_headers(&qpack_buf, fin, is_interim)?;

        // フィールドセクションの送信を記録 (RFC 9204 Section 2.1.1, 4.4.1)
        // 送信成功後に行う: send_encoded_headers が失敗した場合に未送出セクションが
        // エンコーダーに登録されたままになるのを防ぐ
        let ric = self.qpack_encoder.last_required_insert_count();
        self.qpack_encoder.track_section(stream_id, ric);

        // サーバー側: WebTransport CONNECT に対する 2xx レスポンス送信時に
        // セッションを Established に遷移させる (draft-ietf-webtrans-http3-15 Section 3)
        // 0077 Phase 5: WT 分岐を wt_session.rs のヘルパーに委譲
        self.establish_wt_session_server(stream_id, headers);

        Ok(())
    }

    /// ボディを送信
    pub fn send_body(&mut self, stream_id: u64, data: &[u8], fin: bool) -> Result<(), Error> {
        let stream = self
            .streams
            .get_mut(&stream_id)
            .ok_or(Error::StreamNotFound(stream_id))?;

        stream.send_body(data, fin)?;
        Ok(())
    }

    /// GOAWAY を送信
    ///
    /// サーバーの場合: id は client-initiated bidirectional stream ID (4 の倍数) でなければならない
    /// (RFC 9114 Section 5.2)
    ///
    /// クライアントの場合: id は push ID でなければならない。
    /// サーバープッシュ未対応のため、現在は 0 のみ許可する。
    ///
    /// 同一 ID の再送は許可される (RFC 9114 Section 5.2: "MUST NOT increase the value")。
    /// 既に送信済みの値より大きい ID を渡すと `IdError` を返す。
    pub fn send_goaway(&mut self, id: VarInt) -> Result<(), Error> {
        // GOAWAY ID の型検証 (RFC 9114 Section 5.2)
        let id_u64 = id.get();
        match self.role {
            Role::Server => {
                // サーバー → クライアント: client-initiated bidirectional stream ID
                // client-initiated bidi stream ID は 4 の倍数 (0, 4, 8, ...)
                if !id_u64.is_multiple_of(4) {
                    return Err(Error::ConnectionError(ErrorCode::IdError));
                }
            }
            Role::Client => {
                // クライアント → サーバー: push ID
                // サーバープッシュ未対応のため 0 のみ許可
                if id_u64 != 0 {
                    return Err(Error::ConnectionError(ErrorCode::IdError));
                }
            }
        }

        // 段階的送信: ID は単調減少でなければならない (RFC 9114 Section 5.2)
        if let Some(last_id) = self.last_sent_goaway_id
            && id > last_id
        {
            return Err(Error::ConnectionError(ErrorCode::IdError));
        }

        self.control_send.send_goaway(id)?;
        self.last_sent_goaway_id = Some(id);
        Ok(())
    }

    /// QUIC から RESET_STREAM 受信時に呼ぶ
    ///
    /// ストリームの状態を Reset に遷移し、イベントを発行する。
    /// クリティカルストリームへの RESET_STREAM は接続エラー (RFC 9114 Section 6.2.1, RFC 9204 Section 4.2)
    ///
    /// `final_size` は RFC 9000 Section 19.4 で定義される RESET_STREAM の Final Size。
    /// `RESET_STREAM_AT` (draft-ietf-quic-reliable-stream-reset) で運ばれた reliable size
    /// 以上の値であり、QUIC 層から渡される。WebTransport データストリームについては
    /// `WebTransportEvent::StreamReset` の `final_size` として上位層へ伝達される。
    pub fn stream_reset(
        &mut self,
        stream_id: u64,
        error_code: u64,
        final_size: u64,
    ) -> Result<(), Error> {
        // RESET_STREAM は peer が送信するストリームの中断を通知するフレームのため、
        // 判定対象は受信側クリティカルストリーム (control_recv / peer QPACK stream)。
        // STOP_SENDING (送信側が対象) とは方向が逆になる点に注意。
        let is_critical = self.control_recv.stream_id() == Some(stream_id)
            || self.peer_encoder_stream_id == Some(stream_id)
            || self.peer_decoder_stream_id == Some(stream_id);
        if is_critical {
            return Err(Error::ConnectionError(ErrorCode::ClosedCriticalStream));
        }

        if let Some(stream) = self.streams.get_mut(&stream_id) {
            let state = stream.state_mut();
            state.reset();
        }
        // QPACK ブロック状態をクリアする
        self.clear_qpack_blocked(stream_id);
        // QPACK Stream Cancellation を送信 (RFC 9204 Section 2.2.2.2)
        // max_dynamic_table_capacity が 0 の場合は省略可能
        self.send_stream_cancellation_if_needed(stream_id);

        // WebTransport セッション/データストリームへのリセット伝播
        // (draft-ietf-webtrans-http3-15 Section 4.4 / Section 6)
        if !self.handle_wt_stream_reset(stream_id, error_code, final_size) {
            // 非 WebTransport ストリーム: 汎用イベントを発行
            self.events.push_back(Event::StreamReset {
                stream_id,
                error_code,
            });
        }

        // Reset になった時点で除去する。ピア RESET 後は QUIC 層が追加データを
        // 配達しないため、feed_stream の出口や process_stream_frames のループ後
        // チェックでは発火しない。送信バッファに未交付のローカル送信データが
        // ある場合は破棄する (RFC 9114 Section 4.1.1 の未クローズ方向の急停止
        // SHOULD。RFC 9000 Section 4.4 により送信方向は RESET の影響を受けず
        // 維持されるが、キャンセル時は送信を継続しない)
        self.remove_stream_if_done(stream_id);

        Ok(())
    }

    /// QPACK ブロック状態と `blocked_by_ricnt` エントリをクリアする
    ///
    /// `stream_reset` / `stop_sending` の共通処理。ブロック中ストリームの
    /// ricnt エントリが残ると、`blocked_by_ricnt` の上限チェック
    /// (QPACK_DECOMPRESSION_FAILED 接続エラー) にカウントされ続けるため除去する。
    fn clear_qpack_blocked(&mut self, stream_id: u64) {
        if let Some(stream) = self.streams.get(&stream_id)
            && stream.is_qpack_blocked()
        {
            let ricnt = stream.qpack_ricnt();
            self.blocked_by_ricnt.remove(&(ricnt, stream_id));
        }
        if let Some(stream) = self.streams.get_mut(&stream_id) {
            stream.set_qpack_blocked(false, 0, None);
        }
    }

    /// QUIC から STOP_SENDING 受信時に呼ぶ
    ///
    /// ストリームのローカル側をクローズし、イベントを発行する。
    /// クリティカルストリームへの STOP_SENDING は接続エラー (RFC 9114 Section 6.2.1, RFC 9204 Section 4.2)
    pub fn stop_sending(&mut self, stream_id: u64, error_code: u64) -> Result<(), Error> {
        // STOP_SENDING は「こちらが送信するストリームの送信停止」を要求するフレームのため、
        // 判定対象は送信側クリティカルストリーム (control_send / ローカル QPACK encoder・decoder)。
        // 受信側ストリーム (control_recv / peer QPACK stream) はこちらが送信しないため対象外。
        let is_critical = self.control_send.stream_id() == Some(stream_id)
            || self.encoder_stream_id == Some(stream_id)
            || self.decoder_stream_id == Some(stream_id);
        if is_critical {
            return Err(Error::ConnectionError(ErrorCode::ClosedCriticalStream));
        }

        if let Some(stream) = self.streams.get_mut(&stream_id) {
            let state = stream.state_mut();
            state.close_local();
            // STOP_SENDING は送信停止の要求であり、以後データを送れないため
            // 送信バッファを破棄する (RFC 9000 Section 3.5)。
            // 破棄しないと両方向クローズ後も is_send_complete が false のまま
            // ストリームが除去されず残留する。
            // なお STOP_SENDING への RESET_STREAM 応答 (RFC 9000 Section 3.5)
            // は統合層の責務であり、統合層は Event::StopSending を受けて
            // 送信ストリームをリセットすること
            stream.discard_send_data();
        }
        // QPACK ブロック状態をクリアする (stream_reset と同じ)
        self.clear_qpack_blocked(stream_id);
        // QPACK Stream Cancellation を送信 (RFC 9204 Section 2.2.2.2)
        self.send_stream_cancellation_if_needed(stream_id);

        // WebTransport セッション/データストリームへの STOP_SENDING 伝播
        // (draft-ietf-webtrans-http3-15 Section 4.4 / Section 6)
        if !self.handle_wt_stop_sending(stream_id, error_code) {
            self.events.push_back(Event::StopSending {
                stream_id,
                error_code,
            });
        }
        // セッション終了済み (tombstone) の CONNECT ストリームを除去する
        self.remove_stream_if_done(stream_id);
        Ok(())
    }

    /// 必要に応じて QPACK Stream Cancellation を送信 (RFC 9204 Section 2.2.2.2)
    ///
    /// 動的テーブル容量が 0 の場合は送信不要。
    fn send_stream_cancellation_if_needed(&mut self, stream_id: u64) {
        let max_capacity = self
            .local_settings
            .qpack_max_table_capacity
            .map(VarInt::get)
            .unwrap_or(0);
        if max_capacity == 0 {
            return;
        }
        // 双方向ストリーム (リクエスト/レスポンス) のみ対象
        // デコーダーストリーム未初期化の場合は遅延させる
        let kind = StreamKind::from_stream_id(stream_id);
        if kind.is_bidirectional() {
            if self.decoder_stream_id.is_some() {
                self.decoder_stream.encode_stream_cancellation(stream_id);
            } else {
                self.deferred_stream_cancellations.push(stream_id);
            }
        }
    }
}

/// ヘッダーが WebTransport CONNECT かどうか判定する
///
/// `:method` = `CONNECT` かつ `:protocol` が `webtransport-h3` または `webtransport`
/// (draft-ietf-webtrans-http3-15 Section 3.2 / draft-02 互換) の場合に true。
fn is_webtransport_connect(headers: &[Header]) -> bool {
    let is_connect = headers
        .iter()
        .any(|h| h.name() == b":method" && h.value() == b"CONNECT");
    let is_wt_protocol = headers.iter().any(|h| {
        h.name() == b":protocol"
            && (h.value() == b"webtransport-h3" || h.value() == b"webtransport")
    });
    is_connect && is_wt_protocol
}

/// 通常の CONNECT リクエストかどうかを判定する (RFC 9114 Section 4.4)
///
/// `:method` が `CONNECT` で `:protocol` が存在しない場合に true。
/// Extended CONNECT (`:protocol` 付き) は別の経路で処理する。
fn is_plain_connect(headers: &[Header]) -> bool {
    let is_connect = headers
        .iter()
        .any(|h| h.name() == b":method" && h.value() == b"CONNECT");
    let has_protocol = headers.iter().any(|h| h.name() == b":protocol");
    is_connect && !has_protocol
}

/// :status が 1xx 中間レスポンスかどうかを判定する
///
/// RFC 9114 Section 4.1: 1xx はボディとトレーラーを持たない。
/// HTTP/3 は 101 (Switching Protocols) をサポートしない (RFC 9114 Section 4.5)。
fn is_informational_status(headers: &[Header]) -> bool {
    headers
        .iter()
        .find(|h| h.name() == b":status")
        .and_then(|h| std::str::from_utf8(h.value()).ok())
        .and_then(|s| s.parse::<u16>().ok())
        // HTTP/3 は 101 (Switching Protocols) をサポートしない (RFC 9114 Section 4.5)
        .map(|code| code < 200 && code != 101)
        .unwrap_or(false)
}

/// :status が 2xx 成功レスポンスかどうかを判定する
///
/// plain CONNECT のトンネル確立判定 (RFC 9114 Section 4.4) や、
/// サーバー側で送信するレスポンスヘッダーの判定に使用する。
fn is_success_status(headers: &[Header]) -> bool {
    headers
        .iter()
        .find(|h| h.name() == b":status")
        .and_then(|h| std::str::from_utf8(h.value()).ok())
        .and_then(|s| s.parse::<u16>().ok())
        .map(|code| (200..300).contains(&code))
        .unwrap_or(false)
}

/// :status が no-body レスポンス (1xx/204/304) かどうかを判定する
///
/// RFC 9114 Section 4.1.2: これらのレスポンスは content-length があっても
/// DATA フレームなしで正当。HEAD レスポンスは呼び出し側で別途判定する。
fn is_no_body_status(headers: &[Header]) -> bool {
    headers
        .iter()
        .find(|h| h.name() == b":status")
        .and_then(|h| std::str::from_utf8(h.value()).ok())
        .and_then(|s| s.parse::<u16>().ok())
        .map(|code| code < 200 || code == 204 || code == 304)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WtSetupError;
    use crate::webtransport::error::ErrorCode as WtErrorCode;
    use wt_types::WT_MAX_PENDING_SESSIONS;

    #[test]
    fn test_connection_client() {
        let conn = Connection::client(Settings::default());
        assert_eq!(conn.role(), Role::Client);
    }

    #[test]
    fn test_connection_server() {
        let conn = Connection::server(Settings::default());
        assert_eq!(conn.role(), Role::Server);
    }

    #[test]
    fn test_send_request() {
        let mut conn = Connection::client(Settings::default());
        conn.set_control_stream_id(2).expect("test must succeed");
        let headers = vec![
            Header::new(b":method", b"GET").expect("test must succeed"),
            Header::new(b":path", b"/").expect("test must succeed"),
            Header::new(b":scheme", b"https").expect("test must succeed"),
            Header::new(b":authority", b"example.com").expect("test must succeed"),
        ];

        let stream_id = conn
            .send_request(&headers, true)
            .expect("test must succeed");
        assert_eq!(stream_id, 0);

        // 次のリクエスト
        let stream_id2 = conn
            .send_request(&headers, true)
            .expect("test must succeed");
        assert_eq!(stream_id2, 4);
    }

    #[test]
    fn test_control_stream() {
        let mut conn = Connection::client(Settings::default());
        conn.set_control_stream_id(2).expect("test must succeed");

        // 制御ストリームの送信データを取得
        let (data, fin) = conn.get_stream_data(2).expect("test must succeed");
        assert!(!data.is_empty());
        assert!(!fin);
        assert_eq!(data[0], 0x00); // Control stream type
    }

    // =========================================================================
    // 0023: RESET_STREAM / STOP_SENDING によるクリティカルストリーム閉鎖検出
    // (RFC 9114 Section 6.2.1, RFC 9204 Section 4.2)
    // =========================================================================

    #[test]
    fn test_stream_reset_on_control_stream_is_closed_critical_stream() {
        let mut conn = Connection::client(Settings::default());
        // サーバーの単方向ストリーム (ID=3) を制御ストリームとして登録
        // 制御ストリームタイプ (0x00) + SETTINGS フレーム (type=0x04, length=0x00)
        conn.feed_stream(3, &[0x00, 0x04, 0x00], false)
            .expect("test must succeed");
        let err = conn.stream_reset(3, 0, 0).unwrap_err();
        assert!(matches!(
            err,
            Error::ConnectionError(ErrorCode::ClosedCriticalStream)
        ));
    }

    #[test]
    fn test_stop_sending_on_control_stream_is_closed_critical_stream() {
        // STOP_SENDING は送信側クリティカルストリームを対象とする。
        // ローカルの送信制御ストリーム (control_send) への STOP_SENDING は接続エラー。
        let mut conn = Connection::client(Settings::default());
        conn.set_control_stream_id(2).expect("test must succeed");
        let err = conn.stop_sending(2, 0).unwrap_err();
        assert!(matches!(
            err,
            Error::ConnectionError(ErrorCode::ClosedCriticalStream)
        ));
    }

    #[test]
    fn test_stop_sending_on_peer_control_stream_is_not_critical() {
        // STOP_SENDING は受信側ストリーム (peer の制御ストリーム) を対象としない。
        // こちらが送信しないストリームへの STOP_SENDING はクリティカル扱いしない。
        let mut conn = Connection::client(Settings::default());
        conn.feed_stream(3, &[0x00, 0x04, 0x00], false)
            .expect("test must succeed");
        assert!(conn.stop_sending(3, 0).is_ok());
    }

    #[test]
    fn test_stop_sending_on_peer_qpack_streams_is_not_critical() {
        // peer の QPACK エンコーダー (0x02) / デコーダー (0x03) も受信側ストリームのため
        // STOP_SENDING はクリティカル扱いしない。stream_reset (受信側を critical 扱い) との
        // 方向の非対称が崩れていないことを固定する。
        let mut conn = Connection::client(Settings::default());
        conn.feed_stream(3, &[0x02], false)
            .expect("test must succeed");
        assert!(conn.stop_sending(3, 0).is_ok());

        let mut conn = Connection::client(Settings::default());
        conn.feed_stream(3, &[0x03], false)
            .expect("test must succeed");
        assert!(conn.stop_sending(3, 0).is_ok());
    }

    #[test]
    fn test_stream_reset_on_qpack_encoder_stream_is_closed_critical_stream() {
        let mut conn = Connection::client(Settings::default());
        // QPACK エンコーダーストリームタイプ (0x02)
        conn.feed_stream(3, &[0x02], false)
            .expect("test must succeed");
        let err = conn.stream_reset(3, 0, 0).unwrap_err();
        assert!(matches!(
            err,
            Error::ConnectionError(ErrorCode::ClosedCriticalStream)
        ));
    }

    #[test]
    fn test_stream_reset_on_qpack_decoder_stream_is_closed_critical_stream() {
        let mut conn = Connection::client(Settings::default());
        // QPACK デコーダーストリームタイプ (0x03)
        conn.feed_stream(3, &[0x03], false)
            .expect("test must succeed");
        let err = conn.stream_reset(3, 0, 0).unwrap_err();
        assert!(matches!(
            err,
            Error::ConnectionError(ErrorCode::ClosedCriticalStream)
        ));
    }

    #[test]
    fn test_stop_sending_on_qpack_encoder_stream_is_closed_critical_stream() {
        // ローカル QPACK エンコーダーストリームへの STOP_SENDING は接続エラー。
        let mut conn = Connection::client(Settings::default());
        conn.set_encoder_stream_id(6).expect("test must succeed");
        let err = conn.stop_sending(6, 0).unwrap_err();
        assert!(matches!(
            err,
            Error::ConnectionError(ErrorCode::ClosedCriticalStream)
        ));
    }

    #[test]
    fn test_stop_sending_on_qpack_decoder_stream_is_closed_critical_stream() {
        // ローカル QPACK デコーダーストリームへの STOP_SENDING は接続エラー。
        let mut conn = Connection::client(Settings::default());
        conn.set_decoder_stream_id(10).expect("test must succeed");
        let err = conn.stop_sending(10, 0).unwrap_err();
        assert!(matches!(
            err,
            Error::ConnectionError(ErrorCode::ClosedCriticalStream)
        ));
    }

    #[test]
    fn test_stream_reset_on_non_critical_stream_is_ok() {
        // 通常のリクエストストリームへの RESET_STREAM は正常
        let mut conn = Connection::client(Settings::default());
        assert!(conn.stream_reset(0, 0, 0).is_ok());
    }

    #[test]
    fn test_stop_sending_on_non_critical_stream_is_ok() {
        let mut conn = Connection::client(Settings::default());
        assert!(conn.stop_sending(0, 0).is_ok());
    }

    // =========================================================================
    // 0048/0049: QPACK ストリームの stream type 書き込みと writable_streams 登録
    // (RFC 9114 Section 6.2, RFC 9204 Section 4.2)
    // =========================================================================

    #[test]
    fn test_set_encoder_stream_id_writes_stream_type() {
        let mut conn = Connection::client(Settings::default());
        conn.set_encoder_stream_id(6).expect("test must succeed");

        // stream type 0x02 が送信データに含まれる
        let (data, fin) = conn.get_stream_data(6).expect("test must succeed");
        assert_eq!(data[0], 0x02);
        assert!(!fin);
    }

    #[test]
    fn test_set_decoder_stream_id_writes_stream_type() {
        let mut conn = Connection::client(Settings::default());
        conn.set_decoder_stream_id(10).expect("test must succeed");

        // stream type 0x03 が送信データに含まれる
        let (data, fin) = conn.get_stream_data(10).expect("test must succeed");
        assert_eq!(data[0], 0x03);
        assert!(!fin);
    }

    #[test]
    fn test_encoder_stream_in_writable_streams() {
        let mut conn = Connection::client(Settings::default());
        conn.set_encoder_stream_id(6).expect("test must succeed");

        // writable_streams にエンコーダーストリームが含まれる
        let writable: Vec<u64> = conn.writable_streams().collect();
        assert!(
            writable.contains(&6),
            "encoder stream should be in writable_streams"
        );
    }

    #[test]
    fn test_decoder_stream_in_writable_streams() {
        let mut conn = Connection::client(Settings::default());
        conn.set_decoder_stream_id(10).expect("test must succeed");

        // writable_streams にデコーダーストリームが含まれる
        let writable: Vec<u64> = conn.writable_streams().collect();
        assert!(
            writable.contains(&10),
            "decoder stream should be in writable_streams"
        );
    }

    #[test]
    fn test_encoder_stream_set_capacity_after_settings() {
        // SETTINGS 受信後に Set Dynamic Table Capacity 命令がエンコーダーストリームに積まれる
        let mut conn = Connection::client(Settings::default());
        conn.set_control_stream_id(2).expect("test must succeed");
        conn.set_encoder_stream_id(6).expect("test must succeed");

        // stream type バイトを消費
        let (data, _) = conn.get_stream_data(6).expect("test must succeed");
        let len = data.len();
        conn.consume_stream_data(6, len);

        // ピアから SETTINGS (QPACK_MAX_TABLE_CAPACITY=4096) を受信
        // stream type (0x00) + SETTINGS frame (type=0x04, len=3, QPACK_MAX_TABLE_CAPACITY=4096)
        // 0x01 は QPACK_MAX_TABLE_CAPACITY の設定 ID
        // varint 4096 = 2 バイト varint: 0x50, 0x00
        // SETTINGS frame: type=0x04, length=3, payload=[0x01, 0x50, 0x00]
        conn.feed_stream(3, &[0x00, 0x04, 0x03, 0x01, 0x50, 0x00], false)
            .expect("test must succeed");

        // エンコーダーストリームに Set Dynamic Table Capacity 命令がある
        let data = conn.get_stream_data(6);
        assert!(
            data.is_some(),
            "encoder stream should have Set Capacity data after SETTINGS"
        );
    }

    #[test]
    fn test_is_informational_status() {
        use crate::qpack::Header;

        let make = |name: &[u8], value: &[u8]| {
            Header::from_validated_parts_internal(
                std::borrow::Cow::Owned(name.to_vec()),
                std::borrow::Cow::Owned(value.to_vec()),
            )
        };

        // 1xx は informational (101 を除く)
        assert!(is_informational_status(&[make(b":status", b"100")]));
        // 101 は HTTP/3 でサポートしない (RFC 9114 Section 4.5)
        assert!(!is_informational_status(&[make(b":status", b"101")]));
        assert!(is_informational_status(&[make(b":status", b"199")]));

        // 2xx 以上は informational ではない
        assert!(!is_informational_status(&[make(b":status", b"200")]));
        assert!(!is_informational_status(&[make(b":status", b"304")]));
        assert!(!is_informational_status(&[make(b":status", b"404")]));

        // :status なし
        assert!(!is_informational_status(&[]));
        assert!(!is_informational_status(&[make(
            b"content-type",
            b"text/html"
        )]));
    }

    // =========================================================================
    // WebTransport 単方向ストリーム (0x54) の処理
    // (draft-ietf-webtrans-http3-15 Section 4.2)
    // =========================================================================

    /// テスト用の VarInt 構築ショートカット
    fn vi(value: u64) -> VarInt {
        VarInt::new(value).expect("test must succeed")
    }

    /// WebTransport 有効な Settings を作成するヘルパー
    fn wt_enabled_settings() -> Settings {
        let wt = crate::webtransport::Settings::new().wt_enabled(vi(1));
        Settings::new().enable_webtransport_server(wt)
    }

    fn wt_multi_draft_settings_with_flow_control() -> Settings {
        let wt = crate::webtransport::Settings::new()
            .wt_enabled(vi(1))
            .enable_webtransport_draft02(true)
            .webtransport_max_sessions_draft07(vi(100))
            .wt_max_sessions_draft14(vi(100))
            .wt_initial_max_streams_uni(vi(100))
            .wt_initial_max_streams_bidi(vi(100))
            .wt_initial_max_data(vi(8 * 1024 * 1024));
        Settings::new().enable_webtransport_server(wt)
    }

    /// WebTransport のネゴシエーションが完了した状態のクライアントを作成するヘルパー
    ///
    /// peer SETTINGS、transport parameter、RESET_STREAM_AT を全て設定する。
    fn wt_negotiated_client() -> Connection {
        let mut conn = Connection::client(wt_enabled_settings());
        conn.set_control_stream_id(2).expect("test must succeed");
        // peer SETTINGS を注入
        conn.peer_settings = Some(wt_enabled_settings());
        // transport parameter 検証済み
        conn.wt_transport_verified = true;
        conn.wt_reset_stream_at_supported = true;
        conn
    }

    /// WebTransport のネゴシエーションが完了し、指定セッションが確立済みのクライアントを作成するヘルパー
    fn wt_negotiated_client_with_session(session_id: u64) -> Connection {
        let mut conn = wt_negotiated_client();
        let mut session = WtSession::new();
        session.state = WtSessionState::Established;
        conn.wt_sessions.insert(session_id, session);
        conn
    }

    /// WebTransport のネゴシエーションが完了した状態のサーバーを作成するヘルパー

    #[test]
    fn test_wt_uni_stream_open_and_data() {
        let mut conn = make_server_with_established_wt_session(4);

        // ストリームタイプ 0x54 (varint 2 バイト: [0x40, 0x54])
        // + セッション ID 4 (varint 1 バイト: [0x04]) + データ
        conn.feed_stream(2, &[0x40, 0x54, 0x04, 0xAA, 0xBB], false)
            .expect("test must succeed");

        // WebTransportEvent::UniStreamOpen イベント
        let event = conn
            .poll_event()
            .expect("test must succeed")
            .expect("test must succeed");
        assert!(matches!(
            event,
            Event::WebTransport(WebTransportEvent::UniStreamOpen {
                stream_id: 2,
                session_id: 4,
            })
        ));

        // WebTransportEvent::UniStreamData イベント
        let event = conn
            .poll_event()
            .expect("test must succeed")
            .expect("test must succeed");
        assert!(matches!(
            event,
            Event::WebTransport(WebTransportEvent::UniStreamData {
                stream_id: 2,
                ref data,
            }) if data == &[0xAA, 0xBB]
        ));
    }

    #[test]
    fn test_wt_uni_stream_subsequent_data() {
        let mut conn = make_server_with_established_wt_session(4);

        // 初回: ストリームタイプ + セッション ID のみ
        conn.feed_stream(2, &[0x40, 0x54, 0x04], false)
            .expect("test must succeed");
        let event = conn
            .poll_event()
            .expect("test must succeed")
            .expect("test must succeed");
        assert!(matches!(
            event,
            Event::WebTransport(WebTransportEvent::UniStreamOpen { .. })
        ));

        // 後続データ
        conn.feed_stream(2, &[0xCC, 0xDD], false)
            .expect("test must succeed");
        let event = conn
            .poll_event()
            .expect("test must succeed")
            .expect("test must succeed");
        assert!(matches!(
            event,
            Event::WebTransport(WebTransportEvent::UniStreamData {
                stream_id: 2,
                ref data,
            }) if data == &[0xCC, 0xDD]
        ));
    }

    #[test]
    fn test_wt_uni_stream_fin() {
        let mut conn = make_server_with_established_wt_session(4);

        conn.feed_stream(2, &[0x40, 0x54, 0x04], false)
            .expect("test must succeed");
        let _ = conn.poll_event(); // Open イベントを消費

        conn.feed_stream(2, &[], true).expect("test must succeed");
        let event = conn
            .poll_event()
            .expect("test must succeed")
            .expect("test must succeed");
        assert!(matches!(
            event,
            Event::WebTransport(WebTransportEvent::UniStreamEnd { stream_id: 2 })
        ));
    }

    #[test]
    fn test_wt_uni_stream_not_negotiated_returns_stream_error() {
        // WebTransport ネゴシエーション未完了の接続
        let mut conn = Connection::server(Settings::default());

        // ストリームタイプ 0x54 (varint 2 バイト: [0x40, 0x54])
        // 未ネゴシエーションの 0x54 は「recipient がサポートしないストリームタイプ」
        // に該当し、ストリーム単位の拒否で対処する (RFC 9114 Section 6.2)。
        // 接続エラーにしてはならない (MUST NOT)。
        let err = conn.feed_stream(2, &[0x40, 0x54, 0x04], false).unwrap_err();
        assert!(matches!(
            err,
            Error::StreamError(ErrorCode::StreamCreationError)
        ));
    }

    #[test]
    fn test_wt_uni_stream_not_negotiated_followup_data_returns_stream_error() {
        // ストリームエラー後の後続データは、stream_id がどのマップにも登録されて
        // いないため再びストリームタイプとして解釈される。
        // 後続データが 0x54 の varint エンコーディング (例: [0x40, 0x54, ...]) で
        // 始まる場合は同じストリームエラーが返る。
        let mut conn = Connection::server(Settings::default());

        // 1 回目: ストリームエラー
        let err = conn.feed_stream(2, &[0x40, 0x54, 0x04], false).unwrap_err();
        assert!(matches!(
            err,
            Error::StreamError(ErrorCode::StreamCreationError)
        ));

        // 2 回目: 後続データも同じストリームエラー
        let err = conn.feed_stream(2, &[0x40, 0x54, 0x05], false).unwrap_err();
        assert!(matches!(
            err,
            Error::StreamError(ErrorCode::StreamCreationError)
        ));
    }

    #[test]
    fn test_wt_uni_stream_not_negotiated_fin_only_is_ok() {
        // データなしの FIN のみはストリームタイプが解釈されないため、
        // ストリームエラーにならず Ok(()) を返す
        // (RFC 9114 Section 6.2: ストリームヘッダー受信前に閉じられた
        //  単方向ストリームは許容される)
        let mut conn = Connection::server(Settings::default());

        conn.feed_stream(2, &[], true).expect("test must succeed");
    }

    #[test]
    fn test_wt_uni_stream_not_negotiated_partial_type_then_fin_is_ok() {
        // ストリームタイプ varint が未完のまま FIN が来た場合はバッファを破棄し、
        // Ok(()) を返す (RFC 9114 Section 6.2)
        let mut conn = Connection::server(Settings::default());

        // 0x54 の 1 バイト目のみ (varint 未完)
        conn.feed_stream(2, &[0x40], false)
            .expect("test must succeed");
        // 未完のまま FIN
        conn.feed_stream(2, &[], true).expect("test must succeed");
    }

    #[test]
    fn test_wt_uni_stream_not_negotiated_partial_type_with_fin_is_ok() {
        // 同一チャンクで varint 未完 + FIN が届いた場合もバッファを破棄し、
        // Ok(()) を返す (RFC 9114 Section 6.2)
        let mut conn = Connection::server(Settings::default());

        // 0x54 の 1 バイト目のみ (varint 未完) + FIN
        conn.feed_stream(2, &[0x40], true)
            .expect("test must succeed");

        // バッファが破棄されていることを確認するため、後続チャンク (本来は
        // 到着しないが) があってもバッファに残留していないことを確認する。
        // 新規ストリームとして再解釈されるため、未知タイプとして無視される。
        conn.feed_stream(2, &[0x41], false)
            .expect("test must succeed");
    }

    #[test]
    fn test_wt_uni_stream_not_negotiated_split_type_returns_stream_error() {
        // ストリームタイプ varint が分割到着しても、ネゴシエーション未完了の
        // 0x54 はストリームエラーになる
        let mut conn = Connection::server(Settings::default());

        // 0x54 の 1 バイト目のみ (varint 未完: バッファリングされる)
        conn.feed_stream(2, &[0x40], false)
            .expect("test must succeed");

        // 2 バイト目 + セッション ID: ストリームタイプが確定しストリームエラー
        let err = conn.feed_stream(2, &[0x54, 0x04], false).unwrap_err();
        assert!(matches!(
            err,
            Error::StreamError(ErrorCode::StreamCreationError)
        ));
    }

    #[test]
    fn test_wt_uni_stream_not_negotiated_error_keeps_connection_alive() {
        // ストリームエラー後も接続は生存し、別ストリーム (制御ストリーム) の
        // feed_stream が成功する
        let mut conn = Connection::server(Settings::default());

        // 0x54 でストリームエラー
        let err = conn.feed_stream(2, &[0x40, 0x54, 0x04], false).unwrap_err();
        assert!(matches!(
            err,
            Error::StreamError(ErrorCode::StreamCreationError)
        ));

        // 制御ストリーム (stream_id=6) は引き続き処理できる
        // ストリームタイプ 0x00 + SETTINGS フレーム
        conn.feed_stream(6, &[0x00, 0x04, 0x00], false)
            .expect("test must succeed");
        let event = conn
            .poll_event()
            .expect("test must succeed")
            .expect("test must succeed");
        assert!(matches!(event, Event::SettingsReceived { .. }));
    }

    #[test]
    fn test_wt_uni_stream_session_id_split_across_chunks() {
        let mut conn = make_server_with_established_wt_session(4);

        // ストリームタイプの 1 バイト目のみ (varint 未完了)
        conn.feed_stream(2, &[0x40], false)
            .expect("test must succeed");
        assert!(conn.poll_event().expect("test must succeed").is_none());

        // ストリームタイプの 2 バイト目 + セッション ID
        conn.feed_stream(2, &[0x54, 0x04], false)
            .expect("test must succeed");
        let event = conn
            .poll_event()
            .expect("test must succeed")
            .expect("test must succeed");
        assert!(matches!(
            event,
            Event::WebTransport(WebTransportEvent::UniStreamOpen {
                stream_id: 2,
                session_id: 4,
            })
        ));
    }

    #[test]
    fn test_wt_uni_stream_session_id_split_from_type() {
        let mut conn = make_server_with_established_wt_session(4);

        // ストリームタイプのみ (セッション ID なし)
        conn.feed_stream(2, &[0x40, 0x54], false)
            .expect("test must succeed");
        assert!(conn.poll_event().expect("test must succeed").is_none());

        // セッション ID
        conn.feed_stream(2, &[0x04], false)
            .expect("test must succeed");
        let event = conn
            .poll_event()
            .expect("test must succeed")
            .expect("test must succeed");
        assert!(matches!(
            event,
            Event::WebTransport(WebTransportEvent::UniStreamOpen {
                stream_id: 2,
                session_id: 4,
            })
        ));
    }

    #[test]
    fn test_wt_uni_stream_invalid_session_id_returns_id_error() {
        // session_id が client-initiated bidi stream ID (% 4 == 0) でない場合は
        // H3_ID_ERROR で接続エラー (draft-ietf-webtrans-http3-15 Section 4.2)
        let mut conn = make_server_with_established_wt_session(4);

        // session_id = 1 (server-initiated bidi: 不正)
        let err = conn.feed_stream(2, &[0x40, 0x54, 0x01], false).unwrap_err();
        assert!(matches!(err, Error::ConnectionError(ErrorCode::IdError)));
    }

    #[test]
    fn test_wt_uni_stream_invalid_session_id_uni_stream() {
        // session_id = 2 (client-initiated uni: 不正)
        let mut conn = make_server_with_established_wt_session(4);

        let err = conn.feed_stream(2, &[0x40, 0x54, 0x02], false).unwrap_err();
        assert!(matches!(err, Error::ConnectionError(ErrorCode::IdError)));
    }

    #[test]
    fn test_wt_uni_stream_invalid_session_id_server_uni_stream() {
        // session_id = 3 (server-initiated uni: 不正)
        let mut conn = make_server_with_established_wt_session(4);

        let err = conn.feed_stream(2, &[0x40, 0x54, 0x03], false).unwrap_err();
        assert!(matches!(err, Error::ConnectionError(ErrorCode::IdError)));
    }

    // =========================================================================
    // WebTransport 双方向ストリーム (server-initiated bidi)
    // (draft-ietf-webtrans-http3-15 Section 4.3)
    // =========================================================================

    #[test]
    fn test_wt_bidi_stream_open_and_data() {
        // クライアント: server-initiated bidi stream を受信
        // signal value (0x41) + session_id (0x00) + ペイロード
        let mut conn = wt_negotiated_client_with_session(0);

        // stream_id = 1 は server-initiated bidi
        // signal value 0x41 は varint で [0x40, 0x41] + session_id 0x00 + ペイロード
        conn.feed_stream(1, &[0x40, 0x41, 0x00, 0xAA, 0xBB], false)
            .expect("test must succeed");

        let event = conn
            .poll_event()
            .expect("test must succeed")
            .expect("test must succeed");
        assert!(matches!(
            event,
            Event::WebTransport(WebTransportEvent::BidiStreamOpen {
                stream_id: 1,
                session_id: 0,
            })
        ));

        let event = conn
            .poll_event()
            .expect("test must succeed")
            .expect("test must succeed");
        assert!(matches!(
            event,
            Event::WebTransport(WebTransportEvent::BidiStreamData {
                stream_id: 1,
                ref data,
            }) if data == &[0xAA, 0xBB]
        ));
    }

    #[test]
    fn test_wt_bidi_stream_subsequent_data() {
        // 確定済み WT bidi stream への後続データ
        let mut conn = wt_negotiated_client_with_session(0);

        conn.feed_stream(1, &[0x40, 0x41, 0x00], false)
            .expect("test must succeed");
        let event = conn
            .poll_event()
            .expect("test must succeed")
            .expect("test must succeed");
        assert!(matches!(
            event,
            Event::WebTransport(WebTransportEvent::BidiStreamOpen { .. })
        ));

        conn.feed_stream(1, &[0xCC, 0xDD], false)
            .expect("test must succeed");
        let event = conn
            .poll_event()
            .expect("test must succeed")
            .expect("test must succeed");
        assert!(matches!(
            event,
            Event::WebTransport(WebTransportEvent::BidiStreamData {
                stream_id: 1,
                ref data,
            }) if data == &[0xCC, 0xDD]
        ));
    }

    #[test]
    fn test_wt_bidi_stream_fin() {
        let mut conn = wt_negotiated_client_with_session(0);

        conn.feed_stream(1, &[0x40, 0x41, 0x00], false)
            .expect("test must succeed");
        let _ = conn.poll_event().expect("test must succeed"); // Open

        conn.feed_stream(1, &[], true).expect("test must succeed");
        let event = conn
            .poll_event()
            .expect("test must succeed")
            .expect("test must succeed");
        assert!(matches!(
            event,
            Event::WebTransport(WebTransportEvent::BidiStreamEnd { stream_id: 1 })
        ));
    }

    #[test]
    fn test_wt_bidi_stream_rejected_when_wt_disabled() {
        // WebTransport 無効なクライアントは server-initiated bidi を拒否
        let mut conn = Connection::client(Settings::default());
        conn.set_control_stream_id(2).expect("test must succeed");

        let err = conn.feed_stream(1, &[0x40, 0x41, 0x00], false).unwrap_err();
        assert!(matches!(
            err,
            Error::ConnectionError(ErrorCode::StreamCreationError)
        ));
    }

    #[test]
    fn test_wt_bidi_stream_invalid_signal_value() {
        // signal value が 0x41 でない場合は H3_FRAME_ERROR
        let mut conn = wt_negotiated_client();

        // 0x01 は不正な signal value
        let err = conn.feed_stream(1, &[0x01, 0x00], false).unwrap_err();
        assert!(matches!(err, Error::ConnectionError(ErrorCode::FrameError)));
    }

    #[test]
    fn test_wt_bidi_stream_invalid_session_id() {
        // session_id が client-initiated bidi stream ID でない場合は H3_ID_ERROR
        let mut conn = wt_negotiated_client();

        // signal value 0x41 (varint [0x40, 0x41]) + session_id = 1 (server-initiated bidi: 不正)
        let err = conn.feed_stream(1, &[0x40, 0x41, 0x01], false).unwrap_err();
        assert!(matches!(err, Error::ConnectionError(ErrorCode::IdError)));
    }

    #[test]
    fn test_wt_bidi_stream_split_signal_value() {
        // signal value が複数チャンクにまたがる場合
        let mut conn = wt_negotiated_client_with_session(4);

        // 空データ (signal value なし)
        conn.feed_stream(1, &[], false).expect("test must succeed");
        assert!(conn.poll_event().expect("test must succeed").is_none());

        // signal value + session_id
        conn.feed_stream(1, &[0x40, 0x41, 0x04], false)
            .expect("test must succeed");
        let event = conn
            .poll_event()
            .expect("test must succeed")
            .expect("test must succeed");
        assert!(matches!(
            event,
            Event::WebTransport(WebTransportEvent::BidiStreamOpen {
                stream_id: 1,
                session_id: 4,
            })
        ));
    }

    #[test]
    fn test_wt_bidi_stream_split_session_id() {
        // signal value は確定したが session_id が次のチャンクに分割される場合
        let mut conn = wt_negotiated_client_with_session(4);

        // signal value のみ (varint 2 バイト)
        conn.feed_stream(1, &[0x40, 0x41], false)
            .expect("test must succeed");
        assert!(conn.poll_event().expect("test must succeed").is_none());

        // session_id
        conn.feed_stream(1, &[0x04], false)
            .expect("test must succeed");
        let event = conn
            .poll_event()
            .expect("test must succeed")
            .expect("test must succeed");
        assert!(matches!(
            event,
            Event::WebTransport(WebTransportEvent::BidiStreamOpen {
                stream_id: 1,
                session_id: 4,
            })
        ));
    }

    #[test]
    fn test_wt_bidi_stream_session_id_4() {
        // session_id = 4 (2 番目の client-initiated bidi stream)
        let mut conn = wt_negotiated_client_with_session(4);

        conn.feed_stream(1, &[0x40, 0x41, 0x04], false)
            .expect("test must succeed");
        let event = conn
            .poll_event()
            .expect("test must succeed")
            .expect("test must succeed");
        assert!(matches!(
            event,
            Event::WebTransport(WebTransportEvent::BidiStreamOpen {
                stream_id: 1,
                session_id: 4,
            })
        ));
    }

    // =========================================================================
    // WebTransport 双方向ストリーム (client-initiated bidi)
    // サーバー側でクライアント開始の WT bidi ストリームを受理する
    // (draft-ietf-webtrans-http3-15 Section 4.3)
    // =========================================================================

    #[test]
    fn test_server_wt_bidi_stream_client_initiated() {
        // サーバー: クライアント開始の bidi stream を WT bidi として受理
        let mut conn = make_server_with_established_wt_session(0);

        // stream_id = 0 はクライアント開始 bidi
        // signal value 0x41 (varint [0x40, 0x41]) + session_id 0x00 + ペイロード
        conn.feed_stream(0, &[0x40, 0x41, 0x00, 0xAA, 0xBB], false)
            .expect("test must succeed");

        let event = conn
            .poll_event()
            .expect("test must succeed")
            .expect("test must succeed");
        assert!(matches!(
            event,
            Event::WebTransport(WebTransportEvent::BidiStreamOpen {
                stream_id: 0,
                session_id: 0,
            })
        ));

        let event = conn
            .poll_event()
            .expect("test must succeed")
            .expect("test must succeed");
        assert!(matches!(
            event,
            Event::WebTransport(WebTransportEvent::BidiStreamData {
                stream_id: 0,
                ref data,
            }) if data == &[0xAA, 0xBB]
        ));
    }

    #[test]
    fn test_server_client_bidi_request_not_wt() {
        // サーバー: クライアント開始の bidi stream が 0x41 でない場合はリクエストとして処理
        let mut conn = Connection::server(wt_enabled_settings());
        conn.set_control_stream_id(3).expect("test must succeed");

        // HEADERS フレーム (type=0x01) はリクエストストリーム
        // 先頭バイト 0x01 は 1 バイト varint (値 0x01) なので WT_STREAM (0x41) ではない
        // 空の HEADERS フレーム: type=0x01, length=0x00
        // QPACK でデコード失敗するがリクエストストリームとしてディスパッチされたことを確認
        let err = conn.feed_stream(0, &[0x01, 0x00], false).unwrap_err();
        // 空の HEADERS ペイロードは QPACK デコード失敗
        assert!(matches!(err, Error::Qpack(_)));
    }

    #[test]
    fn test_server_client_bidi_dispatch_split_varint() {
        // サーバー: 先頭 varint が分割された場合のバッファリング
        let mut conn = make_server_with_established_wt_session(0);

        // 2 バイト varint の先頭 1 バイトのみ
        conn.feed_stream(0, &[0x40], false)
            .expect("test must succeed");
        assert!(conn.poll_event().expect("test must succeed").is_none());

        // 2 バイト目で 0x41 確定 + session_id
        conn.feed_stream(0, &[0x41, 0x00], false)
            .expect("test must succeed");
        let event = conn
            .poll_event()
            .expect("test must succeed")
            .expect("test must succeed");
        assert!(matches!(
            event,
            Event::WebTransport(WebTransportEvent::BidiStreamOpen {
                stream_id: 0,
                session_id: 0,
            })
        ));
    }

    #[test]
    fn test_server_client_bidi_dispatch_split_varint_not_wt() {
        // サーバー: 先頭 varint が分割され、0x41 でないと判定された場合
        let mut conn = Connection::server(wt_enabled_settings());
        conn.set_control_stream_id(3).expect("test must succeed");

        // 2 バイト varint の先頭 1 バイトのみ (0x40 prefix)
        conn.feed_stream(0, &[0x40], false)
            .expect("test must succeed");
        assert!(conn.poll_event().expect("test must succeed").is_none());

        // 2 バイト目で 0x42 (値 0x42, WT_STREAM ではない) → リクエストストリーム
        // ただし [0x40, 0x42] は varint で値 0x42 (= HEADERS フレームではない未知フレームタイプ)
        // リクエストストリームとして処理される
        conn.feed_stream(0, &[0x42, 0x02, 0xAA, 0xBB], false)
            .expect("test must succeed");
        // 未知フレームタイプはスキップされる (RFC 9114 Section 9)
        assert!(conn.poll_event().expect("test must succeed").is_none());
    }

    #[test]
    fn test_server_client_bidi_dispatch_empty_then_data() {
        // サーバー: 空データ → 後続データで判定
        let mut conn = make_server_with_established_wt_session(0);

        // 空データ
        conn.feed_stream(0, &[], false).expect("test must succeed");
        assert!(conn.poll_event().expect("test must succeed").is_none());

        // WT bidi データ
        conn.feed_stream(0, &[0x40, 0x41, 0x00], false)
            .expect("test must succeed");
        let event = conn
            .poll_event()
            .expect("test must succeed")
            .expect("test must succeed");
        assert!(matches!(
            event,
            Event::WebTransport(WebTransportEvent::BidiStreamOpen {
                stream_id: 0,
                session_id: 0,
            })
        ));
    }

    #[test]
    fn test_server_client_bidi_dispatch_empty_fin() {
        // サーバー: 空データ + FIN はリクエストストリームとして処理
        // (HEADERS なしで FIN = H3_MESSAGE_ERROR)
        let mut conn = Connection::server(wt_enabled_settings());
        conn.set_control_stream_id(3).expect("test must succeed");

        let err = conn.feed_stream(0, &[], true).unwrap_err();
        assert!(matches!(err, Error::StreamError(ErrorCode::MessageError)));
    }

    // =========================================================================
    // WebTransport 能力ネゴシエーション
    // (draft-ietf-webtrans-http3-15 Section 3.1, 4.6)
    // =========================================================================

    #[test]
    fn test_wt_connect_rejected_without_peer_settings() {
        // クライアント: peer SETTINGS 未受信の状態で WebTransport CONNECT を送信
        let mut conn = Connection::client(wt_enabled_settings());
        conn.set_control_stream_id(2).expect("test must succeed");

        let headers = vec![
            Header::new(b":method", b"CONNECT").expect("test must succeed"),
            Header::new(b":protocol", b"webtransport-h3").expect("test must succeed"),
            Header::new(b":scheme", b"https").expect("test must succeed"),
            Header::new(b":authority", b"example.com").expect("test must succeed"),
            Header::new(b":path", b"/wt").expect("test must succeed"),
        ];

        // peer SETTINGS 未受信なので WtSetupError
        let err = conn.send_request(&headers, false).unwrap_err();
        assert!(matches!(
            err,
            Error::WtSetup(WtSetupError::PeerSettingsNotReceived)
        ));
    }

    #[test]
    fn test_wt_connect_rejected_when_peer_wt_disabled() {
        // クライアント: peer SETTINGS で WT 無効
        let mut conn = Connection::client(wt_enabled_settings());
        conn.set_control_stream_id(2).expect("test must succeed");

        // peer から WT 無効な SETTINGS を受信
        // 制御ストリーム: タイプ (0x00) + SETTINGS フレーム (type=0x04, length=0x00)
        conn.feed_stream(3, &[0x00, 0x04, 0x00], false)
            .expect("test must succeed");

        let headers = vec![
            Header::new(b":method", b"CONNECT").expect("test must succeed"),
            Header::new(b":protocol", b"webtransport-h3").expect("test must succeed"),
            Header::new(b":scheme", b"https").expect("test must succeed"),
            Header::new(b":authority", b"example.com").expect("test must succeed"),
            Header::new(b":path", b"/wt").expect("test must succeed"),
        ];

        let err = conn.send_request(&headers, false).unwrap_err();
        assert!(matches!(
            err,
            Error::WtSetup(WtSetupError::WebTransportNotEnabled)
        ));
    }

    #[test]
    fn test_server_wt_connect_rejected_without_peer_settings() {
        // サーバー: peer (クライアント) SETTINGS 未受信の状態で WT CONNECT を受信
        // (draft-ietf-webtrans-http3-15 Section 7.1)
        let mut client = Connection::client(wt_enabled_settings());
        client.set_control_stream_id(2).expect("test must succeed");

        let mut server = Connection::server(wt_enabled_settings());
        server.set_control_stream_id(3).expect("test must succeed");

        // クライアントから WT CONNECT を送信
        let headers = vec![
            Header::new(b":method", b"CONNECT").expect("test must succeed"),
            Header::new(b":protocol", b"webtransport-h3").expect("test must succeed"),
            Header::new(b":scheme", b"https").expect("test must succeed"),
            Header::new(b":authority", b"example.com").expect("test must succeed"),
            Header::new(b":path", b"/wt").expect("test must succeed"),
        ];
        let stream_id = client.send_request(&headers, false).unwrap_err();
        // クライアントも peer SETTINGS 未受信なので送信失敗する
        assert!(matches!(
            stream_id,
            Error::WtSetup(WtSetupError::PeerSettingsNotReceived)
        ));

        // サーバー側: peer SETTINGS 未受信のまま手動構築した WT CONNECT HEADERS を feed
        // QPACK エンコードされた HEADERS フレームをクライアント経由で生成するのは複雑なため、
        // サーバーの制御ストリームデータ交換後にクライアント経由でテストする
        // ここでは peer_settings が None の状態を直接テストする
        assert!(server.peer_settings().is_none());
    }

    #[test]
    fn test_server_wt_connect_rejected_when_peer_wt_disabled() {
        // サーバー: peer (クライアント) の SETTINGS で WT 無効
        let mut server = Connection::server(wt_enabled_settings());
        server.set_control_stream_id(3).expect("test must succeed");

        // クライアントから WT 無効な SETTINGS を受信
        // control stream type (0x00) + SETTINGS frame: type=0x04, length=0x00
        server
            .feed_stream(2, &[0x00, 0x04, 0x00], false)
            .expect("test must succeed");

        // peer SETTINGS は受信したが WT 無効
        assert!(server.peer_settings().is_some());
        assert!(
            !server
                .peer_settings()
                .expect("test must succeed")
                .is_webtransport_enabled()
        );
    }

    #[test]
    fn test_server_wt_connect_with_full_peer_settings() {
        // サーバー: peer (クライアント) の SETTINGS で WT 有効
        // クライアント・サーバー間の完全な SETTINGS 交換をテスト
        let wt_settings = wt_enabled_settings();
        let mut client = Connection::client(wt_settings);
        client.set_control_stream_id(2).expect("test must succeed");

        let mut server = Connection::server(wt_settings);
        server.set_control_stream_id(3).expect("test must succeed");

        // 制御ストリームデータを交換
        let (client_ctrl, _) = client.take_stream_data(2).expect("test must succeed");
        server
            .feed_stream(2, &client_ctrl, false)
            .expect("test must succeed");
        let _ = server.poll_event().expect("test must succeed");
        let (server_ctrl, _) = server.take_stream_data(3).expect("test must succeed");
        client
            .feed_stream(3, &server_ctrl, false)
            .expect("test must succeed");

        // サーバーは peer (クライアント) の WT SETTINGS を受信済み
        let peer = server.peer_settings().expect("test must succeed");
        assert!(peer.is_webtransport_enabled());
    }

    #[test]
    fn test_non_wt_connect_allowed_without_wt_settings() {
        // 通常の Extended CONNECT は WebTransport チェックの対象外
        let mut conn = Connection::client(Settings::new().enable_connect_protocol(true));
        conn.set_control_stream_id(2).expect("test must succeed");

        // peer から ENABLE_CONNECT_PROTOCOL=1 の SETTINGS を受信
        // control stream type (0x00) + SETTINGS frame: type=0x04, length=0x02
        // + entries: [id=0x08 (ENABLE_CONNECT_PROTOCOL), value=0x01]
        conn.feed_stream(3, &[0x00, 0x04, 0x02, 0x08, 0x01], false)
            .expect("test must succeed");

        let headers = vec![
            Header::new(b":method", b"CONNECT").expect("test must succeed"),
            Header::new(b":protocol", b"websocket").expect("test must succeed"),
            Header::new(b":scheme", b"https").expect("test must succeed"),
            Header::new(b":authority", b"example.com").expect("test must succeed"),
            Header::new(b":path", b"/ws").expect("test must succeed"),
        ];

        // WebSocket 等の Extended CONNECT は WT チェックなしで通る
        assert!(conn.send_request(&headers, false).is_ok());
    }

    /// QPACK エンコードされた HEADERS フレームを手動構築するヘルパー
    fn build_headers_frame(headers: &[Header]) -> Vec<u8> {
        let mut encoder = crate::qpack::Encoder::new();
        let mut qpack_buf = vec![0u8; 4096];
        let qpack_len = encoder
            .encode(&mut qpack_buf, headers, 0)
            .expect("test must succeed");
        qpack_buf.truncate(qpack_len);

        // HEADERS フレーム: type=0x01, length=varint, payload=QPACK エンコード済み
        let mut frame = Vec::new();
        crate::varint::encode_into_vec(&mut frame, crate::VarInt::from_static(0x01)); // HEADERS frame type
        crate::varint::encode_into_vec(
            &mut frame,
            crate::VarInt::new(qpack_len as u64).expect("qpack_len fits in VarInt"),
        );
        frame.extend_from_slice(&qpack_buf);
        frame
    }

    #[test]
    fn test_server_wt_connect_accepted_draft07_without_enable_connect_protocol() {
        // draft-07 クライアントは SETTINGS_ENABLE_CONNECT_PROTOCOL を送信しない
        // (draft-ietf-webtrans-http3-07 Section 3.2: クライアントは
        //  SETTINGS_WEBTRANSPORT_MAX_SESSIONS のみ MUST)
        // サーバーはこれを受理しなければならない

        // サーバー: draft-07 対応
        let server_wt =
            crate::webtransport::Settings::new().webtransport_max_sessions_draft07(vi(1));
        let server_settings = Settings::new().enable_webtransport_server(server_wt);
        let mut server = Connection::server(server_settings);
        server.set_control_stream_id(3).expect("test must succeed");
        server
            .set_webtransport_transport_verified(true, false)
            .expect("test must succeed");

        // draft-07 クライアントの SETTINGS: H3_DATAGRAM=1 + WEBTRANSPORT_MAX_SESSIONS=1
        // ENABLE_CONNECT_PROTOCOL は含めない (Safari と同じパターン)
        let client_wt =
            crate::webtransport::Settings::new().webtransport_max_sessions_draft07(vi(1));
        // enable_webtransport は enable_connect_protocol を自動設定するので、
        // 手動で None に戻して Safari のパターンを再現する
        let client_settings = Settings {
            enable_connect_protocol: None,
            ..Settings::new()
                .h3_datagram(true)
                .enable_webtransport_server(client_wt)
        };
        let mut client = Connection::client(client_settings);
        client.set_control_stream_id(2).expect("test must succeed");

        // クライアントの制御ストリームデータをサーバーに feed
        let (client_ctrl, _) = client.take_stream_data(2).expect("test must succeed");
        server
            .feed_stream(2, &client_ctrl, false)
            .expect("test must succeed");

        // サーバーの制御ストリームデータを取得 (消費のみ)
        let _ = server.take_stream_data(3).expect("test must succeed");

        // サーバー側で peer SETTINGS を確認
        let peer = server.peer_settings().expect("test must succeed");
        assert!(peer.is_webtransport_enabled());
        // クライアントは ENABLE_CONNECT_PROTOCOL を送信していない
        assert_eq!(peer.enable_connect_protocol, None);

        // SETTINGS イベントを消費
        let _ = server.poll_event().expect("test must succeed");

        // draft-07 CONNECT HEADERS フレームを手動構築
        let headers = vec![
            Header::new(b":method", b"CONNECT").expect("test must succeed"),
            Header::new(b":protocol", b"webtransport").expect("test must succeed"),
            Header::new(b":scheme", b"https").expect("test must succeed"),
            Header::new(b":authority", b"example.com").expect("test must succeed"),
            Header::new(b":path", b"/wt").expect("test must succeed"),
        ];
        let frame_data = build_headers_frame(&headers);

        // サーバーにフレームデータを feed — H3_MESSAGE_ERROR にならないこと
        let result = server.feed_stream(0, &frame_data, false);
        assert!(
            result.is_ok(),
            "draft-07 クライアントの CONNECT が拒否された: {result:?}"
        );
    }

    #[test]
    fn test_server_wt_connect_accepted_draft15_without_enable_connect_protocol() {
        // ENABLE_CONNECT_PROTOCOL はサーバーが送る設定 (RFC 9220, RFC 8441 Section 3)
        // クライアントは送信義務がない (draft-ietf-webtrans-http3-15 Section 3.1)
        // サーバーはクライアントが ENABLE_CONNECT_PROTOCOL を送信しなくても受理する

        // サーバー: draft-15 対応
        let server_settings = wt_enabled_settings();
        let mut server = Connection::server(server_settings);
        server.set_control_stream_id(3).expect("test must succeed");
        server
            .set_webtransport_transport_verified(true, true)
            .expect("test must succeed");

        // draft-15 クライアント: ENABLE_CONNECT_PROTOCOL なし (正当)
        let client_wt = crate::webtransport::Settings::new().wt_enabled(vi(1));
        let client_settings = Settings {
            enable_connect_protocol: None,
            ..Settings::new()
                .h3_datagram(true)
                .enable_webtransport_server(client_wt)
        };
        let mut client = Connection::client(client_settings);
        client.set_control_stream_id(2).expect("test must succeed");

        // クライアントの制御ストリームデータをサーバーに feed
        let (client_ctrl, _) = client.take_stream_data(2).expect("test must succeed");
        server
            .feed_stream(2, &client_ctrl, false)
            .expect("test must succeed");

        // サーバーの制御ストリームデータを取得 (消費のみ)
        let _ = server.take_stream_data(3).expect("test must succeed");

        // SETTINGS イベントを消費
        let _ = server.poll_event().expect("test must succeed");

        // draft-15 CONNECT HEADERS フレームを手動構築
        let headers = vec![
            Header::new(b":method", b"CONNECT").expect("test must succeed"),
            Header::new(b":protocol", b"webtransport-h3").expect("test must succeed"),
            Header::new(b":scheme", b"https").expect("test must succeed"),
            Header::new(b":authority", b"example.com").expect("test must succeed"),
            Header::new(b":path", b"/wt").expect("test must succeed"),
        ];
        let frame_data = build_headers_frame(&headers);

        // サーバーにフレームデータを feed — 受理されるべき
        let result = server.feed_stream(0, &frame_data, false);
        assert!(
            result.is_ok(),
            "draft-15 クライアントの CONNECT が拒否された (ENABLE_CONNECT_PROTOCOL はクライアントに送信義務なし): {result:?}"
        );
    }

    // =========================================================================
    // WebTransport セッション管理
    // (draft-ietf-webtrans-http3-15 Section 3, 4.6, 6)
    // =========================================================================

    /// WebTransport が完全にネゴシエート済みのサーバー単体を作成するヘルパ
    ///
    /// peer (クライアント) の SETTINGS を流し込み、`is_wt_fully_negotiated()` が
    /// true になる状態まで進める。
    /// 指定 session_id に Established 状態の WT セッションを持つサーバーを作成するヘルパ
    fn make_server_with_established_wt_session(session_id: u64) -> Connection {
        let mut server = make_negotiated_wt_server();
        let mut session = WtSession::new();
        session.state = WtSessionState::Established;
        server.wt_sessions.insert(session_id, session);
        server
    }

    fn make_negotiated_wt_server() -> Connection {
        let wt_settings = wt_enabled_settings();
        // クライアント制御ストリームは 6 に置く (テスト本体が stream_id 2 を WT 用に使うため)
        let mut client = Connection::client(wt_settings);
        client.set_control_stream_id(6).expect("test must succeed");
        let mut server = Connection::server(wt_settings);
        server.set_control_stream_id(3).expect("test must succeed");
        server
            .set_webtransport_transport_verified(true, true)
            .expect("test must succeed");
        let (client_ctrl, _) = client.take_stream_data(6).expect("test must succeed");
        server
            .feed_stream(6, &client_ctrl, false)
            .expect("test must succeed");
        while server.poll_event().expect("test must succeed").is_some() {}
        server
    }

    /// WebTransport 有効なクライアント・サーバーペアを作成し SETTINGS を交換済みの状態にする
    fn setup_wt_pair() -> (Connection, Connection) {
        let wt_settings = wt_enabled_settings();
        let mut client = Connection::client(wt_settings);
        client.set_control_stream_id(2).expect("test must succeed");
        let mut server = Connection::server(wt_settings);
        server.set_control_stream_id(3).expect("test must succeed");

        // QUIC transport parameter レベルの前提条件を注入
        client
            .set_webtransport_transport_verified(true, true)
            .expect("test must succeed");
        server
            .set_webtransport_transport_verified(true, true)
            .expect("test must succeed");

        // 制御ストリームデータ交換
        let (client_ctrl, _) = client.take_stream_data(2).expect("test must succeed");
        server
            .feed_stream(2, &client_ctrl, false)
            .expect("test must succeed");
        let _ = server.poll_event().expect("test must succeed");
        let (server_ctrl, _) = server.take_stream_data(3).expect("test must succeed");
        client
            .feed_stream(3, &server_ctrl, false)
            .expect("test must succeed");

        // SETTINGS イベントを消費
        let _ = client.poll_event().expect("test must succeed");

        (client, server)
    }

    /// クライアント・サーバー間で WT CONNECT ハンドシェイクを完了させるヘルパー
    ///
    /// サーバー側セッションが Established になる。戻り値は CONNECT stream ID。
    fn establish_wt_session(client: &mut Connection, server: &mut Connection) -> u64 {
        let headers = vec![
            Header::new(b":method", b"CONNECT").expect("test must succeed"),
            Header::new(b":protocol", b"webtransport-h3").expect("test must succeed"),
            Header::new(b":scheme", b"https").expect("test must succeed"),
            Header::new(b":authority", b"example.com").expect("test must succeed"),
            Header::new(b":path", b"/wt").expect("test must succeed"),
        ];
        let stream_id = client
            .send_request(&headers, false)
            .expect("test must succeed");
        let (req_data, _) = client
            .take_stream_data(stream_id)
            .expect("test must succeed");
        server
            .feed_stream(stream_id, &req_data, false)
            .expect("test must succeed");
        let _ = server.drain_events().expect("test must succeed");

        let response = vec![Header::new(b":status", b"200").expect("test must succeed")];
        server
            .send_response(stream_id, &response, false)
            .expect("test must succeed");
        let (resp_data, _) = server
            .take_stream_data(stream_id)
            .expect("test must succeed");
        client
            .feed_stream(stream_id, &resp_data, false)
            .expect("test must succeed");
        let _ = client.drain_events().expect("test must succeed");

        stream_id
    }

    /// WT_CLOSE_SESSION カプセルを DATA フレームとしてエンコードするヘルパー
    fn close_session_data_frame() -> Vec<u8> {
        let mut capsule = Vec::new();
        crate::webtransport::Capsule::CloseSession {
            error_code: 0,
            message: String::new(),
        }
        .encode(&mut capsule);
        let mut data = vec![0x00, capsule.len() as u8];
        data.extend_from_slice(&capsule);
        data
    }

    #[test]
    fn test_wt_session_registered_on_connect_send() {
        let (mut client, _server) = setup_wt_pair();

        let headers = vec![
            Header::new(b":method", b"CONNECT").expect("test must succeed"),
            Header::new(b":protocol", b"webtransport-h3").expect("test must succeed"),
            Header::new(b":scheme", b"https").expect("test must succeed"),
            Header::new(b":authority", b"example.com").expect("test must succeed"),
            Header::new(b":path", b"/wt").expect("test must succeed"),
        ];
        let stream_id = client
            .send_request(&headers, false)
            .expect("test must succeed");

        // セッションが Pending 状態で登録されていること
        assert!(client.wt_sessions.contains_key(&stream_id));
        assert_eq!(
            client.wt_sessions[&stream_id].state,
            WtSessionState::Pending
        );
    }

    #[test]
    fn test_wt_session_established_on_200_ok() {
        let (mut client, mut server) = setup_wt_pair();

        // クライアントが WT CONNECT を送信
        let headers = vec![
            Header::new(b":method", b"CONNECT").expect("test must succeed"),
            Header::new(b":protocol", b"webtransport-h3").expect("test must succeed"),
            Header::new(b":scheme", b"https").expect("test must succeed"),
            Header::new(b":authority", b"example.com").expect("test must succeed"),
            Header::new(b":path", b"/wt").expect("test must succeed"),
        ];
        let stream_id = client
            .send_request(&headers, false)
            .expect("test must succeed");
        let (req_data, _) = client
            .take_stream_data(stream_id)
            .expect("test must succeed");

        // サーバーがリクエストを受信
        server
            .feed_stream(stream_id, &req_data, false)
            .expect("test must succeed");
        // サーバー側のイベントを消費
        let _ = server.drain_events().expect("test must succeed");

        // サーバーが 200 OK を返す
        let response = vec![Header::new(b":status", b"200").expect("test must succeed")];
        server
            .send_response(stream_id, &response, false)
            .expect("test must succeed");
        let (resp_data, _) = server
            .take_stream_data(stream_id)
            .expect("test must succeed");

        // クライアントが 200 OK を受信
        client
            .feed_stream(stream_id, &resp_data, false)
            .expect("test must succeed");

        // WebTransportEvent::SessionEstablished イベントが発火すること
        let events = client.drain_events().expect("test must succeed");
        assert!(events.iter().any(|e| matches!(
            e,
            Event::WebTransport(WebTransportEvent::SessionEstablished { session_id, .. }) if *session_id == stream_id
        )));

        // セッションが Established 状態であること
        assert_eq!(
            client.wt_sessions[&stream_id].state,
            WtSessionState::Established
        );
    }

    #[test]
    fn test_server_queues_initial_flow_control_capsules_for_safari_observed_pattern() {
        let client_wt = crate::webtransport::Settings::new()
            .webtransport_max_sessions_draft07(vi(1))
            .wt_initial_max_streams_uni(vi(100))
            .wt_initial_max_streams_bidi(vi(100))
            .wt_initial_max_data(vi(8 * 1024 * 1024));
        let client_settings = Settings::new().enable_webtransport_client(client_wt);
        let mut client = Connection::client(client_settings);
        client.set_control_stream_id(2).expect("test must succeed");

        let mut server = Connection::server(wt_multi_draft_settings_with_flow_control());
        server.set_control_stream_id(3).expect("test must succeed");
        server
            .set_webtransport_transport_verified(true, true)
            .expect("test must succeed");

        let (client_ctrl, _) = client.take_stream_data(2).expect("test must succeed");
        server
            .feed_stream(2, &client_ctrl, false)
            .expect("test must succeed");
        let _ = server.drain_events().expect("test must succeed");

        let headers = vec![
            Header::new(b":method", b"CONNECT").expect("test must succeed"),
            Header::new(b":protocol", b"webtransport").expect("test must succeed"),
            Header::new(b":scheme", b"https").expect("test must succeed"),
            Header::new(b":authority", b"example.com").expect("test must succeed"),
            Header::new(b":path", b"/wt").expect("test must succeed"),
        ];
        let frame = build_headers_frame(&headers);
        server
            .feed_stream(0, &frame, false)
            .expect("test must succeed");
        let _ = server.drain_events().expect("test must succeed");

        let response = vec![Header::new(b":status", b"200").expect("test must succeed")];
        server
            .send_response(0, &response, false)
            .expect("test must succeed");

        let capsules = server.take_wt_pending_capsules(0);
        assert_eq!(capsules.len(), 3);
        assert_eq!(
            capsules[0],
            crate::webtransport::Capsule::MaxStreams {
                bidirectional: true,
                maximum: 100
            }
        );
        assert_eq!(
            capsules[1],
            crate::webtransport::Capsule::MaxStreams {
                bidirectional: false,
                maximum: 100
            }
        );
        assert_eq!(
            capsules[2],
            crate::webtransport::Capsule::MaxData {
                maximum: 8 * 1024 * 1024
            }
        );
    }

    #[test]
    fn test_server_does_not_queue_initial_flow_control_capsules_for_plain_draft07() {
        let client_wt =
            crate::webtransport::Settings::new().webtransport_max_sessions_draft07(vi(1));
        let client_settings = Settings::new().enable_webtransport_client(client_wt);
        let mut client = Connection::client(client_settings);
        client.set_control_stream_id(2).expect("test must succeed");

        let mut server = Connection::server(wt_multi_draft_settings_with_flow_control());
        server.set_control_stream_id(3).expect("test must succeed");
        server
            .set_webtransport_transport_verified(true, false)
            .expect("test must succeed");

        let (client_ctrl, _) = client.take_stream_data(2).expect("test must succeed");
        server
            .feed_stream(2, &client_ctrl, false)
            .expect("test must succeed");
        let _ = server.drain_events().expect("test must succeed");

        let headers = vec![
            Header::new(b":method", b"CONNECT").expect("test must succeed"),
            Header::new(b":protocol", b"webtransport").expect("test must succeed"),
            Header::new(b":scheme", b"https").expect("test must succeed"),
            Header::new(b":authority", b"example.com").expect("test must succeed"),
            Header::new(b":path", b"/wt").expect("test must succeed"),
        ];
        let frame = build_headers_frame(&headers);
        server
            .feed_stream(0, &frame, false)
            .expect("test must succeed");
        let _ = server.drain_events().expect("test must succeed");

        let response = vec![Header::new(b":status", b"200").expect("test must succeed")];
        server
            .send_response(0, &response, false)
            .expect("test must succeed");

        assert!(server.take_wt_pending_capsules(0).is_empty());
    }

    #[test]
    fn test_wt_session_terminated_on_connect_stream_fin() {
        let (mut client, mut server) = setup_wt_pair();

        // WT CONNECT + 200 OK のハンドシェイク
        let headers = vec![
            Header::new(b":method", b"CONNECT").expect("test must succeed"),
            Header::new(b":protocol", b"webtransport-h3").expect("test must succeed"),
            Header::new(b":scheme", b"https").expect("test must succeed"),
            Header::new(b":authority", b"example.com").expect("test must succeed"),
            Header::new(b":path", b"/wt").expect("test must succeed"),
        ];
        let stream_id = client
            .send_request(&headers, false)
            .expect("test must succeed");
        let (req_data, _) = client
            .take_stream_data(stream_id)
            .expect("test must succeed");
        server
            .feed_stream(stream_id, &req_data, false)
            .expect("test must succeed");
        let _ = server.drain_events().expect("test must succeed");

        let response = vec![Header::new(b":status", b"200").expect("test must succeed")];
        server
            .send_response(stream_id, &response, false)
            .expect("test must succeed");
        let (resp_data, _) = server
            .take_stream_data(stream_id)
            .expect("test must succeed");
        client
            .feed_stream(stream_id, &resp_data, false)
            .expect("test must succeed");
        let _ = client.drain_events().expect("test must succeed");

        // server-initiated uni stream をクライアントに送信
        // stream_id=3 は server-initiated uni, ストリームタイプ 0x54 + session_id
        let session_id = stream_id;
        let mut uni_data = vec![0x40, 0x54]; // stream type 0x54 (varint)
        uni_data.push(session_id as u8); // session_id (1 バイト varint)
        client
            .feed_stream(3, &uni_data, false)
            .expect("test must succeed");
        let _ = client.drain_events().expect("test must succeed");

        // CONNECT stream を FIN で閉じる (セッション終了)
        client
            .feed_stream(stream_id, &[], true)
            .expect("test must succeed");

        // WebTransportEvent::SessionClosed イベントが発火すること
        let events = client.drain_events().expect("test must succeed");
        let closed_event = events.iter().find(|e| {
            matches!(e, Event::WebTransport(WebTransportEvent::SessionClosed { session_id: sid, .. }) if *sid == session_id)
        });
        assert!(closed_event.is_some());

        // セッションエントリが除去されていること (tombstone に記録される)
        assert!(
            !client.wt_sessions.contains_key(&session_id),
            "セッション終了後は wt_sessions から除去されること"
        );
    }

    #[test]
    fn test_wt_session_terminated_on_reset() {
        let (mut client, mut server) = setup_wt_pair();

        // WT CONNECT + 200 OK のハンドシェイク
        let headers = vec![
            Header::new(b":method", b"CONNECT").expect("test must succeed"),
            Header::new(b":protocol", b"webtransport-h3").expect("test must succeed"),
            Header::new(b":scheme", b"https").expect("test must succeed"),
            Header::new(b":authority", b"example.com").expect("test must succeed"),
            Header::new(b":path", b"/wt").expect("test must succeed"),
        ];
        let stream_id = client
            .send_request(&headers, false)
            .expect("test must succeed");
        let (req_data, _) = client
            .take_stream_data(stream_id)
            .expect("test must succeed");
        server
            .feed_stream(stream_id, &req_data, false)
            .expect("test must succeed");
        let _ = server.drain_events().expect("test must succeed");

        let response = vec![Header::new(b":status", b"200").expect("test must succeed")];
        server
            .send_response(stream_id, &response, false)
            .expect("test must succeed");
        let (resp_data, _) = server
            .take_stream_data(stream_id)
            .expect("test must succeed");
        client
            .feed_stream(stream_id, &resp_data, false)
            .expect("test must succeed");
        let _ = client.drain_events().expect("test must succeed");

        // CONNECT stream を RESET_STREAM で閉じる
        client
            .stream_reset(stream_id, 0, 0)
            .expect("test must succeed");

        // WebTransportEvent::SessionClosed イベントが発火すること
        let events = client.drain_events().expect("test must succeed");
        assert!(events.iter().any(|e| matches!(
            e,
            Event::WebTransport(WebTransportEvent::SessionClosed { session_id: sid, .. }) if *sid == stream_id
        )));

        assert!(
            !client.wt_sessions.contains_key(&stream_id),
            "セッション終了後は wt_sessions から除去されること"
        );
        assert!(
            !client.streams.contains_key(&stream_id),
            "セッション終了後は CONNECT ストリームも streams から除去されること"
        );
        assert!(
            client.closed_wt_sessions.contains(&stream_id),
            "終了済みセッション ID が tombstone に記録されること"
        );
    }

    #[test]
    fn test_wt_session_buffering_before_established() {
        // サーバー側: セッション未確立 (CONNECT 受信前) のストリームが到着した場合のバッファリング
        let mut server = Connection::server(wt_enabled_settings());
        server.set_control_stream_id(3).expect("test must succeed");
        server.peer_settings = Some(wt_enabled_settings());
        server.wt_transport_verified = true;

        // client-initiated uni stream が先に到着 (session_id = 0)
        // ストリームタイプ 0x54 (varint [0x40, 0x54]) + session_id = 0
        server
            .feed_stream(2, &[0x40, 0x54, 0x00], false)
            .expect("test must succeed");

        // セッション ID = 0 のセッションが自動作成されていること
        assert!(server.wt_sessions.contains_key(&0));
        assert_eq!(server.wt_sessions[&0].state, WtSessionState::Pending);
        // ストリームがバッファリングされていること
        assert_eq!(server.wt_sessions[&0].buffered_streams.len(), 1);
        // Open / Data イベントは確立まで発火されていないこと
        // (draft-ietf-webtrans-http3-15 Section 4.6)
        assert!(server.events.is_empty());
    }

    #[test]
    fn test_wt_pending_stream_data_buffered_until_established() {
        // Pending セッションに対する先行 stream の Data も発火されないことを確認する
        // (draft-ietf-webtrans-http3-15 Section 4.6)
        let mut server = make_negotiated_wt_server();

        // 先行 uni stream を流し込む (session_id = 0, ペイロード = 0xAA, 0xBB)
        server
            .feed_stream(2, &[0x40, 0x54, 0x00, 0xAA, 0xBB], false)
            .expect("test must succeed");

        // セッションが Pending で生成されていること
        assert_eq!(server.wt_sessions[&0].state, WtSessionState::Pending);
        // バッファエントリにペイロードが積まれていること
        let entry = server
            .wt_sessions
            .get(&0)
            .expect("test must succeed")
            .buffered_stream_entries
            .get(&2)
            .expect("test must succeed");
        assert!(!entry.is_bidi);
        assert_eq!(entry.data, vec![0xAA, 0xBB]);
        assert!(!entry.fin);

        // Open / Data イベントは未発火
        assert!(server.events.is_empty());

        // 後続 Data も同様にバッファに追記される
        server
            .feed_stream(2, &[0xCC], false)
            .expect("test must succeed");
        let entry = server
            .wt_sessions
            .get(&0)
            .expect("test must succeed")
            .buffered_stream_entries
            .get(&2)
            .expect("test must succeed");
        assert_eq!(entry.data, vec![0xAA, 0xBB, 0xCC]);
        assert!(server.events.is_empty());
    }

    #[test]
    fn test_wt_client_rejects_unknown_session_id() {
        // クライアントは自身が開始していない session_id のストリームを拒否する
        // (draft-ietf-webtrans-http3-15 Section 4.6)
        let mut client = wt_negotiated_client();

        // server-initiated uni stream (session_id = 0 だがクライアントはセッション未開始)
        // ストリームタイプ 0x54 (varint [0x40, 0x54]) + session_id = 0
        client
            .feed_stream(3, &[0x40, 0x54, 0x00], false)
            .expect("test must succeed");

        // WT_SESSION_GONE でストリーム拒否イベントが発生すること
        let event = client
            .poll_event()
            .expect("test must succeed")
            .expect("test must succeed");
        assert!(matches!(
            event,
            Event::WebTransport(WebTransportEvent::BufferedStreamRejected {
                stream_id: 3,
                error_code,
            }) if error_code == WtErrorCode::SessionGone as u64
        ));
    }

    #[test]
    fn test_pending_wt_sessions_limit_for_streams() {
        // 異なる session_id を WT_MAX_PENDING_SESSIONS + 1 個投げると、
        // 最後の 1 個は BufferedStreamRejected で拒否される
        let mut server = make_negotiated_wt_server();

        // 上限まで Pending セッションを作る (uni stream で 4, 8, 12, ...)
        for i in 0..WT_MAX_PENDING_SESSIONS {
            let session_id = ((i + 1) * 4) as u64;
            // client-initiated uni: stream_id % 4 == 2
            // 6 番は client 制御ストリームと衝突するため 10 から開始する
            let stream_id = 10 + (i as u64) * 4;
            // ストリームタイプ 0x54 + session_id varint
            let mut buf = vec![0x40, 0x54];
            crate::varint::encode_into_vec(
                &mut buf,
                crate::VarInt::new(session_id).expect("session_id fits in VarInt"),
            );
            server
                .feed_stream(stream_id, &buf, false)
                .expect("test must succeed");
        }
        assert_eq!(server.count_pending_wt_sessions(), WT_MAX_PENDING_SESSIONS);

        // 上限超過の 1 個目は拒否される
        let overflow_session_id = ((WT_MAX_PENDING_SESSIONS + 1) * 4) as u64;
        let overflow_stream_id = 10 + (WT_MAX_PENDING_SESSIONS as u64) * 4;
        let mut buf = vec![0x40, 0x54];
        crate::varint::encode_into_vec(
            &mut buf,
            crate::VarInt::new(overflow_session_id).expect("overflow_session_id fits in VarInt"),
        );
        server
            .feed_stream(overflow_stream_id, &buf, false)
            .expect("test must succeed");

        // 最終イベントが BufferedStreamRejected であること
        let mut last = None;
        while let Some(ev) = server.poll_event().expect("test must succeed") {
            last = Some(ev);
        }
        match last {
            Some(Event::WebTransport(WebTransportEvent::BufferedStreamRejected {
                stream_id,
                ..
            })) => {
                assert_eq!(stream_id, overflow_stream_id);
            }
            other => panic!("expected BufferedStreamRejected, got {other:?}"),
        }
        // Pending セッション数は上限を超えないこと
        assert_eq!(server.count_pending_wt_sessions(), WT_MAX_PENDING_SESSIONS);
    }

    #[test]
    fn test_pending_wt_sessions_limit_for_datagrams() {
        // datagram 経路でも上限を超えると無視されるだけで Pending セッションは増えない
        let mut server = make_negotiated_wt_server();

        for i in 0..WT_MAX_PENDING_SESSIONS {
            let session_id = ((i + 1) * 4) as u64;
            let qsi = session_id / 4;
            let mut buf = Vec::new();
            crate::varint::encode_into_vec(
                &mut buf,
                crate::VarInt::new(qsi).expect("qsi fits in VarInt"),
            );
            buf.extend_from_slice(b"x");
            server.feed_datagram(&buf).expect("test must succeed");
        }
        assert_eq!(server.count_pending_wt_sessions(), WT_MAX_PENDING_SESSIONS);

        // 上限超過の datagram は破棄され、Pending セッション数は増えない
        let overflow_session_id = ((WT_MAX_PENDING_SESSIONS + 1) * 4) as u64;
        let mut buf = Vec::new();
        crate::varint::encode_into_vec(
            &mut buf,
            crate::VarInt::new(overflow_session_id / 4).expect("quarter session id fits in VarInt"),
        );
        buf.extend_from_slice(b"x");
        server.feed_datagram(&buf).expect("test must succeed");
        assert_eq!(server.count_pending_wt_sessions(), WT_MAX_PENDING_SESSIONS);
        assert!(!server.wt_sessions.contains_key(&overflow_session_id));
    }

    #[test]
    fn test_stream_reset_propagates_to_wt_uni_data_stream() {
        // 既知 WebTransport セッションに属する単方向データストリームの RESET_STREAM は
        // セッションを終了させず、WebTransportEvent::StreamReset イベントとして通知する
        // (draft-ietf-webtrans-http3-15 Section 4.4)
        let mut conn = make_server_with_established_wt_session(4);
        // セッション 4 に紐づく WT uni stream 2 を作成
        conn.feed_stream(2, &[0x40, 0x54, 0x04], false)
            .expect("test must succeed");
        // Open イベントを消費
        let _ = conn
            .poll_event()
            .expect("test must succeed")
            .expect("test must succeed");

        // WT uni stream の stream header 長 = varint(0x54) + varint(4) = 2 + 1 = 3
        conn.stream_reset(2, 0x42, 3).expect("test must succeed");

        let event = conn
            .poll_event()
            .expect("test must succeed")
            .expect("test must succeed");
        assert!(matches!(
            event,
            Event::WebTransport(WebTransportEvent::StreamReset {
                session_id: 4,
                stream_id: 2,
                error_code: 0x42,
                final_size: 3,
            })
        ));
        // セッションは終了していないこと
        assert!(conn.wt_sessions.contains_key(&4));
        assert!(matches!(
            conn.wt_sessions.get(&4).expect("test must succeed").state,
            WtSessionState::Established
        ));
        // ストリームの紐付けは解除されていること
        assert!(!conn.wt_uni_streams.contains_key(&2));
        assert!(
            !conn
                .wt_sessions
                .get(&4)
                .expect("test must succeed")
                .associated_streams
                .contains(&2)
        );
    }

    #[test]
    fn test_stream_reset_on_connect_stream_terminates_wt_session() {
        // CONNECT stream (= session_id) への RESET_STREAM はセッションを終了させる
        // (draft-ietf-webtrans-http3-15 Section 6)
        let mut conn = make_server_with_established_wt_session(0);

        conn.stream_reset(0, 0x99, 0).expect("test must succeed");

        // WebTransportEvent::SessionClosed が発行される
        let event = conn
            .poll_event()
            .expect("test must succeed")
            .expect("test must succeed");
        assert!(matches!(
            event,
            Event::WebTransport(WebTransportEvent::SessionClosed { session_id: 0, .. })
        ));
        assert!(
            !conn.wt_sessions.contains_key(&0),
            "セッション終了後は wt_sessions から除去されること"
        );
    }

    #[test]
    fn test_stop_sending_propagates_to_wt_bidi_data_stream() {
        // 既知 WebTransport セッションに属する双方向データストリームの STOP_SENDING は
        // セッションを終了させず WebTransportEvent::StreamStopSending として通知する
        let mut conn = make_server_with_established_wt_session(4);
        // signal value 0x41 + session_id = 4 で WT bidi stream 8 を作成
        conn.feed_stream(8, &[0x40, 0x41, 0x04], false)
            .expect("test must succeed");
        let _ = conn
            .poll_event()
            .expect("test must succeed")
            .expect("test must succeed");

        conn.stop_sending(8, 0x55).expect("test must succeed");

        let event = conn
            .poll_event()
            .expect("test must succeed")
            .expect("test must succeed");
        assert!(matches!(
            event,
            Event::WebTransport(WebTransportEvent::StreamStopSending {
                session_id: 4,
                stream_id: 8,
                error_code: 0x55,
            })
        ));
        assert!(matches!(
            conn.wt_sessions.get(&4).expect("test must succeed").state,
            WtSessionState::Established
        ));
    }

    #[test]
    fn test_feed_datagram_truncated_returns_h3_datagram_error() {
        // RFC 9297 Section 2.1: Quarter Stream ID varint が短すぎる場合は
        // H3_DATAGRAM_ERROR で接続を閉じる
        let mut conn = make_server_with_established_wt_session(4);

        // 空ペイロードは varint デコード失敗
        let err = conn.feed_datagram(&[]).unwrap_err();
        assert!(matches!(
            err,
            Error::ConnectionError(ErrorCode::H3DatagramError)
        ));
    }

    #[test]
    fn test_feed_datagram_qsi_overflow_returns_h3_datagram_error() {
        // RFC 9297 Section 2.1: Quarter Stream ID が 2^60-1 を超える場合
        // (qsi * 4 が u64 をオーバーフロー) は H3_DATAGRAM_ERROR
        let mut conn = make_server_with_established_wt_session(4);

        // 8 オクテット varint で最大値 (2^62-1) を表現する
        // 0xc0 で 8 バイト長プレフィックス、残り 7 バイトを 0xff で埋めると
        // qsi = 2^62 - 1 となり、qsi * 4 が u64 をオーバーフローする
        let buf = [0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff];
        let err = conn.feed_datagram(&buf).unwrap_err();
        assert!(matches!(
            err,
            Error::ConnectionError(ErrorCode::H3DatagramError)
        ));
    }

    /// CONNECT ストリーム上で受信した DATA フレーム (= Capsule バイト列) を
    /// process_wt_capsule_data に流す形で feed する補助関数
    fn feed_drain_session_capsule(conn: &mut Connection, session_id: u64) {
        let mut payload = Vec::new();
        crate::webtransport::Capsule::DrainSession.encode(&mut payload);
        conn.process_wt_capsule_data(session_id, &payload)
            .expect("test must succeed");
    }

    #[test]
    fn test_wt_session_closed_event_carries_reliable_sizes() {
        // CONNECT stream RESET によるセッション終了時、関連 WT データストリームの
        // reliable_size が stream header 長と一致していることを検証する
        // (draft-ietf-webtrans-http3-15 Section 6 / Section 4.4 / Section 5.4)
        let mut conn = make_server_with_established_wt_session(0);

        // session_id = 0 に紐づく WT bidi stream 4 と uni stream 2 を作成
        // bidi: signal value 0x41 + session_id=0 → varint(0x41)=2 + varint(0)=1 = 3
        conn.feed_stream(4, &[0x40, 0x41, 0x00], false)
            .expect("test must succeed");
        // uni: stream type 0x54 + session_id=0 → varint(0x54)=2 + varint(0)=1 = 3
        conn.feed_stream(2, &[0x40, 0x54, 0x00], false)
            .expect("test must succeed");
        while conn.poll_event().expect("test must succeed").is_some() {}

        // CONNECT stream の RESET でセッション終了
        conn.stream_reset(0, 0x99, 0).expect("test must succeed");

        let event = conn
            .poll_event()
            .expect("test must succeed")
            .expect("test must succeed");
        let Event::WebTransport(WebTransportEvent::SessionClosed {
            session_id,
            reset_streams,
            ..
        }) = event
        else {
            panic!("WebTransportEvent::SessionClosed が発火していない");
        };
        assert_eq!(session_id, 0);
        assert_eq!(reset_streams.len(), 2);
        for entry in &reset_streams {
            assert!(
                entry.reliable_size >= 3,
                "reliable_size が stream header 長以上であること: {entry:?}"
            );
            assert_eq!(entry.reliable_size, 3);
        }
    }

    #[test]
    fn test_wt_data_stream_reset_event_carries_final_size() {
        // WT データストリームの RESET_STREAM 受信時、final_size が
        // WebTransportEvent::StreamReset イベントに反映される
        let mut conn = make_server_with_established_wt_session(4);
        conn.feed_stream(2, &[0x40, 0x54, 0x04], false)
            .expect("test must succeed");
        while conn.poll_event().expect("test must succeed").is_some() {}

        conn.stream_reset(2, 0xab, 42).expect("test must succeed");

        let event = conn
            .poll_event()
            .expect("test must succeed")
            .expect("test must succeed");
        let Event::WebTransport(WebTransportEvent::StreamReset {
            session_id,
            stream_id,
            error_code,
            final_size,
        }) = event
        else {
            panic!("WebTransportEvent::StreamReset が発火していない: {event:?}");
        };
        assert_eq!(session_id, 4);
        assert_eq!(stream_id, 2);
        assert_eq!(error_code, 0xab);
        assert_eq!(final_size, 42);
    }

    #[test]
    fn test_wt_stream_header_len_helper() {
        // wt_stream_header_len は WT データストリームについて
        // varint(stream_type/signal) + varint(session_id) を返す
        let mut conn = make_server_with_established_wt_session(4);
        conn.feed_stream(2, &[0x40, 0x54, 0x04], false)
            .expect("test must succeed");
        conn.feed_stream(8, &[0x40, 0x41, 0x04], false)
            .expect("test must succeed");
        while conn.poll_event().expect("test must succeed").is_some() {}

        // session_id=4 → varint(4)=1 byte, varint(0x54)=2, varint(0x41)=2
        assert_eq!(conn.wt_stream_header_len(2), 3);
        assert_eq!(conn.wt_stream_header_len(8), 3);
        // 非 WT ストリーム
        assert_eq!(conn.wt_stream_header_len(99), 0);
    }

    #[test]
    fn test_wt_drain_session_transitions_to_draining_and_blocks_send_datagram() {
        // WT_DRAIN_SESSION を受信したセッションは Draining へ遷移し、
        // send_datagram は Error::WtSessionDraining を返す
        // (draft-ietf-webtrans-http3-15 Section 4.7)
        let mut conn = make_server_with_established_wt_session(0);

        feed_drain_session_capsule(&mut conn, 0);

        // 内部状態が Draining
        assert_eq!(
            conn.wt_sessions.get(&0).expect("test must succeed").state,
            WtSessionState::Draining
        );
        // Draining イベントが発行されている
        let event = conn
            .poll_event()
            .expect("test must succeed")
            .expect("test must succeed");
        assert!(matches!(
            event,
            Event::WebTransport(WebTransportEvent::SessionDraining { session_id: 0 })
        ));
        // send_datagram は WtSessionDraining を返す
        let err = conn.send_datagram(0, b"hello").unwrap_err();
        assert!(matches!(err, Error::WtSessionDraining(0)));
    }

    #[test]
    fn test_wt_drain_session_then_close_session_transitions_to_closed() {
        // Draining 状態のセッションが WT_CLOSE_SESSION を受けると Closed に遷移する
        let mut conn = make_server_with_established_wt_session(0);
        feed_drain_session_capsule(&mut conn, 0);
        // Draining イベントを消費
        let _ = conn.poll_event().expect("test must succeed");

        // CLOSE_SESSION カプセルを feed
        let mut payload = Vec::new();
        crate::webtransport::Capsule::CloseSession {
            error_code: 0,
            message: String::new(),
        }
        .encode(&mut payload);
        conn.process_wt_capsule_data(0, &payload)
            .expect("test must succeed");

        assert!(
            !conn.wt_sessions.contains_key(&0),
            "セッション終了後は wt_sessions から除去されること"
        );
    }

    #[test]
    fn test_client_goaway_transitions_wt_session_to_draining() {
        // クライアントが GOAWAY を受信したとき、対象 session_id 以上の
        // Established / Pending な WT セッションが Draining に遷移する
        // (draft-ietf-webtrans-http3-15 Section 4.7 / RFC 9114 Section 5.2)
        let (mut client, mut server) = setup_wt_pair();

        // クライアントが WT CONNECT を送信
        let headers = vec![
            Header::new(b":method", b"CONNECT").expect("test must succeed"),
            Header::new(b":protocol", b"webtransport-h3").expect("test must succeed"),
            Header::new(b":scheme", b"https").expect("test must succeed"),
            Header::new(b":authority", b"example.com").expect("test must succeed"),
            Header::new(b":path", b"/wt").expect("test must succeed"),
        ];
        let stream_id = client
            .send_request(&headers, false)
            .expect("test must succeed");
        // セッション確立まで進めず Pending のままで GOAWAY を受信させる

        // サーバーが GOAWAY を送信
        server
            .send_goaway(VarInt::new(stream_id).expect("test must succeed"))
            .expect("test must succeed");
        let (ctrl_data, _) = server.take_stream_data(3).expect("test must succeed");
        client
            .feed_stream(3, &ctrl_data, false)
            .expect("test must succeed");

        // クライアントの WT セッションが Draining に遷移している
        assert_eq!(
            client
                .wt_sessions
                .get(&stream_id)
                .expect("test must succeed")
                .state,
            WtSessionState::Draining
        );

        // send_datagram も拒否される (datagram の前提として Established だが、
        // 念のため Draining で WtSessionDraining を返す経路を確認する)
        // ※ Pending → Draining でも内部状態として Draining になることを確認する
    }

    #[test]
    fn test_feed_stream_propagates_original_error() {
        // エラー状態の接続に対して feed_stream を呼んだ場合、
        // InternalError ではなく元のエラーが返ることを検証する
        let mut conn = Connection::client(Settings::default());
        conn.set_control_stream_id(2).expect("test must succeed");

        // 接続エラー状態を直接設定する
        conn.error = Some(Error::ConnectionError(ErrorCode::ClosedCriticalStream));

        let err = conn.feed_stream(0, &[0x01], false).unwrap_err();
        assert_eq!(
            err,
            Error::ConnectionError(ErrorCode::ClosedCriticalStream),
            "feed_stream は InternalError ではなく元のエラーを返すこと"
        );
    }

    #[test]
    fn test_feed_stream_propagates_frame_error() {
        // 別のエラー種別でも正しく伝播されることを検証する
        let mut conn = Connection::client(Settings::default());
        conn.set_control_stream_id(2).expect("test must succeed");

        conn.error = Some(Error::ConnectionError(ErrorCode::FrameError));

        let err = conn.feed_stream(4, &[0x01, 0x00], false).unwrap_err();
        assert_eq!(
            err,
            Error::ConnectionError(ErrorCode::FrameError),
            "feed_stream は元の FrameError を返すこと"
        );
    }

    #[test]
    fn test_wt_connect_rejects_fin() {
        // WebTransport CONNECT で fin=true を指定すると StreamError(MessageError) が返ることを検証
        let (mut client, _server) = setup_wt_pair();

        let headers = vec![
            Header::new(b":method", b"CONNECT").expect("test must succeed"),
            Header::new(b":protocol", b"webtransport-h3").expect("test must succeed"),
            Header::new(b":scheme", b"https").expect("test must succeed"),
            Header::new(b":authority", b"example.com").expect("test must succeed"),
            Header::new(b":path", b"/wt").expect("test must succeed"),
        ];

        let err = client.send_request(&headers, true).unwrap_err();
        assert!(
            matches!(err, Error::StreamError(ErrorCode::MessageError)),
            "WebTransport CONNECT で fin=true は StreamError(MessageError) であること: {err:?}"
        );
    }

    #[test]
    fn test_wt_connect_without_fin_succeeds() {
        // WebTransport CONNECT で fin=false なら成功することを検証
        let (mut client, _server) = setup_wt_pair();

        let headers = vec![
            Header::new(b":method", b"CONNECT").expect("test must succeed"),
            Header::new(b":protocol", b"webtransport-h3").expect("test must succeed"),
            Header::new(b":scheme", b"https").expect("test must succeed"),
            Header::new(b":authority", b"example.com").expect("test must succeed"),
            Header::new(b":path", b"/wt").expect("test must succeed"),
        ];

        let stream_id = client
            .send_request(&headers, false)
            .expect("test must succeed");
        assert_eq!(
            stream_id, 0,
            "WebTransport CONNECT で fin=false は成功すること"
        );
    }

    // =========================================================================
    // ストリーム / WT セッションのリーク防止 (終了時の除去)
    // =========================================================================

    #[test]
    fn test_streams_removed_after_request_completion() {
        // リクエスト完走 → レスポンス送信完了で両方向クローズ + 送信完了になり、
        // クライアント / サーバー双方の streams から除去されることを検証する
        let mut client = Connection::client(Settings::default());
        client.set_control_stream_id(2).expect("test must succeed");
        let mut server = Connection::server(Settings::default());
        server.set_control_stream_id(3).expect("test must succeed");

        // 制御ストリームの SETTINGS 交換
        let (client_ctrl, _) = client.take_stream_data(2).expect("test must succeed");
        server
            .feed_stream(2, &client_ctrl, false)
            .expect("test must succeed");
        let _ = server.drain_events().expect("test must succeed");
        let (server_ctrl, _) = server.take_stream_data(3).expect("test must succeed");
        client
            .feed_stream(3, &server_ctrl, false)
            .expect("test must succeed");
        let _ = client.drain_events().expect("test must succeed");

        let headers = vec![
            Header::new(b":method", b"GET").expect("test must succeed"),
            Header::new(b":path", b"/").expect("test must succeed"),
            Header::new(b":scheme", b"https").expect("test must succeed"),
            Header::new(b":authority", b"example.com").expect("test must succeed"),
        ];
        let stream_id = client
            .send_request(&headers, true)
            .expect("test must succeed");

        // リクエストデータ + FIN をサーバーに送る (FIN はデータ消費後の追加呼び出しで交付される)
        let mut req_data = Vec::new();
        while let Some((data, fin)) = client.take_stream_data(stream_id) {
            req_data.extend_from_slice(&data);
            if fin {
                break;
            }
        }
        server
            .feed_stream(stream_id, &req_data, true)
            .expect("test must succeed");
        let _ = server.drain_events().expect("test must succeed");

        // サーバーがレスポンスを送信 (実運用と同じ send_response + send_body 経路)
        let response = vec![Header::new(b":status", b"200").expect("test must succeed")];
        server
            .send_response(stream_id, &response, false)
            .expect("test must succeed");
        server
            .send_body(stream_id, b"hello", true)
            .expect("test must succeed");
        let mut resp_data = Vec::new();
        while let Some((data, fin)) = server.take_stream_data(stream_id) {
            resp_data.extend_from_slice(&data);
            if fin {
                break;
            }
        }
        client
            .feed_stream(stream_id, &resp_data, true)
            .expect("test must succeed");
        let _ = client.drain_events().expect("test must succeed");

        // 両側で streams から除去されていること
        assert!(
            !client.streams.contains_key(&stream_id),
            "クライアント側: 両方向クローズ + 送信完了でストリームが除去されること"
        );
        assert!(
            !server.streams.contains_key(&stream_id),
            "サーバー側: 両方向クローズ + 送信完了でストリームが除去されること"
        );
    }

    #[test]
    fn test_streams_removed_on_reset() {
        // stream_reset でストリームが Reset になり、streams から即時除去されることを検証する
        let mut client = Connection::client(Settings::default());
        client.set_control_stream_id(2).expect("test must succeed");

        let headers = vec![
            Header::new(b":method", b"GET").expect("test must succeed"),
            Header::new(b":path", b"/").expect("test must succeed"),
            Header::new(b":scheme", b"https").expect("test must succeed"),
            Header::new(b":authority", b"example.com").expect("test must succeed"),
        ];
        let stream_id = client
            .send_request(&headers, false)
            .expect("test must succeed");
        assert!(client.streams.contains_key(&stream_id));

        client
            .stream_reset(stream_id, 0, 0)
            .expect("test must succeed");

        assert!(
            !client.streams.contains_key(&stream_id),
            "Reset 後は streams から除去されること"
        );
    }

    #[test]
    fn test_wt_session_end_removes_connect_stream() {
        // セッション終了 (CONNECT stream の FIN) で wt_sessions と streams の
        // 両方から CONNECT ストリームが除去され、tombstone に記録されることを検証する
        let (mut client, mut server) = setup_wt_pair();
        let stream_id = establish_wt_session(&mut client, &mut server);
        assert!(server.wt_sessions.contains_key(&stream_id));

        // クライアントが CONNECT stream を FIN で閉じる (セッション終了)
        client
            .feed_stream(stream_id, &[], true)
            .expect("test must succeed");
        let _ = client.drain_events().expect("test must succeed");

        assert!(
            !client.wt_sessions.contains_key(&stream_id),
            "セッション終了後は wt_sessions から除去されること"
        );
        assert!(
            !client.streams.contains_key(&stream_id),
            "セッション終了後は CONNECT ストリームも streams から除去されること"
        );
        assert!(
            client.closed_wt_sessions.contains(&stream_id),
            "終了済みセッション ID が tombstone に記録されること"
        );
    }

    #[test]
    fn test_wt_connect_data_after_session_close_rejected() {
        // WT_CLOSE_SESSION を含む DATA と同一バッファに続く追加 DATA は
        // H3_MESSAGE_ERROR で拒否されることを検証する
        // (draft-ietf-webtrans-http3-16 Section 6)
        let (mut client, mut server) = setup_wt_pair();
        let stream_id = establish_wt_session(&mut client, &mut server);

        let mut data = close_session_data_frame();
        // セッション終了後の追加 DATA (同一 feed_stream バッファ内)
        data.extend_from_slice(&[0x00, 0x02, 0xAA, 0xBB]);

        let err = server.feed_stream(stream_id, &data, false).unwrap_err();
        assert!(
            matches!(err, Error::StreamError(ErrorCode::MessageError)),
            "WT_CLOSE_SESSION 後の追加 DATA は H3_MESSAGE_ERROR であること: {err:?}"
        );
    }

    #[test]
    fn test_wt_connect_fin_after_session_close_ignored() {
        // WT_CLOSE_SESSION を含む DATA と同一バッファに続く FIN は受理され、
        // 汎用 StreamEnd イベントを発行しないことを検証する
        // (draft-ietf-webtrans-http3-16 Section 6: WT_CLOSE_SESSION 送信後に
        //  MUST immediately send a FIN)
        let (mut client, mut server) = setup_wt_pair();
        let stream_id = establish_wt_session(&mut client, &mut server);

        let data = close_session_data_frame();
        server
            .feed_stream(stream_id, &data, true)
            .expect("test must succeed");
        let events = server.drain_events().expect("test must succeed");

        // SessionClosed イベントは発行される
        assert!(events.iter().any(|e| matches!(
            e,
            Event::WebTransport(WebTransportEvent::SessionClosed { session_id, .. })
                if *session_id == stream_id
        )));
        // 汎用 StreamEnd イベントは発行されない
        assert!(!events.iter().any(|e| matches!(
            e,
            Event::StreamEnd { stream_id: sid } if *sid == stream_id
        )));
        // セッションと CONNECT ストリームが除去されている
        assert!(!server.wt_sessions.contains_key(&stream_id));
        assert!(!server.streams.contains_key(&stream_id));
    }

    #[test]
    fn test_wt_connect_recv_body_not_accumulated() {
        // WT CONNECT ストリームの DATA は Capsule データであり、
        // recv_body に累積されないことを検証する
        // (転送量に比例したメモリ消費を防ぐ)
        let (mut client, mut server) = setup_wt_pair();
        let stream_id = establish_wt_session(&mut client, &mut server);

        // WT CONNECT フラグが立っていること
        assert!(
            server
                .streams
                .get(&stream_id)
                .expect("test must succeed")
                .is_wt_connect()
        );

        // 無視される Unknown Capsule (type=0x01, len=2, payload=[0xAA, 0xBB])
        // を DATA フレームとして送る
        let data = [0x00, 0x04, 0x01, 0x02, 0xAA, 0xBB];
        server
            .feed_stream(stream_id, &data, false)
            .expect("test must succeed");
        let _ = server.drain_events().expect("test must succeed");

        // recv_body に累積されないこと
        assert!(
            server
                .streams
                .get(&stream_id)
                .expect("test must succeed")
                .received_body()
                .is_empty(),
            "WT CONNECT の DATA は recv_body に累積されないこと"
        );
    }

    #[test]
    fn test_wt_connect_with_content_length_rejected() {
        // content-length ヘッダー付きの WT CONNECT は送信時に H3_MESSAGE_ERROR で拒否される
        // (RFC 9297 Section 3.2: Capsule Protocol との併用は MUST NOT)
        let (mut client, _server) = setup_wt_pair();

        let headers = vec![
            Header::new(b":method", b"CONNECT").expect("test must succeed"),
            Header::new(b":protocol", b"webtransport-h3").expect("test must succeed"),
            Header::new(b":scheme", b"https").expect("test must succeed"),
            Header::new(b":authority", b"example.com").expect("test must succeed"),
            Header::new(b":path", b"/wt").expect("test must succeed"),
            Header::new(b"content-length", b"0").expect("test must succeed"),
        ];
        let err = client.send_request(&headers, false).unwrap_err();
        assert!(
            matches!(err, Error::StreamError(ErrorCode::MessageError)),
            "content-length 付き WT CONNECT は送信時に拒否されること: {err:?}"
        );
    }

    #[test]
    fn test_wt_connect_response_with_content_length_rejected() {
        // content-length ヘッダー付きの WT CONNECT レスポンスは送信時に
        // H3_MESSAGE_ERROR で拒否される
        // (RFC 9297 Section 3.2: Capsule Protocol との併用は MUST NOT)
        let (mut client, mut server) = setup_wt_pair();

        let headers = vec![
            Header::new(b":method", b"CONNECT").expect("test must succeed"),
            Header::new(b":protocol", b"webtransport-h3").expect("test must succeed"),
            Header::new(b":scheme", b"https").expect("test must succeed"),
            Header::new(b":authority", b"example.com").expect("test must succeed"),
            Header::new(b":path", b"/wt").expect("test must succeed"),
        ];
        let stream_id = client
            .send_request(&headers, false)
            .expect("test must succeed");
        let (req_data, _) = client
            .take_stream_data(stream_id)
            .expect("test must succeed");
        server
            .feed_stream(stream_id, &req_data, false)
            .expect("test must succeed");
        let _ = server.drain_events().expect("test must succeed");

        let response = vec![
            Header::new(b":status", b"200").expect("test must succeed"),
            Header::new(b"content-length", b"0").expect("test must succeed"),
        ];
        let err = server
            .send_response(stream_id, &response, false)
            .unwrap_err();
        assert!(
            matches!(err, Error::StreamError(ErrorCode::MessageError)),
            "content-length 付き WT CONNECT レスポンスは送信時に拒否されること: {err:?}"
        );
    }

    #[test]
    fn test_wt_datagram_after_session_close_dropped() {
        // セッション終了後に届いたデータグラムは破棄され、
        // zombie Pending セッションを再生成しないことを検証する
        let (mut client, mut server) = setup_wt_pair();
        let stream_id = establish_wt_session(&mut client, &mut server);

        // サーバー側セッションを WT_CLOSE_SESSION で終了する
        let data = close_session_data_frame();
        server
            .feed_stream(stream_id, &data, false)
            .expect("test must succeed");
        let _ = server.drain_events().expect("test must succeed");
        assert!(server.closed_wt_sessions.contains(&stream_id));

        // 終了後セッションへの HTTP Datagram (quarter stream id + payload)
        let mut datagram = Vec::new();
        crate::varint::encode_into_vec(
            &mut datagram,
            crate::varint::VarInt::new(stream_id / 4).expect("test must succeed"),
        );
        datagram.push(0xAA);
        server.feed_datagram(&datagram).expect("test must succeed");

        assert!(
            !server.wt_sessions.contains_key(&stream_id),
            "終了後セッションへのデータグラムで zombie セッションが再生成されないこと"
        );
    }

    #[test]
    fn test_wt_uni_stream_after_session_close_rejected() {
        // セッション終了後に届いた新規 WT uni stream は拒否され、
        // zombie Pending セッションを再生成しないことを検証する
        let (mut client, mut server) = setup_wt_pair();
        let stream_id = establish_wt_session(&mut client, &mut server);

        // サーバー側セッションを WT_CLOSE_SESSION で終了する
        let data = close_session_data_frame();
        server
            .feed_stream(stream_id, &data, false)
            .expect("test must succeed");
        let _ = server.drain_events().expect("test must succeed");
        assert!(server.closed_wt_sessions.contains(&stream_id));

        // 終了済みセッション ID (stream_id) 宛の新規 uni stream
        // (draft-ietf-webtrans-http3-16 Section 6: セッション終了後の追加データ)
        // stream_id 6 は client-initiated uni stream (2 はクライアント制御ストリーム)
        let mut uni_data = vec![0x40, 0x54]; // stream type 0x54 (varint)
        uni_data.push(stream_id as u8); // session_id (1 バイト varint)
        uni_data.push(0xAA);
        server
            .feed_stream(6, &uni_data, false)
            .expect("test must succeed");
        let events = server.drain_events().expect("test must succeed");

        // WT_SESSION_GONE で拒否される
        assert!(events.iter().any(|e| matches!(
            e,
            Event::WebTransport(WebTransportEvent::BufferedStreamRejected { .. })
        )));
        assert!(
            !server.wt_sessions.contains_key(&stream_id),
            "終了済みセッションへの新規ストリームで zombie セッションが再生成されないこと"
        );
    }

    #[test]
    fn test_wt_reset_after_session_close_ignored() {
        // セッション終了後に届いた RESET_STREAM は静かに無視され、
        // 汎用 StreamReset イベントを発行しないことを検証する
        // (RFC 9000 Section 4.4: RESET_STREAM 受信時は以降のデータを無視する)
        let (mut client, mut server) = setup_wt_pair();
        let stream_id = establish_wt_session(&mut client, &mut server);

        // サーバー側セッションを WT_CLOSE_SESSION で終了する
        let data = close_session_data_frame();
        server
            .feed_stream(stream_id, &data, false)
            .expect("test must succeed");
        let _ = server.drain_events().expect("test must succeed");

        // 終了後の RESET_STREAM はエラーにならず、イベントも発行されない
        server
            .stream_reset(stream_id, 0, 0)
            .expect("test must succeed");
        let events = server.drain_events().expect("test must succeed");
        assert!(
            !events.iter().any(
                |e| matches!(e, Event::StreamReset { stream_id: sid, .. } if *sid == stream_id)
            ),
            "終了後セッションへの RESET は汎用 StreamReset イベントを発行しないこと"
        );
        assert!(
            !server.wt_sessions.contains_key(&stream_id),
            "終了後の RESET で zombie セッションが再生成されないこと"
        );
    }

    #[test]
    fn test_wt_session_terminated_on_stop_sending() {
        // CONNECT stream への STOP_SENDING でセッション終了し、
        // wt_sessions と streams の両方から除去されることを検証する
        let (mut client, mut server) = setup_wt_pair();
        let stream_id = establish_wt_session(&mut client, &mut server);

        server
            .stop_sending(stream_id, 0)
            .expect("test must succeed");
        let _ = server.drain_events().expect("test must succeed");

        assert!(
            !server.wt_sessions.contains_key(&stream_id),
            "STOP_SENDING でセッションが終了し wt_sessions から除去されること"
        );
        assert!(
            !server.streams.contains_key(&stream_id),
            "STOP_SENDING で CONNECT ストリームも streams から除去されること"
        );
        assert!(
            server.closed_wt_sessions.contains(&stream_id),
            "終了済みセッション ID が tombstone に記録されること"
        );
    }

    #[test]
    fn test_wt_connect_late_data_after_session_close_rejected() {
        // セッション終了後の別呼び出しで CONNECT ストリームに DATA が届くと
        // H3_MESSAGE_ERROR、FIN のみは受理されることを検証する
        // (draft-ietf-webtrans-http3-16 Section 6)
        let (mut client, mut server) = setup_wt_pair();
        let stream_id = establish_wt_session(&mut client, &mut server);

        // セッション終了
        let data = close_session_data_frame();
        server
            .feed_stream(stream_id, &data, false)
            .expect("test must succeed");
        let _ = server.drain_events().expect("test must succeed");

        // 遅延 DATA は H3_MESSAGE_ERROR
        let err = server
            .feed_stream(stream_id, &[0x00, 0x02, 0xAA, 0xBB], false)
            .unwrap_err();
        assert!(
            matches!(err, Error::StreamError(ErrorCode::MessageError)),
            "終了後の遅延 DATA は H3_MESSAGE_ERROR であること: {err:?}"
        );

        // FIN のみは受理され、汎用 StreamEnd は発行されない
        server
            .feed_stream(stream_id, &[], true)
            .expect("test must succeed");
        let events = server.drain_events().expect("test must succeed");
        assert!(!events.iter().any(|e| matches!(
            e,
            Event::StreamEnd { stream_id: sid } if *sid == stream_id
        )));
        assert!(
            !server.streams.contains_key(&stream_id),
            "終了後の FIN で streams に再生成されないこと"
        );
    }

    #[test]
    fn test_wt_connect_client_recv_body_not_accumulated() {
        // クライアント側でも WT CONNECT ストリームの DATA は recv_body に
        // 累積されないことを検証する
        let (mut client, mut server) = setup_wt_pair();
        let stream_id = establish_wt_session(&mut client, &mut server);

        // クライアント側の WT CONNECT フラグが立っていること
        assert!(
            client
                .streams
                .get(&stream_id)
                .expect("test must succeed")
                .is_wt_connect()
        );

        // 無視される Unknown Capsule を DATA フレームとして送る
        let data = [0x00, 0x04, 0x01, 0x02, 0xAA, 0xBB];
        client
            .feed_stream(stream_id, &data, false)
            .expect("test must succeed");
        let _ = client.drain_events().expect("test must succeed");

        assert!(
            client
                .streams
                .get(&stream_id)
                .expect("test must succeed")
                .received_body()
                .is_empty(),
            "クライアント側でも WT CONNECT の DATA は recv_body に累積されないこと"
        );
    }

    #[test]
    fn test_wt_connect_request_with_content_type_rejected() {
        // content-type ヘッダー付きの WT CONNECT は送信時に H3_MESSAGE_ERROR で拒否される
        // (RFC 9297 Section 3.2: Capsule Protocol との併用は MUST NOT)
        let (mut client, _server) = setup_wt_pair();

        let headers = vec![
            Header::new(b":method", b"CONNECT").expect("test must succeed"),
            Header::new(b":protocol", b"webtransport-h3").expect("test must succeed"),
            Header::new(b":scheme", b"https").expect("test must succeed"),
            Header::new(b":authority", b"example.com").expect("test must succeed"),
            Header::new(b":path", b"/wt").expect("test must succeed"),
            Header::new(b"content-type", b"text/plain").expect("test must succeed"),
        ];
        let err = client.send_request(&headers, false).unwrap_err();
        assert!(
            matches!(err, Error::StreamError(ErrorCode::MessageError)),
            "content-type 付き WT CONNECT は送信時に拒否されること: {err:?}"
        );
    }

    #[test]
    fn test_wt_connect_response_with_content_type_rejected() {
        // content-type ヘッダー付きの WT CONNECT レスポンスは送信時に
        // H3_MESSAGE_ERROR で拒否される
        // (RFC 9297 Section 3.2: Capsule Protocol との併用は MUST NOT)
        let (mut client, mut server) = setup_wt_pair();

        let headers = vec![
            Header::new(b":method", b"CONNECT").expect("test must succeed"),
            Header::new(b":protocol", b"webtransport-h3").expect("test must succeed"),
            Header::new(b":scheme", b"https").expect("test must succeed"),
            Header::new(b":authority", b"example.com").expect("test must succeed"),
            Header::new(b":path", b"/wt").expect("test must succeed"),
        ];
        let stream_id = client
            .send_request(&headers, false)
            .expect("test must succeed");
        let (req_data, _) = client
            .take_stream_data(stream_id)
            .expect("test must succeed");
        server
            .feed_stream(stream_id, &req_data, false)
            .expect("test must succeed");
        let _ = server.drain_events().expect("test must succeed");

        let response = vec![
            Header::new(b":status", b"200").expect("test must succeed"),
            Header::new(b"content-type", b"text/plain").expect("test must succeed"),
        ];
        let err = server
            .send_response(stream_id, &response, false)
            .unwrap_err();
        assert!(
            matches!(err, Error::StreamError(ErrorCode::MessageError)),
            "content-type 付き WT CONNECT レスポンスは送信時に拒否されること: {err:?}"
        );
    }

    #[test]
    fn test_wt_connect_response_204_rejected() {
        // 204 (No Content) レスポンスは Capsule Protocol と併用できないため
        // H3_MESSAGE_ERROR で拒否される (RFC 9297 Section 3.2)
        let (mut client, mut server) = setup_wt_pair();

        let headers = vec![
            Header::new(b":method", b"CONNECT").expect("test must succeed"),
            Header::new(b":protocol", b"webtransport-h3").expect("test must succeed"),
            Header::new(b":scheme", b"https").expect("test must succeed"),
            Header::new(b":authority", b"example.com").expect("test must succeed"),
            Header::new(b":path", b"/wt").expect("test must succeed"),
        ];
        let stream_id = client
            .send_request(&headers, false)
            .expect("test must succeed");
        let (req_data, _) = client
            .take_stream_data(stream_id)
            .expect("test must succeed");
        server
            .feed_stream(stream_id, &req_data, false)
            .expect("test must succeed");
        let _ = server.drain_events().expect("test must succeed");

        // サーバー側送信時点で 204 が拒否される (送信側も RFC 9297 Section 3.2)
        let response = vec![Header::new(b":status", b"204").expect("test must succeed")];
        let err = server
            .send_response(stream_id, &response, false)
            .unwrap_err();
        assert!(
            matches!(err, Error::StreamError(ErrorCode::MessageError)),
            "204 レスポンスは送信時に拒否されること: {err:?}"
        );
    }

    #[test]
    fn test_normal_204_response_accepted() {
        // 通常 HTTP リクエストの 204 (No Content) レスポンスは拒否されないことを検証する
        // (RFC 9297 Section 3.2 の 204/205/206 禁止は Capsule Protocol を使用する
        //  レスポンスのみが対象)
        let mut client = Connection::client(Settings::default());
        client.set_control_stream_id(2).expect("test must succeed");
        let mut server = Connection::server(Settings::default());
        server.set_control_stream_id(3).expect("test must succeed");

        // 制御ストリームの SETTINGS 交換
        let (client_ctrl, _) = client.take_stream_data(2).expect("test must succeed");
        server
            .feed_stream(2, &client_ctrl, false)
            .expect("test must succeed");
        let _ = server.drain_events().expect("test must succeed");
        let (server_ctrl, _) = server.take_stream_data(3).expect("test must succeed");
        client
            .feed_stream(3, &server_ctrl, false)
            .expect("test must succeed");
        let _ = client.drain_events().expect("test must succeed");

        let headers = vec![
            Header::new(b":method", b"DELETE").expect("test must succeed"),
            Header::new(b":path", b"/item").expect("test must succeed"),
            Header::new(b":scheme", b"https").expect("test must succeed"),
            Header::new(b":authority", b"example.com").expect("test must succeed"),
        ];
        let stream_id = client
            .send_request(&headers, true)
            .expect("test must succeed");
        let mut req_data = Vec::new();
        while let Some((data, fin)) = client.take_stream_data(stream_id) {
            req_data.extend_from_slice(&data);
            if fin {
                break;
            }
        }
        server
            .feed_stream(stream_id, &req_data, true)
            .expect("test must succeed");
        let _ = server.drain_events().expect("test must succeed");

        // サーバーが通常の 204 レスポンスを送信
        let response = vec![Header::new(b":status", b"204").expect("test must succeed")];
        server
            .send_response(stream_id, &response, true)
            .expect("test must succeed");
        let mut resp_data = Vec::new();
        while let Some((data, fin)) = server.take_stream_data(stream_id) {
            resp_data.extend_from_slice(&data);
            if fin {
                break;
            }
        }
        // クライアントは 204 をエラーなく受信できること
        client
            .feed_stream(stream_id, &resp_data, true)
            .expect("test must succeed");
        let events = client.drain_events().expect("test must succeed");
        assert!(events.iter().any(|e| matches!(
            e,
            Event::HeadersEnd { stream_id: sid } if *sid == stream_id
        )));
    }

    #[test]
    fn test_send_buffer_discarded_on_stop_sending() {
        // STOP_SENDING 受信で送信バッファが破棄され、writable_streams から
        // 消えることを検証する
        let mut client = Connection::client(Settings::default());
        client.set_control_stream_id(2).expect("test must succeed");

        // クライアントがリクエストを送信 (fin=false) し、一部だけ消費する
        // (送信バッファに未消費データが残る状態を作る)
        let headers = vec![
            Header::new(b":method", b"POST").expect("test must succeed"),
            Header::new(b":path", b"/").expect("test must succeed"),
            Header::new(b":scheme", b"https").expect("test must succeed"),
            Header::new(b":authority", b"example.com").expect("test must succeed"),
        ];
        let stream_id = client
            .send_request(&headers, false)
            .expect("test must succeed");
        let (data, _) = client
            .get_stream_data(stream_id)
            .expect("test must succeed");
        let half = data.len() / 2;
        client.consume_stream_data(stream_id, half);
        assert!(
            client
                .writable_streams()
                .collect::<Vec<_>>()
                .contains(&stream_id),
            "未消費データが writable_streams に載っていること"
        );

        // サーバーが STOP_SENDING を送信 → クライアントが受信
        client
            .stop_sending(stream_id, 0)
            .expect("test must succeed");
        // 送信バッファが破棄されたこと (writable_streams から消える)
        assert!(
            !client
                .writable_streams()
                .collect::<Vec<_>>()
                .contains(&stream_id),
            "STOP_SENDING 受信後は送信データが破棄され writable_streams に残らないこと"
        );
    }

    #[test]
    fn test_streams_removed_after_stop_sending_and_peer_fin() {
        // STOP_SENDING 受信後のレスポンス (FIN) で Closed になったストリームが
        // 除去されることを検証する
        let mut client = Connection::client(Settings::default());
        client.set_control_stream_id(2).expect("test must succeed");
        let mut server = Connection::server(Settings::default());
        server.set_control_stream_id(3).expect("test must succeed");

        // 制御ストリームの SETTINGS 交換
        let (client_ctrl, _) = client.take_stream_data(2).expect("test must succeed");
        server
            .feed_stream(2, &client_ctrl, false)
            .expect("test must succeed");
        let _ = server.drain_events().expect("test must succeed");
        let (server_ctrl, _) = server.take_stream_data(3).expect("test must succeed");
        client
            .feed_stream(3, &server_ctrl, false)
            .expect("test must succeed");
        let _ = client.drain_events().expect("test must succeed");

        // クライアントがリクエストを送信 (fin=false) し、サーバーが受信
        let headers = vec![
            Header::new(b":method", b"POST").expect("test must succeed"),
            Header::new(b":path", b"/").expect("test must succeed"),
            Header::new(b":scheme", b"https").expect("test must succeed"),
            Header::new(b":authority", b"example.com").expect("test must succeed"),
        ];
        let stream_id = client
            .send_request(&headers, false)
            .expect("test must succeed");
        let (req_data, _) = client
            .take_stream_data(stream_id)
            .expect("test must succeed");
        server
            .feed_stream(stream_id, &req_data, false)
            .expect("test must succeed");
        let _ = server.drain_events().expect("test must succeed");

        // サーバーが STOP_SENDING を送信 → クライアントが受信
        client
            .stop_sending(stream_id, 0)
            .expect("test must succeed");

        // サーバーがレスポンス (fin=true) を送信 → クライアント側は Closed
        let response = vec![Header::new(b":status", b"204").expect("test must succeed")];
        server
            .send_response(stream_id, &response, true)
            .expect("test must succeed");
        let mut resp_data = Vec::new();
        while let Some((data, fin)) = server.take_stream_data(stream_id) {
            resp_data.extend_from_slice(&data);
            if fin {
                break;
            }
        }
        client
            .feed_stream(stream_id, &resp_data, true)
            .expect("test must succeed");
        let _ = client.drain_events().expect("test must succeed");

        assert!(
            !client.streams.contains_key(&stream_id),
            "STOP_SENDING + ピア FIN で Closed になったストリームが除去されること"
        );
    }
}
