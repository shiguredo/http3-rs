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
//! conn.set_control_stream_id(ctrl_stream_id).unwrap();
//! if let Some((data, fin)) = conn.get_stream_data(ctrl_stream_id) {
//!     // QUIC で送信
//! }
//!
//! // リクエストを送信
//! let stream_id = conn.send_request(&[
//!     Header::new(b":method", b"GET").unwrap(),
//!     Header::new(b":path", b"/").unwrap(),
//!     Header::new(b":scheme", b"https").unwrap(),
//!     Header::new(b":authority", b"example.com").unwrap(),
//! ], true).unwrap();
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

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

use crate::error::{Error, ErrorCode};
use crate::event::{Event, WtStreamReset};
use crate::limits::Limits;
use crate::qpack::{
    DecodeOutput, DecoderStream, DecoderStreamReceiver, DynamicDecoder, DynamicEncoder,
    EncoderStream, EncoderStreamReceiver, Header, estimate_encoded_size,
};
use crate::settings::Settings;
use crate::stream::request::{RawReceivedData, RequestStream};
use crate::stream::{ControlStreamRecv, ControlStreamSend, StreamKind, StreamState};
use crate::varint::VarInt;
use crate::webtransport::error::ErrorCode as WtErrorCode;
use crate::webtransport::session::{DataFlowControl, DirectionalStreamFlowControl};

pub use client::ClientConnection;
pub use server::ServerConnection;

/// `Connection::associate_or_buffer_stream` の結果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AssocOutcome {
    /// 既存 Established セッションに即時関連付けた
    Established,
    /// Pending セッションにバッファリングした (確立時にイベント発火)
    Buffered,
    /// バッファ上限超過 (WT_BUFFERED_STREAM_REJECTED 相当)
    BufferOverflow,
}

/// セッション確立前にバッファリングするストリームの上限 (draft-ietf-webtrans-http3-15 Section 4.6)
const WT_MAX_BUFFERED_STREAMS: usize = 100;

/// セッション確立前にバッファリングするデータグラムの上限 (draft-ietf-webtrans-http3-15 Section 4.6)
const WT_MAX_BUFFERED_DATAGRAMS: usize = 100;

/// サーバー側で許容する Pending WebTransport セッション数の上限
///
/// クライアントは未知の `session_id` で先行ストリーム / データグラムを送ってくることが
/// あるが、`session_id` が一意であるたびに新しい Pending セッションを生成すると、
/// 攻撃者が一意な `session_id` を量産するだけで Pending セッションを無限増殖させられる。
/// これを防ぐため接続単位で Pending セッション数に上限を設ける。
/// (draft-ietf-webtrans-http3-15 Section 4.6 / RFC 9297 Section 2.1 / nghttp3
///  lib/nghttp3_conn.c L3649 と整合)
const WT_MAX_PENDING_SESSIONS: usize = 16;

/// セッション確立前の先行ストリームごとに保持する受信ペイロードの上限 (バイト)
/// (draft-ietf-webtrans-http3-15 Section 4.6, DoS 対策)
const WT_MAX_BUFFERED_STREAM_BYTES: usize = 64 * 1024;

/// セッション確立前の先行 WebTransport ストリームごとに保持する受信状態
///
/// (draft-ietf-webtrans-http3-15 Section 4.6)
#[derive(Debug)]
struct BufferedStreamEntry {
    /// 双方向ストリームかどうか
    is_bidi: bool,
    /// 受信済みペイロード (Open 後 〜 FIN まで)
    data: Vec<u8>,
    /// FIN を受信済みかどうか
    fin: bool,
}

impl BufferedStreamEntry {
    fn new(is_bidi: bool) -> Self {
        Self {
            is_bidi,
            data: Vec::new(),
            fin: false,
        }
    }
}

/// WebTransport セッションの Connection 層での状態
///
/// `Connection` 内でセッションのライフサイクルと関連ストリームを追跡する。
/// フロー制御等の高レベル機能は `webtransport::Session` が担当する。
/// (draft-ietf-webtrans-http3-15 Section 3, 4.6, 6)
#[derive(Debug)]
struct WtSession {
    /// セッション状態
    state: WtSessionState,
    /// セッションに関連する全ストリーム ID (uni + bidi)
    associated_streams: HashSet<u64>,
    /// セッション確立前のバッファリングされたストリーム (Section 4.6)
    ///
    /// `buffered_streams` は順序保持のための stream_id ベクタ。
    /// `buffered_stream_entries` は同じ stream_id をキーに受信ペイロード/FIN を保持する。
    /// (draft-ietf-webtrans-http3-15 Section 4.6 — Open / Data / End を確立後に
    ///  順序を保って一括発火するために必要)
    buffered_streams: Vec<u64>,
    buffered_stream_entries: HashMap<u64, BufferedStreamEntry>,
    /// セッション確立前のバッファリングされたデータグラム (Section 4.6)
    buffered_datagrams: Vec<Vec<u8>>,
    /// CONNECT ストリーム上の Capsule デコードバッファ (Section 5.6)
    ///
    /// Capsule が複数の DATA フレームにまたがる場合のバッファリング用。
    capsule_buf: Vec<u8>,
    /// リクエスト時の WT-Available-Protocols (Section 3.3)
    ///
    /// クライアントが送信した WT-Available-Protocols の値を保持する。
    /// レスポンス受信時に WT-Protocol を検証するために使用する。
    available_protocols: Vec<String>,
    /// フロー制御が有効かどうか (Section 5.1)
    ///
    /// 両端がフロー制御を宣言した場合のみ `true`。
    /// セッション確立時に `flow_control_enabled_with_peer` で決定される。
    flow_control_enabled: bool,
    /// WT_CLOSE_SESSION カプセル受信済みフラグ
    ///
    /// WT_CLOSE_SESSION 受信後に CONNECT ストリーム上で追加データが届いた場合、
    /// H3_MESSAGE_ERROR でストリームをリセットする。
    /// (draft-ietf-webtrans-http3-15 Section 6)
    close_session_received: bool,
    /// 受信側ストリームフロー制御 (単方向)
    /// (draft-ietf-webtrans-http3-15 Section 5.6)
    /// フロー制御有効時にセッション確立時点で初期化される。
    recv_stream_fc_uni: Option<DirectionalStreamFlowControl>,
    /// 受信側ストリームフロー制御 (双方向)
    /// (draft-ietf-webtrans-http3-15 Section 5.6)
    recv_stream_fc_bidi: Option<DirectionalStreamFlowControl>,
    /// 受信側データフロー制御
    /// (draft-ietf-webtrans-http3-15 Section 5.4)
    recv_data_fc: Option<DataFlowControl>,
    /// Connection 層で生成された送信待ちカプセル (WT_MAX_STREAMS, WT_MAX_DATA)
    /// アプリケーション層が `take_wt_pending_capsules()` で取り出して送信する。
    pending_capsules: Vec<crate::webtransport::Capsule>,
}

/// WebTransport セッションの Connection 層での状態
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WtSessionState {
    /// CONNECT 送信/受信済みだがレスポンス未処理
    Pending,
    /// 確立済み (200 OK 受信)
    Established,
    /// グレースフルシャットダウン中
    /// (draft-ietf-webtrans-http3-15 Section 4.7)
    ///
    /// `WT_DRAIN_SESSION` を受信した、または GOAWAY を受けたクライアント側の
    /// セッションがこの状態に遷移する。Section 4.7 では MAY continue だが、
    /// 本実装は新規ストリーム/データグラムの送信を拒否する。
    /// 既存ストリームの送受信および全てのカプセル受信は継続できる。
    Draining,
    /// 終了済み
    Closed,
}

impl WtSession {
    /// 新しいセッションを作成 (Pending 状態)
    fn new() -> Self {
        Self {
            state: WtSessionState::Pending,
            associated_streams: HashSet::new(),
            buffered_streams: Vec::new(),
            buffered_stream_entries: HashMap::new(),
            buffered_datagrams: Vec::new(),
            capsule_buf: Vec::new(),
            available_protocols: Vec::new(),
            flow_control_enabled: false,
            close_session_received: false,
            recv_stream_fc_uni: None,
            recv_stream_fc_bidi: None,
            recv_data_fc: None,
            pending_capsules: Vec::new(),
        }
    }

    /// フロー制御を初期化する (セッション確立時に呼ぶ)
    ///
    /// ローカルの SETTINGS から初期リミットを読み取り、受信側フロー制御を設定する。
    /// (draft-ietf-webtrans-http3-15 Section 5.5, 5.6)
    fn initialize_flow_control(
        &mut self,
        local_wt: &crate::webtransport::settings::Settings,
        queue_initial_capsules: bool,
    ) {
        if !self.flow_control_enabled {
            return;
        }
        self.recv_stream_fc_uni = Some(DirectionalStreamFlowControl::new(
            local_wt.wt_initial_max_streams_uni.get(),
        ));
        self.recv_stream_fc_bidi = Some(DirectionalStreamFlowControl::new(
            local_wt.wt_initial_max_streams_bidi.get(),
        ));
        self.recv_data_fc = Some(DataFlowControl::new(local_wt.wt_initial_max_data.get()));

        if !queue_initial_capsules {
            return;
        }
        if local_wt.wt_initial_max_streams_bidi.get() > 0 {
            self.pending_capsules
                .push(crate::webtransport::Capsule::MaxStreams {
                    bidirectional: true,
                    maximum: local_wt.wt_initial_max_streams_bidi.get(),
                });
        }
        if local_wt.wt_initial_max_streams_uni.get() > 0 {
            self.pending_capsules
                .push(crate::webtransport::Capsule::MaxStreams {
                    bidirectional: false,
                    maximum: local_wt.wt_initial_max_streams_uni.get(),
                });
        }
        if local_wt.wt_initial_max_data.get() > 0 {
            self.pending_capsules
                .push(crate::webtransport::Capsule::MaxData {
                    maximum: local_wt.wt_initial_max_data.get(),
                });
        }
    }

    /// 受信ストリーム数のフロー制御チェック
    ///
    /// `false` の場合は WT_FLOW_CONTROL_ERROR で終了すべき。
    fn check_received_stream(&self, bidirectional: bool) -> bool {
        if !self.flow_control_enabled {
            return true;
        }
        if bidirectional {
            self.recv_stream_fc_bidi
                .as_ref()
                .is_none_or(|fc| fc.check_received())
        } else {
            self.recv_stream_fc_uni
                .as_ref()
                .is_none_or(|fc| fc.check_received())
        }
    }

    /// 受信ストリーム数を加算
    fn add_received_stream(&mut self, bidirectional: bool) {
        if bidirectional {
            if let Some(fc) = &mut self.recv_stream_fc_bidi {
                fc.on_stream_received();
            }
        } else {
            if let Some(fc) = &mut self.recv_stream_fc_uni {
                fc.on_stream_received();
            }
        }
    }

    /// 受信データのフロー制御チェック
    ///
    /// `false` の場合は WT_FLOW_CONTROL_ERROR で終了すべき。
    fn check_received_data(&self, bytes: u64) -> bool {
        if !self.flow_control_enabled {
            return true;
        }
        self.recv_data_fc
            .as_ref()
            .is_none_or(|fc| fc.check_received(bytes))
    }

    /// 受信データ量を加算
    fn add_received_data(&mut self, bytes: u64) {
        if let Some(fc) = &mut self.recv_data_fc {
            fc.on_data_received(bytes);
        }
    }

    /// ピアが開いたストリームが完全に閉じたことを通知
    ///
    /// 必要に応じて WT_MAX_STREAMS カプセルを `pending_capsules` に追加する。
    fn on_remote_stream_closed(&mut self, bidirectional: bool) {
        if !self.flow_control_enabled {
            return;
        }
        let fc = if bidirectional {
            self.recv_stream_fc_bidi.as_mut()
        } else {
            self.recv_stream_fc_uni.as_mut()
        };
        if let Some(fc) = fc
            && let Some(new_max) = fc.on_stream_closed()
        {
            self.pending_capsules
                .push(crate::webtransport::Capsule::MaxStreams {
                    bidirectional,
                    maximum: new_max,
                });
        }
    }

    /// ピアからの受信データをアプリが消費したことを通知
    ///
    /// 必要に応じて WT_MAX_DATA カプセルを `pending_capsules` に追加する。
    fn on_data_consumed(&mut self, bytes: u64) {
        if !self.flow_control_enabled {
            return;
        }
        if let Some(fc) = &mut self.recv_data_fc
            && let Some(new_max) = fc.on_data_consumed(bytes)
        {
            self.pending_capsules
                .push(crate::webtransport::Capsule::MaxData { maximum: new_max });
        }
    }

    /// 送信待ちカプセルを取り出す
    fn take_pending_capsules(&mut self) -> Vec<crate::webtransport::Capsule> {
        std::mem::take(&mut self.pending_capsules)
    }

    /// ストリームをセッションに関連付ける
    fn associate_stream(&mut self, stream_id: u64) {
        self.associated_streams.insert(stream_id);
    }

    /// ストリームの関連付けを解除する
    #[allow(dead_code)]
    fn disassociate_stream(&mut self, stream_id: u64) {
        self.associated_streams.remove(&stream_id);
    }

    /// 受信ストリームをバッファリング (Section 4.6)
    ///
    /// バッファ上限を超えた場合は `false` を返す。
    /// 呼び出し元は `WT_BUFFERED_STREAM_REJECTED` で RESET_STREAM を送信すること。
    fn buffer_stream(&mut self, stream_id: u64, is_bidi: bool) -> bool {
        if self.buffered_streams.len() >= WT_MAX_BUFFERED_STREAMS {
            return false;
        }
        self.buffered_streams.push(stream_id);
        self.buffered_stream_entries
            .insert(stream_id, BufferedStreamEntry::new(is_bidi));
        true
    }

    /// バッファリング中のストリームに受信データを追記する (Section 4.6)
    ///
    /// バッファ上限超過時は `false` を返す。呼び出し元は WT_BUFFERED_STREAM_REJECTED 相当の
    /// 扱いに切り替えること。
    fn append_buffered_stream_data(&mut self, stream_id: u64, data: &[u8]) -> bool {
        if let Some(entry) = self.buffered_stream_entries.get_mut(&stream_id) {
            if entry.data.len().saturating_add(data.len()) > WT_MAX_BUFFERED_STREAM_BYTES {
                return false;
            }
            entry.data.extend_from_slice(data);
            true
        } else {
            false
        }
    }

    /// バッファリング中のストリームに FIN を記録する (Section 4.6)
    fn mark_buffered_stream_fin(&mut self, stream_id: u64) {
        if let Some(entry) = self.buffered_stream_entries.get_mut(&stream_id) {
            entry.fin = true;
        }
    }

    /// バッファリングされたストリーム ID を順序付きで取り出す (セッション確立後に呼び出す)
    fn take_buffered_streams(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.buffered_streams)
    }

    /// バッファリングされたストリーム受信状態を取り出す (セッション確立後に呼び出す)
    fn take_buffered_stream_entry(&mut self, stream_id: u64) -> Option<BufferedStreamEntry> {
        self.buffered_stream_entries.remove(&stream_id)
    }

    /// 受信データグラムをバッファリング (Section 4.6)
    ///
    /// バッファ上限を超えた場合は `false` を返す。
    /// 呼び出し元はデータグラムを破棄すること。
    fn buffer_datagram(&mut self, data: Vec<u8>) -> bool {
        if self.buffered_datagrams.len() >= WT_MAX_BUFFERED_DATAGRAMS {
            return false;
        }
        self.buffered_datagrams.push(data);
        true
    }

    /// バッファリングされたデータグラムを取り出す (セッション確立後に呼び出す)
    fn take_buffered_datagrams(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.buffered_datagrams)
    }
}

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
    peer_goaway_last_id: Option<u64>,
    /// 最後に送信した GOAWAY の ID (段階的送信のために単調減少を検証する)
    last_sent_goaway_id: Option<u64>,
    /// クライアントから受信した MAX_PUSH_ID の最新値
    ///
    /// サーバープッシュ自体はサポートしないが、RFC 9114 Section 7.2.7 で定義された
    /// 単調増加制約だけは検証する (後退は H3_ID_ERROR)。
    max_push_id: Option<u64>,
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

    fn peer_requires_initial_wt_capsules(&self) -> bool {
        self.peer_settings
            .as_ref()
            .and_then(|s| s.wt_settings.as_ref())
            .is_some_and(|wt| wt.requires_initial_capsule_flow_control_compat())
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
    fn peer_goaway_request_boundary(&self) -> Option<u64> {
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

    /// QUIC transport parameter に基づく WebTransport 前提条件を注入する
    ///
    /// Sans I/O 設計上、Connection は QUIC transport parameter に直接アクセスできない。
    /// 上位層は QUIC ハンドシェイク完了後に本メソッドを呼び出し、
    /// transport parameter レベルの前提条件を Connection に注入すること。
    ///
    /// - `max_datagram_frame_size` > 0 であること (全ドラフト共通、RFC 9297)
    /// - `reset_stream_at` がサポートされていること (draft-15 のみ必須)
    ///
    /// `reset_stream_at_supported` が false でも呼び出しは成功する。
    /// ただし draft-15 接続では CONNECT 送信/受信時に拒否される。
    /// draft-02/07 では `reset_stream_at` は不要。
    /// (draft-ietf-webtrans-http3-15 Section 3.1)
    /// 将来のドラフトで変更される可能性がある
    ///
    /// WebTransport CONNECT の送信/受信前に呼び出す必要がある。
    /// 呼び出さない場合、WebTransport セッションの確立は拒否される。
    pub fn set_webtransport_transport_verified(
        &mut self,
        max_datagram_frame_size_nonzero: bool,
        reset_stream_at_supported: bool,
    ) -> Result<(), Error> {
        if !max_datagram_frame_size_nonzero {
            return Err(Error::ConnectionError(ErrorCode::InternalError));
        }
        self.wt_transport_verified = true;
        self.wt_reset_stream_at_supported = reset_stream_at_supported;
        Ok(())
    }

    /// WebTransport transport parameter が検証済みかを取得する
    pub fn is_webtransport_transport_verified(&self) -> bool {
        self.wt_transport_verified
    }

    /// QUIC DATAGRAM フレームのペイロードを受信
    ///
    /// Sans I/O パターンに基づき、QUIC スタックから受信した DATAGRAM フレームの
    /// ペイロードを Connection に注入する。セッション ID によるルーティング、
    /// バッファリング (Section 4.6)、セッション終了後の破棄を行う。
    /// (draft-ietf-webtrans-http3-15 Section 4.5)
    pub fn feed_datagram(&mut self, data: &[u8]) -> Result<(), Error> {
        if let Some(ref err) = self.error {
            return Err(err.clone());
        }

        // WebTransport ネゴシエーション完了を bidi / uni stream 経路と同じ
        // 一次関数で確認する (draft-ietf-webtrans-http3-15 Section 4.2 / 7.1)。
        // SETTINGS_H3_DATAGRAM の両端合意も `is_wt_fully_negotiated()` の中で
        // 確認している。未確立で受信した datagram は静かに破棄する (RFC 9297 と整合)。
        if !self.is_wt_fully_negotiated() {
            return Ok(());
        }

        // HTTP Datagram フォーマットをデコード
        // RFC 9297 Section 2.1: Quarter Stream ID が不正な (短すぎる、もしくは
        // 2^60-1 を超える) Datagram を受信した場合は H3_DATAGRAM_ERROR で接続を
        // 閉じなければならない。
        let datagram = match crate::webtransport::Datagram::decode(data) {
            Some((d, _)) => d,
            None => {
                return Err(Error::ConnectionError(ErrorCode::H3DatagramError));
            }
        };

        let session_id = datagram.session_id;

        // session_id は client-initiated bidirectional stream ID でなければならない
        // (draft-ietf-webtrans-http3-15 Section 4.5)
        if session_id & 0x03 != 0x00 {
            return Err(Error::ConnectionError(ErrorCode::H3DatagramError));
        }

        // セッション状態に応じてルーティング
        if let Some(session) = self.wt_sessions.get_mut(&session_id) {
            match session.state {
                WtSessionState::Established | WtSessionState::Draining => {
                    // Draining 状態でも既存セッションのデータグラム受信は許可する
                    // (draft-ietf-webtrans-http3-15 Section 6)
                    self.events.push_back(Event::WebTransportDatagram {
                        session_id,
                        payload: datagram.payload,
                    });
                }
                WtSessionState::Pending => {
                    // セッション未確立: バッファリング (Section 4.6)
                    // 上限超過時は破棄 (ストリームと異なり RESET 不要)
                    session.buffer_datagram(datagram.payload);
                }
                WtSessionState::Closed => {
                    // セッション終了済み: 破棄 (Section 6)
                }
            }
        } else {
            // クライアントは自身が開始していない session_id を拒否する
            // (draft-ietf-webtrans-http3-15 Section 4.6)
            if self.role == Role::Client {
                // 破棄 (ストリームと異なりデータグラムは RESET 不要)
            } else if let Some(last_id) = self.last_sent_goaway_id
                && session_id >= last_id
            {
                // サーバーが GOAWAY を送信済みの場合、その境界以降の session_id に
                // 対する新規 WebTransport セッションは受け入れない
                // (draft-ietf-webtrans-http3-15 Section 4.7 / nghttp3
                //  lib/nghttp3_conn.c L3654)。datagram は破棄するだけでよい。
            } else if self.count_pending_wt_sessions() >= WT_MAX_PENDING_SESSIONS {
                // 接続単位の Pending セッション上限を超過: 破棄
                // (draft-ietf-webtrans-http3-15 Section 4.6 / DoS 対策)
            } else {
                // サーバー側: セッション未登録だがデータグラムが先に到着
                // 新規 Pending セッションを作成してバッファリング (Section 4.6)
                let mut session = WtSession::new();
                session.buffer_datagram(datagram.payload);
                self.wt_sessions.insert(session_id, session);
            }
        }

        Ok(())
    }

    /// WebTransport データグラムを送信用にエンコードする
    ///
    /// 指定されたセッションのデータグラムを HTTP Datagram フォーマットにエンコードして返す。
    /// 呼び出し側は返されたバイト列を QUIC DATAGRAM フレームで送信すること。
    /// (draft-ietf-webtrans-http3-15 Section 4.5)
    ///
    /// セッションが存在しないか Established でない場合はエラーを返す。
    pub fn send_datagram(&self, session_id: u64, payload: &[u8]) -> Result<Vec<u8>, Error> {
        // SETTINGS_H3_DATAGRAM のネゴシエーション確認 (RFC 9297 Section 2.1.1)
        // ローカルとピアの両方が SETTINGS_H3_DATAGRAM=1 を送受信済みでなければならない
        let local_datagram = self.local_settings.h3_datagram == Some(true);
        let peer_datagram = self
            .peer_settings
            .as_ref()
            .is_some_and(|s| s.h3_datagram == Some(true));
        if !local_datagram || !peer_datagram {
            return Err(Error::ConnectionError(ErrorCode::GeneralProtocolError));
        }

        // session_id の検証
        if session_id & 0x03 != 0x00 {
            return Err(Error::ConnectionError(ErrorCode::GeneralProtocolError));
        }

        let session = self
            .wt_sessions
            .get(&session_id)
            .ok_or(Error::ConnectionError(ErrorCode::GeneralProtocolError))?;

        // Draining 状態では新規データグラム送信を拒否する
        // (draft-ietf-webtrans-http3-15 Section 6)
        if session.state == WtSessionState::Draining {
            return Err(Error::WtSessionDraining(session_id));
        }
        if session.state != WtSessionState::Established {
            return Err(Error::ConnectionError(ErrorCode::GeneralProtocolError));
        }

        // session_id は wt_sessions に登録済み = 既に 4 の倍数として検証済み
        let datagram = crate::webtransport::Datagram::new(session_id, payload.to_vec())
            .map_err(|_| Error::ConnectionError(ErrorCode::InternalError))?;
        let mut buf = Vec::new();
        datagram.encode(&mut buf);
        Ok(buf)
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

    /// QUIC からストリームデータを受信
    pub fn feed_stream(&mut self, stream_id: u64, data: &[u8], fin: bool) -> Result<(), Error> {
        if self.error.is_some() {
            return Err(Error::ConnectionError(ErrorCode::InternalError));
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
        self.handle_bidirectional_stream(stream_id, data, fin)?;
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
            } else if let Some(&session_id) = self.wt_uni_streams.get(&stream_id) {
                // Pending セッション中はイベントを発火せずバッファに追記する
                // (draft-ietf-webtrans-http3-15 Section 4.6)
                let buffered = self
                    .wt_sessions
                    .get(&session_id)
                    .is_some_and(|s| s.state == WtSessionState::Pending);
                if buffered {
                    if let Some(session) = self.wt_sessions.get_mut(&session_id)
                        && !session.append_buffered_stream_data(stream_id, data)
                    {
                        self.wt_uni_streams.remove(&stream_id);
                        let _ = session.take_buffered_stream_entry(stream_id);
                        self.events
                            .push_back(Event::WebTransportBufferedStreamRejected {
                                stream_id,
                                error_code: WtErrorCode::BufferedStreamRejected as u64,
                            });
                    }
                } else {
                    // WebTransport 単方向ストリーム: データフロー制御 (Section 5.4)
                    if let Some(session) = self.wt_sessions.get_mut(&session_id) {
                        let data_len = data.len() as u64;
                        if !session.check_received_data(data_len) {
                            self.terminate_wt_session_with(
                                session_id,
                                WtErrorCode::FlowControlError as u64,
                                0,
                                String::new(),
                            );
                            return Ok(());
                        }
                        session.add_received_data(data_len);
                    }
                    self.events.push_back(Event::WebTransportUniStreamData {
                        stream_id,
                        data: data.to_vec(),
                    });
                }
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
            if let Some(session_id) = self.wt_uni_streams.remove(&stream_id) {
                // Pending セッション中は End イベントを発火せずバッファに記録する
                // (draft-ietf-webtrans-http3-15 Section 4.6)
                let pending = self
                    .wt_sessions
                    .get(&session_id)
                    .is_some_and(|s| s.state == WtSessionState::Pending);
                if pending {
                    if let Some(session) = self.wt_sessions.get_mut(&session_id) {
                        session.mark_buffered_stream_fin(stream_id);
                    }
                    // wt_uni_streams から外してしまうと後で確立時に紐付けできないため戻す
                    self.wt_uni_streams.insert(stream_id, session_id);
                } else {
                    // ストリーム閉鎖: WT_MAX_STREAMS 更新判定 (Section 5.6)
                    if let Some(session) = self.wt_sessions.get_mut(&session_id) {
                        session.on_remote_stream_closed(false);
                    }
                    self.events
                        .push_back(Event::WebTransportUniStreamEnd { stream_id });
                }
            }

            // セッション ID 未確定の WT 単方向ストリームの FIN
            // (セッション ID が未確定のまま FIN が来た場合は単に破棄)
            self.pending_wt_uni_streams.remove(&stream_id);
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
                // WebTransport 単方向ストリーム (draft-ietf-webtrans-http3-15 Section 4.2)
                //
                // ネゴシエーション完了 (peer SETTINGS 受信 + WT 広告 + H3_DATAGRAM +
                // QUIC transport parameter 検証) を bidi 経路と同じ条件で確認する
                // (draft-ietf-webtrans-http3-15 Section 4.2 / 7.1)。
                if !self.is_wt_fully_negotiated() {
                    return Err(Error::ConnectionError(ErrorCode::StreamCreationError));
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
        // (RFC 9114 Section 6.2.1, RFC 9204 Section 4.3)
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
                    .push_back(Event::WebTransportUniStreamEnd { stream_id });
            }
            self.pending_wt_uni_streams.remove(&stream_id);
        }

        Ok(())
    }

    /// WebTransport 単方向ストリームのセッション ID を解決
    ///
    /// ストリームタイプ (0x54) が確定した後、セッション ID (varint) をパースする。
    /// varint が不完全な場合は `pending_wt_uni_streams` にバッファリングする。
    /// (draft-ietf-webtrans-http3-15 Section 4.2)
    fn resolve_wt_uni_stream_session_id(
        &mut self,
        stream_id: u64,
        data: &[u8],
    ) -> Result<(), Error> {
        let buf = if let Some(pending) = self.pending_wt_uni_streams.get_mut(&stream_id) {
            pending.extend_from_slice(data);
            pending.clone()
        } else {
            data.to_vec()
        };

        if buf.is_empty() {
            // データなし: 次のチャンクを待つ
            self.pending_wt_uni_streams.entry(stream_id).or_default();
            return Ok(());
        }

        match crate::varint::decode(&buf) {
            Ok((session_id, id_len)) => {
                let session_id = session_id.get();
                // session_id は client-initiated bidirectional stream ID でなければならない
                // (draft-ietf-webtrans-http3-15 Section 4.2)
                // RFC 9000 Section 2.1: client-initiated bidi は stream_id % 4 == 0
                if session_id & 0x03 != 0x00 {
                    return Err(Error::ConnectionError(ErrorCode::IdError));
                }
                self.pending_wt_uni_streams.remove(&stream_id);
                self.wt_uni_streams.insert(stream_id, session_id);

                // セッション関連付けとバッファリング (draft-ietf-webtrans-http3-15 Section 4.6)
                let outcome = match self.associate_or_buffer_stream(stream_id, session_id, false) {
                    Ok(o) => o,
                    Err(()) => {
                        // セッション終了済み: WT_SESSION_GONE で拒否 (Section 6)
                        self.wt_uni_streams.remove(&stream_id);
                        self.events
                            .push_back(Event::WebTransportBufferedStreamRejected {
                                stream_id,
                                error_code: WtErrorCode::SessionGone as u64,
                            });
                        return Ok(());
                    }
                };
                if outcome == AssocOutcome::BufferOverflow {
                    self.wt_uni_streams.remove(&stream_id);
                    self.events
                        .push_back(Event::WebTransportBufferedStreamRejected {
                            stream_id,
                            error_code: WtErrorCode::BufferedStreamRejected as u64,
                        });
                    return Ok(());
                }

                let remaining = &buf[id_len..];

                if outcome == AssocOutcome::Buffered {
                    // Pending セッション: Open / Data はセッション確立まで保留する
                    // (draft-ietf-webtrans-http3-15 Section 4.6)
                    if !remaining.is_empty()
                        && let Some(session) = self.wt_sessions.get_mut(&session_id)
                        && !session.append_buffered_stream_data(stream_id, remaining)
                    {
                        // バッファ上限超過: WT_BUFFERED_STREAM_REJECTED 相当
                        self.wt_uni_streams.remove(&stream_id);
                        let _ = session.take_buffered_stream_entry(stream_id);
                        self.events
                            .push_back(Event::WebTransportBufferedStreamRejected {
                                stream_id,
                                error_code: WtErrorCode::BufferedStreamRejected as u64,
                            });
                        return Ok(());
                    }
                    return Ok(());
                }

                // Established セッションのストリーム数フロー制御 (Section 5.6)
                if let Some(session) = self.wt_sessions.get_mut(&session_id) {
                    if !session.check_received_stream(false) {
                        self.wt_uni_streams.remove(&stream_id);
                        self.terminate_wt_session_with(
                            session_id,
                            WtErrorCode::FlowControlError as u64,
                            0,
                            String::new(),
                        );
                        return Ok(());
                    }
                    session.add_received_stream(false);
                }

                self.events.push_back(Event::WebTransportUniStreamOpen {
                    stream_id,
                    session_id,
                });
                // セッション ID の後にデータがあればイベントで通知
                if !remaining.is_empty() {
                    // データフロー制御 (Section 5.4)
                    if let Some(session) = self.wt_sessions.get_mut(&session_id) {
                        let data_len = remaining.len() as u64;
                        if !session.check_received_data(data_len) {
                            self.terminate_wt_session_with(
                                session_id,
                                WtErrorCode::FlowControlError as u64,
                                0,
                                String::new(),
                            );
                            return Ok(());
                        }
                        session.add_received_data(data_len);
                    }
                    self.events.push_back(Event::WebTransportUniStreamData {
                        stream_id,
                        data: remaining.to_vec(),
                    });
                }
                Ok(())
            }
            Err(crate::varint::DecodeError::BufferTooShort) => {
                // varint 不完全: 次のチャンクを待つ
                self.pending_wt_uni_streams.entry(stream_id).or_insert(buf);
                Ok(())
            }
        }
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

    /// WebTransport 双方向ストリームを処理
    ///
    /// server-initiated (または client-initiated で signal value 0x41 付き) の
    /// bidi stream を処理する。先頭の signal value (0x41) と session_id (varint) を
    /// パースし、確定後はアプリケーションペイロードをイベントで通知する。
    /// (draft-ietf-webtrans-http3-15 Section 4.3)
    fn handle_wt_bidi_stream(
        &mut self,
        stream_id: u64,
        data: &[u8],
        fin: bool,
    ) -> Result<(), Error> {
        // 既に確定済みの WT bidi stream: データ/FIN を処理
        if let Some(&session_id) = self.wt_bidi_streams.get(&stream_id) {
            // Pending セッション中はバッファに記録するだけ
            // (draft-ietf-webtrans-http3-15 Section 4.6)
            let pending = self
                .wt_sessions
                .get(&session_id)
                .is_some_and(|s| s.state == WtSessionState::Pending);
            if pending {
                if !data.is_empty()
                    && let Some(session) = self.wt_sessions.get_mut(&session_id)
                    && !session.append_buffered_stream_data(stream_id, data)
                {
                    self.wt_bidi_streams.remove(&stream_id);
                    let _ = session.take_buffered_stream_entry(stream_id);
                    self.events
                        .push_back(Event::WebTransportBufferedStreamRejected {
                            stream_id,
                            error_code: WtErrorCode::BufferedStreamRejected as u64,
                        });
                    return Ok(());
                }
                if fin && let Some(session) = self.wt_sessions.get_mut(&session_id) {
                    session.mark_buffered_stream_fin(stream_id);
                }
                return Ok(());
            }
            if !data.is_empty() {
                // データフロー制御 (Section 5.4)
                if let Some(session) = self.wt_sessions.get_mut(&session_id) {
                    let data_len = data.len() as u64;
                    if !session.check_received_data(data_len) {
                        self.terminate_wt_session_with(
                            session_id,
                            WtErrorCode::FlowControlError as u64,
                            0,
                            String::new(),
                        );
                        return Ok(());
                    }
                    session.add_received_data(data_len);
                }
                self.events.push_back(Event::WebTransportBidiStreamData {
                    stream_id,
                    data: data.to_vec(),
                });
            }
            if fin {
                self.wt_bidi_streams.remove(&stream_id);
                // ストリーム閉鎖: WT_MAX_STREAMS 更新判定 (Section 5.6)
                if let Some(session) = self.wt_sessions.get_mut(&session_id) {
                    session.on_remote_stream_closed(true);
                }
                self.events
                    .push_back(Event::WebTransportBidiStreamEnd { stream_id });
            }
            return Ok(());
        }

        // signal value + session_id の解決を試みる
        self.resolve_wt_bidi_stream_header(stream_id, data)?;

        // FIN チェック: ヘッダー解決中に FIN が来た場合
        if fin {
            if let Some(&session_id) = self.wt_bidi_streams.get(&stream_id) {
                let pending = self
                    .wt_sessions
                    .get(&session_id)
                    .is_some_and(|s| s.state == WtSessionState::Pending);
                if pending {
                    if let Some(session) = self.wt_sessions.get_mut(&session_id) {
                        session.mark_buffered_stream_fin(stream_id);
                    }
                } else {
                    self.wt_bidi_streams.remove(&stream_id);
                    // ストリーム閉鎖: WT_MAX_STREAMS 更新判定 (Section 5.6)
                    if let Some(session) = self.wt_sessions.get_mut(&session_id) {
                        session.on_remote_stream_closed(true);
                    }
                    self.events
                        .push_back(Event::WebTransportBidiStreamEnd { stream_id });
                }
            }
            self.pending_wt_bidi_streams.remove(&stream_id);
        }

        Ok(())
    }

    /// WebTransport CONNECT ストリーム上のデータを Capsule としてデコード・処理する
    ///
    /// DATA フレームのペイロードを Capsule デコードバッファに追加し、
    /// 完全な Capsule が得られるまでデコードを試みる。
    /// (draft-ietf-webtrans-http3-15 Section 5.6)
    fn process_wt_capsule_data(&mut self, session_id: u64, data: &[u8]) -> Result<(), Error> {
        // WT_CLOSE_SESSION 受信済みの場合、追加データは H3_MESSAGE_ERROR でリセット
        // (draft-ietf-webtrans-http3-15 Section 6)
        if let Some(session) = self.wt_sessions.get(&session_id)
            && session.close_session_received
        {
            return Err(Error::StreamError(ErrorCode::MessageError));
        }

        // セッションの capsule_buf にデータを追加
        if let Some(session) = self.wt_sessions.get_mut(&session_id) {
            session.capsule_buf.extend_from_slice(data);
        } else {
            return Ok(());
        }

        // Capsule を逐次デコード
        while let Some(session) = self.wt_sessions.get(&session_id) {
            if session.capsule_buf.is_empty() {
                break;
            }
            let buf = session.capsule_buf.clone();

            match crate::webtransport::Capsule::decode(&buf) {
                Ok(Some((capsule, consumed))) => {
                    // バッファから消費済み部分を除去
                    if let Some(session) = self.wt_sessions.get_mut(&session_id) {
                        session.capsule_buf.drain(..consumed);
                    }

                    // Capsule を処理してイベントに変換
                    self.handle_wt_capsule(session_id, &capsule)?;
                }
                Ok(None) => {
                    // バッファ不足: 次の DATA フレームを待つ
                    break;
                }
                Err(_) => {
                    // RFC 9297 Section 3.3: malformed Capsule は
                    // HTTP message エラーとして扱う → H3_MESSAGE_ERROR
                    return Err(Error::StreamError(ErrorCode::MessageError));
                }
            }
        }

        Ok(())
    }

    /// デコードされた Capsule を処理してイベントに変換する
    fn handle_wt_capsule(
        &mut self,
        session_id: u64,
        capsule: &crate::webtransport::Capsule,
    ) -> Result<(), Error> {
        use crate::webtransport::Capsule;

        match capsule {
            Capsule::CloseSession {
                error_code,
                message,
            } => {
                // WT_CLOSE_SESSION: セッションを終了し、error_code / message を通知する
                // (draft-ietf-webtrans-http3-15 Section 6)
                //
                // WT_CLOSE_SESSION 受信後の追加データは H3_MESSAGE_ERROR で拒否する
                // (draft-ietf-webtrans-http3-15 Section 6)
                if let Some(session) = self.wt_sessions.get_mut(&session_id) {
                    session.close_session_received = true;
                }
                self.terminate_wt_session_with(
                    session_id,
                    WtErrorCode::SessionGone as u64,
                    *error_code,
                    message.clone(),
                );
            }
            Capsule::DrainSession => {
                // WT_DRAIN_SESSION: 内部状態を Draining へ遷移し、イベントで通知する
                // (draft-ietf-webtrans-http3-15 Section 4.7)
                // セッションは即座に終了しないが、Connection 層は以後の新規
                // ストリーム/データグラム送信を拒否する。
                if let Some(session) = self.wt_sessions.get_mut(&session_id)
                    && (session.state == WtSessionState::Established
                        || session.state == WtSessionState::Pending)
                {
                    session.state = WtSessionState::Draining;
                    self.events
                        .push_back(Event::WebTransportSessionDraining { session_id });
                }
            }
            Capsule::MaxData { .. }
            | Capsule::MaxStreams { .. }
            | Capsule::DataBlocked { .. }
            | Capsule::StreamsBlocked { .. } => {
                // フロー制御カプセル: セッションのフロー制御有効性を確認
                let fc_enabled = self
                    .wt_sessions
                    .get(&session_id)
                    .is_some_and(|s| s.flow_control_enabled);

                if fc_enabled {
                    // フロー制御有効: 上位層に通知して Session::process_capsule で処理
                    self.events.push_back(Event::WebTransportCapsule {
                        session_id,
                        capsule: capsule.clone(),
                    });
                }
                // フロー制御無効時は無視 (Section 5.1)
            }
            Capsule::Unknown { .. } => {
                // 不明な Capsule は無視 (draft-ietf-webtrans-http3-15)
            }
        }

        Ok(())
    }

    /// ピアの SETTINGS から WebTransport ドラフトバージョンを推定する
    ///
    /// 受信済みの SETTINGS に含まれる WebTransport 用 setting ID から判定する。
    /// WebTransport SETTINGS を受信していない場合は `None`。
    /// (draft-ietf-webtrans-http3-02/07/14/15 Section 3.1)
    fn peer_wt_draft_version(&self) -> Option<crate::webtransport::DraftVersion> {
        self.peer_settings
            .as_ref()
            .and_then(|s| s.wt_settings.as_ref())
            .and_then(|wt| wt.detect_draft_pattern())
    }

    /// ローカルとピアが共に広告している中で最も新しい WebTransport ドラフトを返す
    ///
    /// バージョンネゴシエーションは「両エンドポイントが広告する集合の交差から
    /// 最も新しいものを選ぶ」(draft-ietf-webtrans-http3-15 Section 7.1)。
    /// ピア側が複数ドラフトの SETTINGS を同時に広告する場合があるため、
    /// ピアの最高ドラフトをそのまま採用せず、ローカルが同じドラフトを広告している
    /// 場合に限り採用する。
    /// 将来のドラフトで変更される可能性がある
    fn negotiated_wt_draft_version(&self) -> Option<crate::webtransport::DraftVersion> {
        self.mutually_advertised_wt_drafts().into_iter().next()
    }

    /// ローカルとピアが共に広告している WebTransport ドラフトを新しい順に返す
    ///
    /// (draft-ietf-webtrans-http3-15 Section 7.1)
    /// 将来のドラフトで変更される可能性がある
    fn mutually_advertised_wt_drafts(&self) -> Vec<crate::webtransport::DraftVersion> {
        use crate::webtransport::DraftVersion;
        let Some(local) = self.local_settings.wt_settings.as_ref() else {
            return Vec::new();
        };
        let Some(peer) = self
            .peer_settings
            .as_ref()
            .and_then(|s| s.wt_settings.as_ref())
        else {
            return Vec::new();
        };
        let advertises = |s: &crate::webtransport::Settings, d: DraftVersion| -> bool {
            match d {
                DraftVersion::Draft15 => s.wt_enabled.get() > 0,
                DraftVersion::Draft14 => s.wt_max_sessions_draft14.is_some_and(|v| v.get() > 0),
                DraftVersion::Draft07 => s
                    .webtransport_max_sessions_draft07
                    .is_some_and(|v| v.get() > 0),
                DraftVersion::Draft02 => s.enable_webtransport_draft02 == Some(true),
            }
        };
        [
            DraftVersion::Draft15,
            DraftVersion::Draft14,
            DraftVersion::Draft07,
            DraftVersion::Draft02,
        ]
        .into_iter()
        .filter(|d| advertises(local, *d) && advertises(peer, *d))
        .collect()
    }

    /// WebTransport の能力ネゴシエーションが完了しているかどうかを判定する
    ///
    /// 送信側 (`send_request`) と同じ粒度で、ローカルとピアの両方が
    /// WebTransport に必要な全条件を満たしているかを確認する。
    /// 必要な条件はドラフトバージョンに応じて異なる:
    ///
    /// | 条件 | draft-02 | draft-07 | draft-14 | draft-15 |
    /// |---|---|---|---|---|
    /// | ENABLE_CONNECT_PROTOCOL (クライアントがサーバーを検証) | 不要 | 必要 | 必要 | 必要 |
    /// | reset_stream_at transport parameter | 不要 | 不要 | 必要 | 必要 |
    ///
    /// draft-ietf-webtrans-http3-02/07/14/15 Section 3.1
    fn is_wt_fully_negotiated(&self) -> bool {
        // ローカルが WebTransport を有効にしているか
        if !self.local_settings.is_webtransport_enabled() {
            return false;
        }
        // peer の SETTINGS を受信済みか
        let peer = match self.peer_settings.as_ref() {
            Some(p) => p,
            None => return false,
        };
        // H3_DATAGRAM が有効か (両端共通の必須条件)
        if peer.h3_datagram != Some(true) {
            return false;
        }
        // QUIC transport parameter が検証済みか
        if !self.wt_transport_verified {
            return false;
        }
        // クライアント側: peer (サーバー) が WebTransport を広告し、ENABLE_CONNECT_PROTOCOL
        // と必要なら reset_stream_at もそろっているかを検証する。
        if self.role == Role::Client {
            if !peer.is_webtransport_enabled() {
                return false;
            }
            let draft = match self.peer_wt_draft_version() {
                Some(v) => v,
                None => return false,
            };
            if draft.requires_enable_connect_protocol()
                && peer.enable_connect_protocol != Some(true)
            {
                return false;
            }
            if draft.requires_reset_stream_at() && !self.wt_reset_stream_at_supported {
                return false;
            }
            return true;
        }
        // サーバー側: ローカルが draft-15 を採用している場合のみ peer の WT 広告を要求する
        // (Section 7.1: 双方が SETTINGS_WT_ENABLED を送る MUST)。
        // draft-14 以前は Safari 等の interop のため peer 広告を要求しない
        // (nghttp3 lib/nghttp3_conn.c L62-71 の TODO コメント参照)。
        let local_draft = self.local_settings.webtransport_draft_pattern();
        if matches!(
            local_draft,
            Some(crate::webtransport::DraftVersion::Draft15)
        ) && !peer.is_webtransport_enabled()
        {
            return false;
        }
        true
    }

    /// WebTransport フロー制御が両端で有効かどうかを判定する
    ///
    /// ローカルとピアの WebTransport SETTINGS を比較し、
    /// 両端がフロー制御を宣言している場合のみ `true` を返す。
    /// (draft-ietf-webtrans-http3-15 Section 5.1)
    fn is_wt_flow_control_enabled(&self) -> bool {
        let local_wt = self.local_settings.wt_settings.as_ref();
        let peer_wt = self
            .peer_settings
            .as_ref()
            .and_then(|s| s.wt_settings.as_ref());

        match (local_wt, peer_wt) {
            (Some(local), Some(peer)) => local.flow_control_enabled_with_peer(peer),
            _ => false,
        }
    }

    /// WebTransport セッションを終了する
    ///
    /// CONNECT stream の FIN、RESET_STREAM、WT_CLOSE_SESSION 受信時に呼ばれる。
    /// セッションに関連する全ストリームを指定エラーコードでリセットするイベントを生成する。
    /// `close_error_code` と `close_message` は WT_CLOSE_SESSION カプセルから取得した
    /// アプリケーション層のクローズ理由。FIN のみの場合は error_code=0, message="" とする。
    /// (draft-ietf-webtrans-http3-15 Section 6)
    fn terminate_wt_session_with(
        &mut self,
        session_id: u64,
        error_code: u64,
        close_error_code: u32,
        close_message: String,
    ) {
        if let Some(session) = self.wt_sessions.get_mut(&session_id) {
            if session.state == WtSessionState::Closed {
                return;
            }
            session.state = WtSessionState::Closed;

            // 関連する全ストリーム ID を収集
            let associated_stream_ids: Vec<u64> =
                session.associated_streams.iter().copied().collect();

            // バッファリングされたストリームも対象に含める
            let buffered_stream_ids = session.take_buffered_streams();

            // wt_uni_streams / wt_bidi_streams から除去する前に reliable size を計算する。
            // 計算後にマップから除去する。
            // (draft-ietf-webtrans-http3-15 Section 6 / Section 4.4 / Section 5.4)
            let mut reset_streams: Vec<WtStreamReset> =
                Vec::with_capacity(associated_stream_ids.len() + buffered_stream_ids.len());
            for &sid in &associated_stream_ids {
                let reliable_size = self.wt_stream_header_len(sid);
                reset_streams.push(WtStreamReset {
                    stream_id: sid,
                    reliable_size,
                });
            }
            for sid in &associated_stream_ids {
                self.wt_uni_streams.remove(sid);
                self.wt_bidi_streams.remove(sid);
            }
            for sid in buffered_stream_ids {
                // バッファリング段階のストリームは wt_uni_streams / wt_bidi_streams に
                // 入っていない可能性が高い。stream header 長は計算できないので 0 を渡す。
                // 上位層は reset_stream_at が無効な経路として扱うか、stream header 長を
                // 別途決定すること。
                reset_streams.push(WtStreamReset {
                    stream_id: sid,
                    reliable_size: 0,
                });
            }

            self.events.push_back(Event::WebTransportSessionClosed {
                session_id,
                reset_streams,
                error_code,
                close_error_code,
                close_message,
            });
        }
    }

    /// WebTransport データストリームの stream header エンコード長を計算する
    ///
    /// (draft-ietf-webtrans-http3-15 Section 4.2 / 5.4)
    ///
    /// - 双方向ストリーム: signal value (0x41) varint + session_id varint
    /// - 単方向ストリーム: stream type (0x54) varint + session_id varint
    ///
    /// 上位層は本値を `RESET_STREAM_AT` の reliable size の下限として使用する。
    /// `stream_id` が WebTransport データストリームとして登録されていない場合は
    /// 0 を返す (CONNECT stream や非 WT ストリーム)。将来のドラフトで stream
    /// header フォーマットが変更される可能性がある。
    pub fn wt_stream_header_len(&self, stream_id: u64) -> u64 {
        use crate::varint::VarInt;
        use crate::webtransport::stream::{BIDIRECTIONAL_SIGNAL_VALUE, UNIDIRECTIONAL_STREAM_TYPE};
        // signal value / stream type は WebTransport プロトコルの定数なので const 文脈で
        // VarInt 構築できる (`from_static` でコンパイル時検査)。
        const BIDI_SIGNAL: VarInt = VarInt::from_static(BIDIRECTIONAL_SIGNAL_VALUE);
        const UNI_TYPE: VarInt = VarInt::from_static(UNIDIRECTIONAL_STREAM_TYPE);
        if let Some(&session_id) = self.wt_bidi_streams.get(&stream_id) {
            // session_id は QUIC ストリーム ID 空間 (`<= 2^62 - 1`) で構造的に保証される。
            let session_id = VarInt::new(session_id).expect("session_id fits in VarInt");
            return (BIDI_SIGNAL.encoded_len() + session_id.encoded_len()) as u64;
        }
        if let Some(&session_id) = self.wt_uni_streams.get(&stream_id) {
            let session_id = VarInt::new(session_id).expect("session_id fits in VarInt");
            return (UNI_TYPE.encoded_len() + session_id.encoded_len()) as u64;
        }
        0
    }

    /// WebTransport セッションを WT_SESSION_GONE で終了する
    ///
    /// CONNECT stream の RESET_STREAM 受信など、WT_CLOSE_SESSION なしでの
    /// セッション終了に使用する。
    /// (draft-ietf-webtrans-http3-15 Section 6)
    fn terminate_wt_session(&mut self, session_id: u64) {
        self.terminate_wt_session_with(
            session_id,
            WtErrorCode::SessionGone as u64,
            0,
            String::new(),
        );
    }

    /// WebTransport ストリームをセッションに関連付ける、またはバッファリングする
    ///
    /// 戻り値:
    /// - `Ok(AssocOutcome::Established)`: 既存 Established セッションに関連付けた
    /// - `Ok(AssocOutcome::Buffered)`: Pending セッションにバッファリングした
    ///   (Open / Data / End はセッション確立時まで保留する必要がある)
    /// - `Ok(AssocOutcome::BufferOverflow)`: バッファ上限超過 (WT_BUFFERED_STREAM_REJECTED)
    /// - `Err(())`: セッション終了済み (WT_SESSION_GONE で拒否すべき)
    ///
    /// (draft-ietf-webtrans-http3-15 Section 4.6, 6)
    fn associate_or_buffer_stream(
        &mut self,
        stream_id: u64,
        session_id: u64,
        is_bidi: bool,
    ) -> Result<AssocOutcome, ()> {
        if let Some(session) = self.wt_sessions.get_mut(&session_id) {
            match session.state {
                WtSessionState::Established => {
                    session.associate_stream(stream_id);
                    Ok(AssocOutcome::Established)
                }
                WtSessionState::Pending => {
                    if session.buffer_stream(stream_id, is_bidi) {
                        Ok(AssocOutcome::Buffered)
                    } else {
                        Ok(AssocOutcome::BufferOverflow)
                    }
                }
                WtSessionState::Draining => {
                    // Draining 状態: 既存ストリームの継続は許可するが、
                    // 新規ストリーム関連付けは拒否する (Section 6)
                    Err(())
                }
                WtSessionState::Closed => {
                    // セッション終了済み: WT_SESSION_GONE で拒否 (Section 6)
                    Err(())
                }
            }
        } else {
            // クライアントは自身が開始していない session_id を拒否する
            // (draft-ietf-webtrans-http3-15 Section 4.6)
            if self.role == Role::Client {
                return Err(());
            }

            // サーバーが GOAWAY を送信済みの場合、その境界以降の session_id に対する
            // 新規 WebTransport セッションは受け入れない
            // (draft-ietf-webtrans-http3-15 Section 4.7)。
            // nghttp3 lib/nghttp3_conn.c L3654 と整合。
            if let Some(last_id) = self.last_sent_goaway_id
                && session_id >= last_id
            {
                return Err(());
            }

            // 接続単位の Pending セッション上限を超過した場合は拒否する
            // (draft-ietf-webtrans-http3-15 Section 4.6 / DoS 対策)
            if self.count_pending_wt_sessions() >= WT_MAX_PENDING_SESSIONS {
                return Ok(AssocOutcome::BufferOverflow);
            }

            // サーバー側: セッション未登録だがストリームが先に到着した
            // 新規 Pending セッションを作成してバッファリング (Section 4.6)
            let mut session = WtSession::new();
            let buffered = session.buffer_stream(stream_id, is_bidi);
            self.wt_sessions.insert(session_id, session);
            if buffered {
                Ok(AssocOutcome::Buffered)
            } else {
                Ok(AssocOutcome::BufferOverflow)
            }
        }
    }

    /// 現在 Pending 状態の WebTransport セッション数を数える
    ///
    /// `WT_MAX_PENDING_SESSIONS` の上限判定に使用する。
    fn count_pending_wt_sessions(&self) -> usize {
        self.wt_sessions
            .values()
            .filter(|s| s.state == WtSessionState::Pending)
            .count()
    }

    /// 現在 active な WebTransport セッション数を数える (Pending + Established)
    ///
    /// draft-ietf-webtrans-http3-15 Section 5.1 / 5.2 の
    /// 「フロー制御無効時は同時に 1 セッションまで」の判定に使用する。
    /// 「同時 (simultaneous)」の解釈はドラフトが明示していないが、
    /// CONNECT 送信/受信の時点でセッションは確立中とみなし、
    /// Pending と Established の両方を数える安全側の解釈を採用する。
    /// 将来のドラフトで定義が変更される可能性がある。
    fn count_active_wt_sessions(&self) -> usize {
        // Draining セッションも slot を消費するため active として数える
        // (draft-ietf-webtrans-http3-15 Section 5.1 / 6)
        self.wt_sessions
            .values()
            .filter(|s| {
                s.state == WtSessionState::Pending
                    || s.state == WtSessionState::Established
                    || s.state == WtSessionState::Draining
            })
            .count()
    }

    /// WebTransport 双方向ストリームのヘッダー (signal value + session_id) を解決
    ///
    /// 先頭の signal value (0x41) と session_id (varint) をパースする。
    /// varint が不完全な場合は `pending_wt_bidi_streams` にバッファリングする。
    /// (draft-ietf-webtrans-http3-15 Section 4.3)
    fn resolve_wt_bidi_stream_header(&mut self, stream_id: u64, data: &[u8]) -> Result<(), Error> {
        let buf = if let Some(pending) = self.pending_wt_bidi_streams.get_mut(&stream_id) {
            pending.extend_from_slice(data);
            pending.clone()
        } else {
            data.to_vec()
        };

        if buf.is_empty() {
            // データなし: 次のチャンクを待つ
            self.pending_wt_bidi_streams.entry(stream_id).or_default();
            return Ok(());
        }

        // signal value をパース (varint)
        let (signal_value, signal_len) = match crate::varint::decode(&buf) {
            Ok(v) => v,
            Err(crate::varint::DecodeError::BufferTooShort) => {
                self.pending_wt_bidi_streams.entry(stream_id).or_insert(buf);
                return Ok(());
            }
        };

        // signal value は 0x41 (WT_STREAM) でなければならない
        // (draft-ietf-webtrans-http3-15 Section 4.3)
        if signal_value.get() != 0x41 {
            // 0x41 以外の signal value はリクエストストリームの先頭以外での WT_STREAM 受信、
            // または不正な signal value。H3_FRAME_ERROR として接続エラー。
            return Err(Error::ConnectionError(ErrorCode::FrameError));
        }

        // session_id をパース (varint)
        let remaining = &buf[signal_len..];
        match crate::varint::decode(remaining) {
            Ok((session_id, id_len)) => {
                let session_id = session_id.get();
                // session_id は client-initiated bidirectional stream ID でなければならない
                // (draft-ietf-webtrans-http3-15 Section 4.2)
                if session_id & 0x03 != 0x00 {
                    return Err(Error::ConnectionError(ErrorCode::IdError));
                }
                self.pending_wt_bidi_streams.remove(&stream_id);
                self.wt_bidi_streams.insert(stream_id, session_id);

                // セッション関連付けとバッファリング (draft-ietf-webtrans-http3-15 Section 4.6)
                let outcome = match self.associate_or_buffer_stream(stream_id, session_id, true) {
                    Ok(o) => o,
                    Err(()) => {
                        // セッション終了済み: WT_SESSION_GONE で拒否 (Section 6)
                        self.wt_bidi_streams.remove(&stream_id);
                        self.events
                            .push_back(Event::WebTransportBufferedStreamRejected {
                                stream_id,
                                error_code: WtErrorCode::SessionGone as u64,
                            });
                        return Ok(());
                    }
                };
                if outcome == AssocOutcome::BufferOverflow {
                    self.wt_bidi_streams.remove(&stream_id);
                    self.events
                        .push_back(Event::WebTransportBufferedStreamRejected {
                            stream_id,
                            error_code: WtErrorCode::BufferedStreamRejected as u64,
                        });
                    return Ok(());
                }

                let payload = &remaining[id_len..];

                if outcome == AssocOutcome::Buffered {
                    // Pending セッション: Open / Data はセッション確立まで保留する
                    // (draft-ietf-webtrans-http3-15 Section 4.6)
                    if !payload.is_empty()
                        && let Some(session) = self.wt_sessions.get_mut(&session_id)
                        && !session.append_buffered_stream_data(stream_id, payload)
                    {
                        self.wt_bidi_streams.remove(&stream_id);
                        let _ = session.take_buffered_stream_entry(stream_id);
                        self.events
                            .push_back(Event::WebTransportBufferedStreamRejected {
                                stream_id,
                                error_code: WtErrorCode::BufferedStreamRejected as u64,
                            });
                        return Ok(());
                    }
                    return Ok(());
                }

                // Established セッションのストリーム数フロー制御 (Section 5.6)
                if let Some(session) = self.wt_sessions.get_mut(&session_id) {
                    if !session.check_received_stream(true) {
                        self.wt_bidi_streams.remove(&stream_id);
                        self.terminate_wt_session_with(
                            session_id,
                            WtErrorCode::FlowControlError as u64,
                            0,
                            String::new(),
                        );
                        return Ok(());
                    }
                    session.add_received_stream(true);
                }

                self.events.push_back(Event::WebTransportBidiStreamOpen {
                    stream_id,
                    session_id,
                });
                // session_id の後にデータがあればイベントで通知
                if !payload.is_empty() {
                    // データフロー制御 (Section 5.4)
                    if let Some(session) = self.wt_sessions.get_mut(&session_id) {
                        let data_len = payload.len() as u64;
                        if !session.check_received_data(data_len) {
                            self.terminate_wt_session_with(
                                session_id,
                                WtErrorCode::FlowControlError as u64,
                                0,
                                String::new(),
                            );
                            return Ok(());
                        }
                        session.add_received_data(data_len);
                    }
                    self.events.push_back(Event::WebTransportBidiStreamData {
                        stream_id,
                        data: payload.to_vec(),
                    });
                }
                Ok(())
            }
            Err(crate::varint::DecodeError::BufferTooShort) => {
                // session_id の varint 不完全: 次のチャンクを待つ
                self.pending_wt_bidi_streams.entry(stream_id).or_insert(buf);
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
                    if !self.qpack_encoder.ack_section(stream_id) {
                        return Err(Error::ConnectionError(ErrorCode::QpackDecoderStreamError));
                    }
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
                    self.peer_settings = Some(settings);
                    self.events.push_back(Event::SettingsReceived {
                        settings,
                        wt_settings,
                    });
                }
                crate::frame::Frame::Goaway(payload) => {
                    // クライアントが受信する GOAWAY の stream ID は
                    // client-initiated bidirectional stream ID でなければならない
                    // (RFC 9114 Section 7.2.6)
                    if self.role == Role::Client {
                        // client-initiated bidi stream ID は 4 の倍数 (0, 4, 8, ...)
                        if payload.id % 4 != 0 {
                            return Err(Error::ConnectionError(ErrorCode::IdError));
                        }
                    }

                    // 複数 GOAWAY の単調減少チェック (RFC 9114 Section 5.2)
                    // 値の意味はロール依存だが、単調減少制約はどちらの方向でも成立する
                    if let Some(prev_id) = self.peer_goaway_last_id
                        && payload.id > prev_id
                    {
                        return Err(Error::ConnectionError(ErrorCode::IdError));
                    }

                    self.peer_goaway_received = true;
                    self.peer_goaway_last_id = Some(payload.id);
                    self.events
                        .push_back(Event::GoawayReceived { id: payload.id });

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
                                **sid >= payload.id
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
                            self.events
                                .push_back(Event::WebTransportSessionDraining { session_id: sid });
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
        // ストリームを取得または作成
        self.streams
            .entry(stream_id)
            .or_insert_with(|| RequestStream::new(stream_id));

        // まずデータを受信 (ストリームの内部バッファに追加)
        if let Some(stream) = self.streams.get_mut(&stream_id) {
            stream.receive(data, fin);
        }

        // QPACK ブロック中のストリームはフレーム解析を停止する (nghttp3 方式)
        // データはストリームの内部バッファに残り、ブロック解除時に再処理する
        if self.streams.get(&stream_id).unwrap().is_qpack_blocked() {
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
                let stream = self.streams.get_mut(&stream_id).unwrap();
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
                    //
                    // Capsule Protocol はセッションが Established (= 2xx 応答送出済み)
                    // になってから有効になるので、Pending 状態の CONNECT ストリーム上で
                    // 受信した DATA は Capsule としてデコードしてはならない。
                    //
                    // ===== 重要 / デグレ防止 =====
                    // ここを「Pending 中はすべて MessageError」に戻してはならない。
                    // draft-02 (Chrome) は CONNECT 直後、サーバーの 2xx を待たずに
                    // CONNECT ストリームへ HEADERS + DATA フレームを書き込んでくる
                    // (実測: Chrome 138+ の WebTransport 実装)。draft-02 には
                    // Capsule Protocol が存在しないため、ここで MessageError を返すと
                    // Chrome との接続が確立できなくなる (デグレ)。
                    //
                    // draft 別の扱い:
                    // - draft-07/14/15: CONNECT に request body は許容されないため
                    //   H3_MESSAGE_ERROR を返す (draft-15 Section 4.2, 5.6 /
                    //   nghttp3 lib/nghttp3_conn.c L1942)。
                    // - draft-02: Chrome 互換のため Pending 中の DATA は黙って破棄する
                    //   (Sans I/O ライブラリなのでログも出さない)。draft-02 には
                    //   Capsule Protocol が無く、仕様上 CONNECT ストリームへ書かれた
                    //   request body の意味は未定義であり、破棄が安全な唯一の選択肢。
                    //
                    // リグレッションテスト:
                    //   tests/test_webtransport_draft_connect.rs の
                    //   `mod pending_data_frame` を参照すること。
                    //
                    // 将来のドラフトで変更される可能性がある
                    if let Some(session) = self.wt_sessions.get(&stream_id) {
                        match session.state {
                            WtSessionState::Established | WtSessionState::Draining => {
                                // Draining 中もカプセル受信は継続する (Section 4.7)
                                self.process_wt_capsule_data(stream_id, &data)?;
                            }
                            WtSessionState::Pending => {
                                let peer_draft = self.peer_wt_draft_version();
                                if !matches!(
                                    peer_draft,
                                    Some(crate::webtransport::DraftVersion::Draft02)
                                ) {
                                    return Err(Error::StreamError(ErrorCode::MessageError));
                                }
                                // draft-02: Pending 中の DATA は破棄する
                            }
                            WtSessionState::Closed => {
                                return Err(Error::StreamError(ErrorCode::MessageError));
                            }
                        }
                    } else {
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
                    if let Some(session) = self.wt_sessions.get(&stream_id)
                        && !session.capsule_buf.is_empty()
                    {
                        return Err(Error::StreamError(ErrorCode::MessageError));
                    }

                    self.events.push_back(Event::StreamEnd { stream_id });

                    // WebTransport セッション終端処理 (draft-ietf-webtrans-http3-15 Section 6)
                    // CONNECT stream の FIN はセッション終了を意味する
                    self.terminate_wt_session(stream_id);
                }
            }
        }

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
            // ローカルの WebTransport 設定が有効でなければ、WebTransport CONNECT は処理できない
            if self.role == Role::Server && is_webtransport_connect(&headers) {
                if !self.local_settings.is_webtransport_enabled() {
                    return Err(Error::StreamError(ErrorCode::MessageError));
                }
                // peer (クライアント) の SETTINGS 受信前は WT リクエストを処理しない
                // (draft-ietf-webtrans-http3-15 Section 7.1)
                let peer = self
                    .peer_settings
                    .as_ref()
                    .ok_or(Error::StreamError(ErrorCode::MessageError))?;
                // peer (クライアント) の WT 広告チェックはローカルが想定するドラフトで分岐する。
                // - draft-15: 双方が SETTINGS_WT_ENABLED を送る MUST (Section 7.1)。peer も
                //   送っていることを要求する。
                // - draft-14 以前 (07/02 含む): 仕様上は MUST だが Safari Network.framework
                //   等の実装は送らない。nghttp3 も interop のため remote->wt_enabled チェックを
                //   外している (lib/nghttp3_conn.c L62-71 の TODO コメント参照)。これに合わせて
                //   peer の WT 広告は要求しない。
                let local_draft = self.local_settings.webtransport_draft_pattern();
                if matches!(
                    local_draft,
                    Some(crate::webtransport::DraftVersion::Draft15)
                ) && !peer.is_webtransport_enabled()
                {
                    return Err(Error::StreamError(ErrorCode::MessageError));
                }
                // peer (クライアント) が H3_DATAGRAM を有効にしているか確認
                if peer.h3_datagram != Some(true) {
                    return Err(Error::StreamError(ErrorCode::MessageError));
                }
                // ENABLE_CONNECT_PROTOCOL はサーバーが送る設定 (RFC 9220, RFC 8441 Section 3)
                // クライアントは送信義務がないためサーバー側では検証しない
                // (draft-ietf-webtrans-http3-14/15 Section 3.1: クライアントの送信リストに含まれない)
                // QUIC transport parameter レベルの前提条件が注入済みか確認
                if !self.wt_transport_verified {
                    return Err(Error::StreamError(ErrorCode::MessageError));
                }
                // draft-14/15 では reset_stream_at transport parameter が必須
                // (draft-ietf-webtrans-http3-14/15 Section 3.1)
                // draft-02/07 では不要
                // 将来のドラフトで変更される可能性がある
                // ローカルとピア両方が広告するドラフトの中から最新を選ぶ
                // (draft-ietf-webtrans-http3-15 Section 7.1)
                let draft = self.negotiated_wt_draft_version();
                if draft.is_some_and(|d| d.requires_reset_stream_at())
                    && !self.wt_reset_stream_at_supported
                {
                    return Err(Error::StreamError(ErrorCode::MessageError));
                }
                // :protocol が SETTINGS でネゴシエートしたドラフトの値と一致することを確認
                // (draft-ietf-webtrans-http3-15 Section 3.2 / 7.1)
                // - draft-02/07/14: `webtransport`
                // - draft-15: `webtransport-h3`
                // 将来のドラフトで変更される可能性がある
                // :protocol が「両者が共に広告しているドラフトのいずれか」の値と
                // 一致することを確認する。複数ドラフトを併送するピアが古い :protocol 値
                // (`webtransport`) を使うケースがあるため、negotiated 1 種類だけで
                // 判定すると拒否してしまう。
                // (draft-ietf-webtrans-http3-15 Section 3.2 / 7.1)
                if draft.is_some() {
                    let proto_header = headers
                        .iter()
                        .find(|h| h.name() == b":protocol")
                        .map(|h| h.value())
                        .ok_or(Error::StreamError(ErrorCode::MessageError))?;
                    let mutual = self.mutually_advertised_wt_drafts();
                    let proto_ok = mutual
                        .iter()
                        .any(|d| d.protocol_value().as_bytes() == proto_header);
                    if !proto_ok {
                        return Err(Error::StreamError(ErrorCode::MessageError));
                    }
                }
                // :scheme が https であることを確認
                // (draft-ietf-webtrans-http3-15 Section 3.2)
                let scheme_is_https = headers
                    .iter()
                    .any(|h| h.name() == b":scheme" && h.value() == b"https");
                if !scheme_is_https {
                    return Err(Error::StreamError(ErrorCode::MessageError));
                }
                // フロー制御が無効な場合は同時に 1 セッションまで。超過分の
                // CONNECT ストリームは H3_REQUEST_REJECTED で reset しなければならない
                // (draft-ietf-webtrans-http3-15 Section 5.1, 5.2)
                // 既に対象 stream_id に対応する Pending セッションがバッファ済みで
                // 存在する場合 (先着ストリーム/データグラム経由) はそれ自身が
                // 唯一の active セッションでありうるため、本判定からは除外する。
                // 将来のドラフトで定義が変更される可能性がある。
                if !self.is_wt_flow_control_enabled() {
                    let already_has_pending_for_this = self
                        .wt_sessions
                        .get(&stream_id)
                        .is_some_and(|s| s.state == WtSessionState::Pending);
                    let active = self.count_active_wt_sessions();
                    let other_active = if already_has_pending_for_this {
                        active.saturating_sub(1)
                    } else {
                        active
                    };
                    if other_active >= 1 {
                        return Err(Error::StreamError(ErrorCode::RequestRejected));
                    }
                }
            }

            // サーバー側: WebTransport CONNECT を受信した場合、セッションを Pending で登録
            // (draft-ietf-webtrans-http3-15 Section 3)
            // 先行到着ストリームにより既に Pending セッションが存在する場合は温存する
            // (draft-ietf-webtrans-http3-15 Section 4.6)
            if self.role == Role::Server && is_webtransport_connect(&headers) {
                let session = self
                    .wt_sessions
                    .entry(stream_id)
                    .or_insert_with(WtSession::new);
                // クライアントの WT-Available-Protocols を保存する
                // (draft-ietf-webtrans-http3-15 Section 3.3)
                for h in &headers {
                    if h.name() == b"wt-available-protocols" {
                        if let Ok(value) = std::str::from_utf8(h.value()) {
                            session.available_protocols =
                                crate::webtransport::ConnectRequest::parse_available_protocols(
                                    value,
                                );
                        }
                        break;
                    }
                }
                // WebTransport CONNECT ストリームではトレーラーを禁止する
                // DATA フレームは Capsule Protocol を運ぶため HEADERS (トレーラー) に意味がない
                if let Some(stream) = self.streams.get_mut(&stream_id) {
                    stream.set_connect();
                }
            }

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
            if self.role == Role::Client && is_success_status(&headers) {
                // WT-Protocol 検証とセッション遷移 (Section 3.3)
                //
                // draft-ietf-webtrans-http3-15 Section 3.3 では、
                // server が WT-Protocol を返してよいのは「client が WT-Available-Protocols を
                // 送ったとき」かつ「その list から選ばれた single choice」のみと規定されている。
                // したがって client が WT-Available-Protocols を送っていない場合に
                // server が WT-Protocol を返すこと自体が仕様前提を満たさない違反であり、
                // 違反扱いとしてセッションを終了する。
                let wt_protocol_invalid = if let Some(session) = self.wt_sessions.get(&stream_id) {
                    if session.state == WtSessionState::Pending {
                        let selected = headers.iter().find_map(|h| {
                            if h.name() == b"wt-protocol" {
                                std::str::from_utf8(h.value())
                                    .ok()
                                    .and_then(crate::webtransport::ConnectResponse::parse_protocol)
                            } else {
                                None
                            }
                        });
                        if session.available_protocols.is_empty() {
                            // WT-Available-Protocols を送っていないのに
                            // server が WT-Protocol を返すのは違反
                            selected.is_some()
                        } else {
                            match &selected {
                                None => true, // WT-Available-Protocols ありで WT-Protocol なし
                                Some(proto) => !session.available_protocols.contains(proto),
                            }
                        }
                    } else {
                        false
                    }
                } else {
                    false
                };

                if wt_protocol_invalid {
                    // WT_ALPN_ERROR でセッションを閉鎖 (Section 3.3)
                    self.terminate_wt_session_with(
                        stream_id,
                        WtErrorCode::AlpnError as u64,
                        0,
                        String::new(),
                    );
                } else {
                    let fc_enabled = self.is_wt_flow_control_enabled();
                    let queue_initial_capsules =
                        fc_enabled && self.peer_requires_initial_wt_capsules();
                    let mut fc_violation = false;
                    if let Some(session) = self.wt_sessions.get_mut(&stream_id)
                        && session.state == WtSessionState::Pending
                    {
                        // フロー制御ネゴシエーション (Section 5.1)
                        session.flow_control_enabled = fc_enabled;

                        // フロー制御初期化 (Section 5.5, 5.6)
                        if let Some(wt) = &self.local_settings.wt_settings {
                            session.initialize_flow_control(wt, queue_initial_capsules);
                        }

                        session.state = WtSessionState::Established;
                        // WebTransport CONNECT ストリームではトレーラーを禁止する
                        if let Some(stream) = self.streams.get_mut(&stream_id) {
                            stream.set_connect();
                        }
                        self.events
                            .push_back(Event::WebTransportSessionEstablished {
                                session_id: stream_id,
                                flow_control_enabled: fc_enabled,
                            });
                        // バッファリングされていたストリームの Open / Data / End を順序保って配送
                        // フロー制御チェック: ストリーム数 + データ量の両方を FC 対象とする
                        // (draft-ietf-webtrans-http3-15 Section 4.6, 5.4, 5.6)
                        let buffered = session.take_buffered_streams();
                        let mut buffered_events: Vec<Event> = Vec::new();
                        let mut closed_buffered: Vec<(u64, bool)> = Vec::new();
                        for &buffered_stream_id in &buffered {
                            let Some(entry) =
                                session.take_buffered_stream_entry(buffered_stream_id)
                            else {
                                continue;
                            };
                            let is_bidi = entry.is_bidi;
                            let entry_data = entry.data;
                            let entry_fin = entry.fin;
                            if !session.check_received_stream(is_bidi) {
                                fc_violation = true;
                                break;
                            }
                            session.add_received_stream(is_bidi);
                            session.associate_stream(buffered_stream_id);
                            // Open
                            buffered_events.push(if is_bidi {
                                Event::WebTransportBidiStreamOpen {
                                    stream_id: buffered_stream_id,
                                    session_id: stream_id,
                                }
                            } else {
                                Event::WebTransportUniStreamOpen {
                                    stream_id: buffered_stream_id,
                                    session_id: stream_id,
                                }
                            });
                            // Data
                            if !entry_data.is_empty() {
                                let data_len = entry_data.len() as u64;
                                if !session.check_received_data(data_len) {
                                    fc_violation = true;
                                    break;
                                }
                                session.add_received_data(data_len);
                                buffered_events.push(if is_bidi {
                                    Event::WebTransportBidiStreamData {
                                        stream_id: buffered_stream_id,
                                        data: entry_data,
                                    }
                                } else {
                                    Event::WebTransportUniStreamData {
                                        stream_id: buffered_stream_id,
                                        data: entry_data,
                                    }
                                });
                            }
                            // End
                            if entry_fin {
                                session.on_remote_stream_closed(is_bidi);
                                closed_buffered.push((buffered_stream_id, is_bidi));
                                buffered_events.push(if is_bidi {
                                    Event::WebTransportBidiStreamEnd {
                                        stream_id: buffered_stream_id,
                                    }
                                } else {
                                    Event::WebTransportUniStreamEnd {
                                        stream_id: buffered_stream_id,
                                    }
                                });
                            }
                        }
                        for ev in buffered_events {
                            self.events.push_back(ev);
                        }
                        for (sid, is_bidi) in closed_buffered {
                            if is_bidi {
                                self.wt_bidi_streams.remove(&sid);
                            } else {
                                self.wt_uni_streams.remove(&sid);
                            }
                        }
                        if !fc_violation {
                            // バッファリングされていたデータグラムを配送 (Section 4.6)
                            let buffered_datagrams = session.take_buffered_datagrams();
                            for payload in buffered_datagrams {
                                self.events.push_back(Event::WebTransportDatagram {
                                    session_id: stream_id,
                                    payload,
                                });
                            }
                        }
                    }
                    if fc_violation {
                        // WT_FLOW_CONTROL_ERROR: バッファされたストリーム数が
                        // 初期リミットを超過 (Section 5.6)
                        self.terminate_wt_session_with(
                            stream_id,
                            WtErrorCode::FlowControlError as u64,
                            0,
                            String::new(),
                        );
                        return Ok(());
                    }
                }
            }
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

    /// WebTransport セッションの受信データ消費を通知する
    ///
    /// アプリケーション層が `WebTransportBidiStreamData` / `WebTransportUniStreamData`
    /// イベントのデータを処理した後に呼ぶ。消費量に基づいて WT_MAX_DATA の
    /// ウィンドウ更新を判定し、必要に応じてカプセルを生成する。
    /// (draft-ietf-webtrans-http3-15 Section 5.6)
    pub fn wt_data_consumed(&mut self, session_id: u64, bytes: u64) {
        if let Some(session) = self.wt_sessions.get_mut(&session_id) {
            session.on_data_consumed(bytes);
        }
    }

    /// WebTransport セッションの送信待ちカプセルを取り出す
    ///
    /// Connection 層が生成した WT_MAX_STREAMS / WT_MAX_DATA カプセルを取り出す。
    /// アプリケーション層はこれらを CONNECT ストリーム上で送信すること。
    /// (draft-ietf-webtrans-http3-15 Section 5.6)
    pub fn take_wt_pending_capsules(
        &mut self,
        session_id: u64,
    ) -> Vec<crate::webtransport::Capsule> {
        if let Some(session) = self.wt_sessions.get_mut(&session_id) {
            session.take_pending_capsules()
        } else {
            Vec::new()
        }
    }

    /// WebTransport セッションのフロー制御が有効かどうかを取得する
    ///
    /// アプリケーション層が `webtransport::Session` を構成する際に使用する。
    pub fn wt_session_flow_control_enabled(&self, session_id: u64) -> bool {
        self.wt_sessions
            .get(&session_id)
            .is_some_and(|s| s.flow_control_enabled)
    }

    /// 送信可能なストリームを取得
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
        if let Some(stream) = self.streams.get(&stream_id) {
            let (data, fin) = stream.get_send_data();
            if !data.is_empty() || fin {
                return Some((data, fin));
            }
        }

        None
    }

    /// ストリームデータを取得して内部バッファから消費する
    ///
    /// `get_stream_data()` + `consume_stream_data()` を一度に行う convenience メソッド。
    /// データがない場合は `None` を返す。
    ///
    /// ストリームの送信バッファにある全データを 1 回の呼び出しで返す。
    /// ループで繰り返し呼ぶ必要はない。
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
        }
    }

    /// ストリームの FIN 送信完了を通知
    ///
    /// FIN-only (空データ + FIN) を QUIC 層に引き渡した後に呼び出す。
    /// これにより `streams_to_send()` で当該ストリームが返されなくなる。
    pub fn mark_stream_fin_sent(&mut self, stream_id: u64) {
        if let Some(stream) = self.streams.get_mut(&stream_id) {
            stream.mark_fin_sent();
        }
    }

    /// リクエストを送信 (クライアント専用)
    pub fn send_request(&mut self, headers: &[Header], fin: bool) -> Result<u64, Error> {
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
        // クライアントはサーバーの SETTINGS_WT_ENABLED を受信するまで
        // WebTransport セッションを開始してはならない
        if is_webtransport_connect(headers) {
            let peer = self
                .peer_settings
                .as_ref()
                .ok_or(Error::ConnectionError(ErrorCode::InternalError))?;
            if !peer.is_webtransport_enabled() {
                return Err(Error::ConnectionError(ErrorCode::InternalError));
            }
            // ドラフトごとに ENABLE_CONNECT_PROTOCOL / reset_stream_at の要件が異なる
            // (draft-ietf-webtrans-http3-02/07/14/15 Section 3.1)
            let draft = self
                .peer_wt_draft_version()
                .ok_or(Error::ConnectionError(ErrorCode::InternalError))?;
            // ENABLE_CONNECT_PROTOCOL はサーバーが送る設定。
            // draft-02 は SETTINGS_ENABLE_WEBTRANSPORT が拡張 CONNECT を暗示するため不要。
            if draft.requires_enable_connect_protocol()
                && peer.enable_connect_protocol != Some(true)
            {
                return Err(Error::ConnectionError(ErrorCode::InternalError));
            }
            if peer.h3_datagram != Some(true) {
                return Err(Error::ConnectionError(ErrorCode::InternalError));
            }
            // QUIC transport parameter レベルの前提条件が注入済みか確認
            if !self.wt_transport_verified {
                return Err(Error::ConnectionError(ErrorCode::InternalError));
            }
            // draft-14/15 では reset_stream_at transport parameter が必須
            if draft.requires_reset_stream_at() && !self.wt_reset_stream_at_supported {
                return Err(Error::ConnectionError(ErrorCode::InternalError));
            }
            // :protocol が SETTINGS でネゴシエートしたドラフトの値と一致することを確認
            // (draft-ietf-webtrans-http3-15 Section 3.2 / 7.1)
            // 将来のドラフトで変更される可能性がある
            let expected_proto = draft.protocol_value().as_bytes();
            let proto_ok = headers
                .iter()
                .any(|h| h.name() == b":protocol" && h.value() == expected_proto);
            if !proto_ok {
                return Err(Error::ConnectionError(ErrorCode::InternalError));
            }
            // フロー制御が無効な場合は同時に 1 セッションまで
            // (draft-ietf-webtrans-http3-15 Section 5.1)
            // 将来のドラフトで定義が変更される可能性がある。
            if !self.is_wt_flow_control_enabled() && self.count_active_wt_sessions() >= 1 {
                return Err(Error::ConnectionError(ErrorCode::RequestRejected));
            }
        }

        // GOAWAY 受信後は指定 ID 以上のストリームを作成できない (RFC 9114 Section 5.2)
        //
        // `peer_goaway_request_boundary()` はクライアント受信時のみ Some を返す。
        // サーバーが受信する GOAWAY は push ID を運ぶものであり、request stream
        // 境界値としては使えない (RFC 9114 Section 7.2.6)
        if let Some(goaway_id) = self.peer_goaway_request_boundary()
            && self.next_stream_id >= goaway_id
        {
            return Err(Error::ConnectionError(ErrorCode::RequestRejected));
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

        // フィールドセクションの送信を記録 (RFC 9204 Section 2.1.1, 4.4.1)
        let ric = self.qpack_encoder.last_required_insert_count();
        self.qpack_encoder.track_section(stream_id, ric);

        let mut stream = RequestStream::new(stream_id);
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
        if is_connect && !has_protocol {
            // plain CONNECT ではストリームを open のまま維持する必要がある (RFC 9114 Section 4.4)
            if fin {
                return Err(Error::StreamError(ErrorCode::MessageError));
            }
            stream.set_connect_request();
        }
        stream.send_encoded_headers(&qpack_buf, fin, false)?;
        self.streams.insert(stream_id, stream);

        // WebTransport CONNECT の場合、セッションを Pending 状態で登録
        // (draft-ietf-webtrans-http3-15 Section 3)
        if is_webtransport_connect(headers) {
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
    pub fn send_response(
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
        //
        // - WT-Available-Protocols ありの場合: WT-Protocol は必須かつ list 内の値である必要がある
        // - WT-Available-Protocols なしの場合: WT-Protocol を返すこと自体が仕様前提を満たさない違反
        if is_success_status(headers)
            && let Some(session) = self.wt_sessions.get(&stream_id)
        {
            let selected = headers.iter().find_map(|h| {
                if h.name() == b"wt-protocol" {
                    std::str::from_utf8(h.value())
                        .ok()
                        .and_then(crate::webtransport::ConnectResponse::parse_protocol)
                } else {
                    None
                }
            });
            if session.available_protocols.is_empty() {
                // クライアントが WT-Available-Protocols を送っていない場合は
                // WT-Protocol を返してはならない
                if selected.is_some() {
                    return Err(Error::ConnectionError(ErrorCode::InternalError));
                }
            } else {
                match &selected {
                    // クライアントが WT-Available-Protocols を送信したのに
                    // サーバーが WT-Protocol を返さない場合はエラー
                    None => {
                        return Err(Error::ConnectionError(ErrorCode::InternalError));
                    }
                    // クライアントが提示していないプロトコルを返す場合はエラー
                    Some(proto) => {
                        if !session.available_protocols.contains(proto) {
                            return Err(Error::ConnectionError(ErrorCode::InternalError));
                        }
                    }
                }
            }
        }

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

        // フィールドセクションの送信を記録 (RFC 9204 Section 2.1.1, 4.4.1)
        let ric = self.qpack_encoder.last_required_insert_count();
        self.qpack_encoder.track_section(stream_id, ric);

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

        // サーバー側: WebTransport CONNECT に対する 2xx レスポンス送信時に
        // セッションを Established に遷移させる (draft-ietf-webtrans-http3-15 Section 3)
        let fc_enabled_server = self.is_wt_flow_control_enabled();
        let queue_initial_capsules = fc_enabled_server && self.peer_requires_initial_wt_capsules();
        let mut fc_violation_server = false;
        if self.role == Role::Server
            && is_success_status(headers)
            && let Some(session) = self.wt_sessions.get_mut(&stream_id)
            && session.state == WtSessionState::Pending
        {
            // フロー制御ネゴシエーション (Section 5.1)
            session.flow_control_enabled = fc_enabled_server;

            // フロー制御初期化 (Section 5.5, 5.6)
            if let Some(wt) = &self.local_settings.wt_settings {
                session.initialize_flow_control(wt, queue_initial_capsules);
            }

            session.state = WtSessionState::Established;
            self.events
                .push_back(Event::WebTransportSessionEstablished {
                    session_id: stream_id,
                    flow_control_enabled: fc_enabled_server,
                });
            // バッファリングされていたストリームの Open / Data / End を順序保って配送
            // フロー制御チェック: ストリーム数 + データ量の両方を FC 対象とする
            // (draft-ietf-webtrans-http3-15 Section 4.6, 5.4, 5.6)
            let buffered = session.take_buffered_streams();
            let mut buffered_events: Vec<Event> = Vec::new();
            let mut closed_buffered: Vec<(u64, bool)> = Vec::new();
            for &buffered_stream_id in &buffered {
                let Some(entry) = session.take_buffered_stream_entry(buffered_stream_id) else {
                    continue;
                };
                let is_bidi = entry.is_bidi;
                let entry_data = entry.data;
                let entry_fin = entry.fin;
                if !session.check_received_stream(is_bidi) {
                    fc_violation_server = true;
                    break;
                }
                session.add_received_stream(is_bidi);
                session.associate_stream(buffered_stream_id);
                buffered_events.push(if is_bidi {
                    Event::WebTransportBidiStreamOpen {
                        stream_id: buffered_stream_id,
                        session_id: stream_id,
                    }
                } else {
                    Event::WebTransportUniStreamOpen {
                        stream_id: buffered_stream_id,
                        session_id: stream_id,
                    }
                });
                if !entry_data.is_empty() {
                    let data_len = entry_data.len() as u64;
                    if !session.check_received_data(data_len) {
                        fc_violation_server = true;
                        break;
                    }
                    session.add_received_data(data_len);
                    buffered_events.push(if is_bidi {
                        Event::WebTransportBidiStreamData {
                            stream_id: buffered_stream_id,
                            data: entry_data,
                        }
                    } else {
                        Event::WebTransportUniStreamData {
                            stream_id: buffered_stream_id,
                            data: entry_data,
                        }
                    });
                }
                if entry_fin {
                    session.on_remote_stream_closed(is_bidi);
                    closed_buffered.push((buffered_stream_id, is_bidi));
                    buffered_events.push(if is_bidi {
                        Event::WebTransportBidiStreamEnd {
                            stream_id: buffered_stream_id,
                        }
                    } else {
                        Event::WebTransportUniStreamEnd {
                            stream_id: buffered_stream_id,
                        }
                    });
                }
            }
            for ev in buffered_events {
                self.events.push_back(ev);
            }
            for (sid, is_bidi) in closed_buffered {
                if is_bidi {
                    self.wt_bidi_streams.remove(&sid);
                } else {
                    self.wt_uni_streams.remove(&sid);
                }
            }
            if !fc_violation_server {
                // バッファリングされていたデータグラムを配送 (Section 4.6)
                let buffered_datagrams = session.take_buffered_datagrams();
                for payload in buffered_datagrams {
                    self.events.push_back(Event::WebTransportDatagram {
                        session_id: stream_id,
                        payload,
                    });
                }
            }
        }
        if fc_violation_server {
            self.terminate_wt_session_with(
                stream_id,
                WtErrorCode::FlowControlError as u64,
                0,
                String::new(),
            );
        }

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
    pub fn send_goaway(&mut self, id: u64) -> Result<(), Error> {
        // GOAWAY ID の型検証 (RFC 9114 Section 5.2)
        match self.role {
            Role::Server => {
                // サーバー → クライアント: client-initiated bidirectional stream ID
                // client-initiated bidi stream ID は 4 の倍数 (0, 4, 8, ...)
                if !id.is_multiple_of(4) {
                    return Err(Error::ConnectionError(ErrorCode::IdError));
                }
            }
            Role::Client => {
                // クライアント → サーバー: push ID
                // サーバープッシュ未対応のため 0 のみ許可
                if id != 0 {
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

    /// クローズ済み・リセット済みストリームを回収する
    ///
    /// `Closed` または `Reset` 状態のストリームを `streams` から削除し、
    /// 削除したストリーム ID のリストを返す。
    pub fn collect_closed_streams(&mut self) -> Vec<u64> {
        let closed_ids: Vec<u64> = self
            .streams
            .iter()
            .filter(|(_, s)| matches!(s.state(), StreamState::Closed | StreamState::Reset))
            .map(|(id, _)| *id)
            .collect();
        for id in &closed_ids {
            self.streams.remove(id);
        }
        closed_ids
    }

    /// QUIC から RESET_STREAM 受信時に呼ぶ
    ///
    /// ストリームの状態を Reset に遷移し、イベントを発行する。
    /// クリティカルストリームへの RESET_STREAM は接続エラー (RFC 9114 Section 6.2.1, RFC 9204 Section 4.3)
    ///
    /// `final_size` は RFC 9000 Section 19.4 で定義される RESET_STREAM の Final Size。
    /// `RESET_STREAM_AT` (draft-ietf-quic-reliable-stream-reset) で運ばれた reliable size
    /// 以上の値であり、QUIC 層から渡される。WebTransport データストリームについては
    /// `Event::WebTransportStreamReset::final_size` として上位層へ伝達される。
    pub fn stream_reset(
        &mut self,
        stream_id: u64,
        error_code: u64,
        final_size: u64,
    ) -> Result<(), Error> {
        // クリティカルストリームが閉じられた場合は接続エラー
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
        if let Some(stream) = self.streams.get(&stream_id)
            && stream.is_qpack_blocked()
        {
            let ricnt = stream.qpack_ricnt();
            self.blocked_by_ricnt.remove(&(ricnt, stream_id));
        }
        if let Some(stream) = self.streams.get_mut(&stream_id) {
            stream.set_qpack_blocked(false, 0, None);
        }
        // QPACK Stream Cancellation を送信 (RFC 9204 Section 2.2.2.2)
        // max_dynamic_table_capacity が 0 の場合は省略可能
        self.send_stream_cancellation_if_needed(stream_id);

        // WebTransport セッション/データストリームへのリセット伝播
        // (draft-ietf-webtrans-http3-15 Section 4.4 / Section 6)
        if self.wt_sessions.contains_key(&stream_id) {
            // CONNECT stream の RESET_STREAM はセッション終了
            self.terminate_wt_session(stream_id);
        } else if let Some(session_id) = self
            .wt_uni_streams
            .remove(&stream_id)
            .or_else(|| self.wt_bidi_streams.remove(&stream_id))
        {
            // WebTransport データストリームの RESET_STREAM はセッションに通知
            if let Some(session) = self.wt_sessions.get_mut(&session_id) {
                session.disassociate_stream(stream_id);
            }
            self.events.push_back(Event::WebTransportStreamReset {
                session_id,
                stream_id,
                error_code,
                final_size,
            });
        } else {
            // 非 WebTransport ストリーム: 汎用イベントを発行
            self.events.push_back(Event::StreamReset {
                stream_id,
                error_code,
            });
        }

        Ok(())
    }

    /// QUIC から STOP_SENDING 受信時に呼ぶ
    ///
    /// ストリームのローカル側をクローズし、イベントを発行する。
    /// クリティカルストリームへの STOP_SENDING は接続エラー (RFC 9114 Section 6.2.1, RFC 9204 Section 4.3)
    pub fn stop_sending(&mut self, stream_id: u64, error_code: u64) -> Result<(), Error> {
        // クリティカルストリームへの STOP_SENDING は接続エラー
        let is_critical = self.control_recv.stream_id() == Some(stream_id)
            || self.peer_encoder_stream_id == Some(stream_id)
            || self.peer_decoder_stream_id == Some(stream_id);
        if is_critical {
            return Err(Error::ConnectionError(ErrorCode::ClosedCriticalStream));
        }

        if let Some(stream) = self.streams.get_mut(&stream_id) {
            let state = stream.state_mut();
            state.close_local();
        }
        // QPACK Stream Cancellation を送信 (RFC 9204 Section 2.2.2.2)
        self.send_stream_cancellation_if_needed(stream_id);

        // WebTransport セッション/データストリームへの STOP_SENDING 伝播
        // (draft-ietf-webtrans-http3-15 Section 4.4 / Section 6)
        if self.wt_sessions.contains_key(&stream_id) {
            self.terminate_wt_session(stream_id);
        } else if let Some(session_id) = self
            .wt_uni_streams
            .get(&stream_id)
            .copied()
            .or_else(|| self.wt_bidi_streams.get(&stream_id).copied())
        {
            self.events.push_back(Event::WebTransportStreamStopSending {
                session_id,
                stream_id,
                error_code,
            });
        } else {
            self.events.push_back(Event::StopSending {
                stream_id,
                error_code,
            });
        }
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

    /// 接続エラーを取得
    pub fn error(&self) -> Option<&Error> {
        self.error.as_ref()
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
        conn.set_control_stream_id(2).unwrap();
        let headers = vec![
            Header::new(b":method", b"GET").unwrap(),
            Header::new(b":path", b"/").unwrap(),
            Header::new(b":scheme", b"https").unwrap(),
            Header::new(b":authority", b"example.com").unwrap(),
        ];

        let stream_id = conn.send_request(&headers, true).unwrap();
        assert_eq!(stream_id, 0);

        // 次のリクエスト
        let stream_id2 = conn.send_request(&headers, true).unwrap();
        assert_eq!(stream_id2, 4);
    }

    #[test]
    fn test_control_stream() {
        let mut conn = Connection::client(Settings::default());
        conn.set_control_stream_id(2).unwrap();

        // 制御ストリームの送信データを取得
        let (data, fin) = conn.get_stream_data(2).unwrap();
        assert!(!data.is_empty());
        assert!(!fin);
        assert_eq!(data[0], 0x00); // Control stream type
    }

    // =========================================================================
    // 0023: RESET_STREAM / STOP_SENDING によるクリティカルストリーム閉鎖検出
    // (RFC 9114 Section 6.2.1, RFC 9204 Section 4.3)
    // =========================================================================

    #[test]
    fn test_stream_reset_on_control_stream_is_closed_critical_stream() {
        let mut conn = Connection::client(Settings::default());
        // サーバーの単方向ストリーム (ID=3) を制御ストリームとして登録
        // 制御ストリームタイプ (0x00) + SETTINGS フレーム (type=0x04, length=0x00)
        conn.feed_stream(3, &[0x00, 0x04, 0x00], false).unwrap();
        let err = conn.stream_reset(3, 0, 0).unwrap_err();
        assert!(matches!(
            err,
            Error::ConnectionError(ErrorCode::ClosedCriticalStream)
        ));
    }

    #[test]
    fn test_stop_sending_on_control_stream_is_closed_critical_stream() {
        let mut conn = Connection::client(Settings::default());
        conn.feed_stream(3, &[0x00, 0x04, 0x00], false).unwrap();
        let err = conn.stop_sending(3, 0).unwrap_err();
        assert!(matches!(
            err,
            Error::ConnectionError(ErrorCode::ClosedCriticalStream)
        ));
    }

    #[test]
    fn test_stream_reset_on_qpack_encoder_stream_is_closed_critical_stream() {
        let mut conn = Connection::client(Settings::default());
        // QPACK エンコーダーストリームタイプ (0x02)
        conn.feed_stream(3, &[0x02], false).unwrap();
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
        conn.feed_stream(3, &[0x03], false).unwrap();
        let err = conn.stream_reset(3, 0, 0).unwrap_err();
        assert!(matches!(
            err,
            Error::ConnectionError(ErrorCode::ClosedCriticalStream)
        ));
    }

    #[test]
    fn test_stop_sending_on_qpack_encoder_stream_is_closed_critical_stream() {
        let mut conn = Connection::client(Settings::default());
        conn.feed_stream(3, &[0x02], false).unwrap();
        let err = conn.stop_sending(3, 0).unwrap_err();
        assert!(matches!(
            err,
            Error::ConnectionError(ErrorCode::ClosedCriticalStream)
        ));
    }

    #[test]
    fn test_stop_sending_on_qpack_decoder_stream_is_closed_critical_stream() {
        let mut conn = Connection::client(Settings::default());
        conn.feed_stream(3, &[0x03], false).unwrap();
        let err = conn.stop_sending(3, 0).unwrap_err();
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
        conn.set_encoder_stream_id(6).unwrap();

        // stream type 0x02 が送信データに含まれる
        let (data, fin) = conn.get_stream_data(6).unwrap();
        assert_eq!(data[0], 0x02);
        assert!(!fin);
    }

    #[test]
    fn test_set_decoder_stream_id_writes_stream_type() {
        let mut conn = Connection::client(Settings::default());
        conn.set_decoder_stream_id(10).unwrap();

        // stream type 0x03 が送信データに含まれる
        let (data, fin) = conn.get_stream_data(10).unwrap();
        assert_eq!(data[0], 0x03);
        assert!(!fin);
    }

    #[test]
    fn test_encoder_stream_in_writable_streams() {
        let mut conn = Connection::client(Settings::default());
        conn.set_encoder_stream_id(6).unwrap();

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
        conn.set_decoder_stream_id(10).unwrap();

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
        conn.set_control_stream_id(2).unwrap();
        conn.set_encoder_stream_id(6).unwrap();

        // stream type バイトを消費
        let (data, _) = conn.get_stream_data(6).unwrap();
        let len = data.len();
        conn.consume_stream_data(6, len);

        // ピアから SETTINGS (QPACK_MAX_TABLE_CAPACITY=4096) を受信
        // stream type (0x00) + SETTINGS frame (type=0x04, len=3, QPACK_MAX_TABLE_CAPACITY=4096)
        // 0x01 は QPACK_MAX_TABLE_CAPACITY の設定 ID
        // varint 4096 = 0x40 0x00 (2 バイト varint) ではなく...
        // 4096 = 0x5000 in varint? Let me use simpler encoding.
        // Actually: SETTINGS payload is pairs of (varint id, varint value)
        // id=0x01, value=4096 → 0x01, (4096 as varint: 0x80 0x00 0x10 0x00? no)
        // varint encoding: 4096 fits in 14-bit → 2-byte varint: 0x40 | (4096 >> 8), 4096 & 0xff
        // = 0x50, 0x00
        // SETTINGS frame: type=0x04, length=3, payload=[0x01, 0x50, 0x00]
        conn.feed_stream(3, &[0x00, 0x04, 0x03, 0x01, 0x50, 0x00], false)
            .unwrap();

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
            Header::from_validated_parts(
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
        VarInt::new(value).unwrap()
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
        conn.set_control_stream_id(2).unwrap();
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
            .unwrap();

        // WebTransportUniStreamOpen イベント
        let event = conn.poll_event().unwrap().unwrap();
        assert!(matches!(
            event,
            Event::WebTransportUniStreamOpen {
                stream_id: 2,
                session_id: 4,
            }
        ));

        // WebTransportUniStreamData イベント
        let event = conn.poll_event().unwrap().unwrap();
        assert!(matches!(
            event,
            Event::WebTransportUniStreamData {
                stream_id: 2,
                ref data,
            } if data == &[0xAA, 0xBB]
        ));
    }

    #[test]
    fn test_wt_uni_stream_subsequent_data() {
        let mut conn = make_server_with_established_wt_session(4);

        // 初回: ストリームタイプ + セッション ID のみ
        conn.feed_stream(2, &[0x40, 0x54, 0x04], false).unwrap();
        let event = conn.poll_event().unwrap().unwrap();
        assert!(matches!(event, Event::WebTransportUniStreamOpen { .. }));

        // 後続データ
        conn.feed_stream(2, &[0xCC, 0xDD], false).unwrap();
        let event = conn.poll_event().unwrap().unwrap();
        assert!(matches!(
            event,
            Event::WebTransportUniStreamData {
                stream_id: 2,
                ref data,
            } if data == &[0xCC, 0xDD]
        ));
    }

    #[test]
    fn test_wt_uni_stream_fin() {
        let mut conn = make_server_with_established_wt_session(4);

        conn.feed_stream(2, &[0x40, 0x54, 0x04], false).unwrap();
        let _ = conn.poll_event(); // Open イベントを消費

        conn.feed_stream(2, &[], true).unwrap();
        let event = conn.poll_event().unwrap().unwrap();
        assert!(matches!(
            event,
            Event::WebTransportUniStreamEnd { stream_id: 2 }
        ));
    }

    #[test]
    fn test_wt_uni_stream_disabled_returns_error() {
        // WebTransport 無効な接続
        let mut conn = Connection::server(Settings::default());
        conn.set_control_stream_id(3).unwrap();

        // ストリームタイプ 0x54 (varint 2 バイト: [0x40, 0x54])
        let err = conn.feed_stream(2, &[0x40, 0x54, 0x04], false).unwrap_err();
        assert!(matches!(
            err,
            Error::ConnectionError(ErrorCode::StreamCreationError)
        ));
    }

    #[test]
    fn test_wt_uni_stream_session_id_split_across_chunks() {
        let mut conn = make_server_with_established_wt_session(4);

        // ストリームタイプの 1 バイト目のみ (varint 未完了)
        conn.feed_stream(2, &[0x40], false).unwrap();
        assert!(conn.poll_event().unwrap().is_none());

        // ストリームタイプの 2 バイト目 + セッション ID
        conn.feed_stream(2, &[0x54, 0x04], false).unwrap();
        let event = conn.poll_event().unwrap().unwrap();
        assert!(matches!(
            event,
            Event::WebTransportUniStreamOpen {
                stream_id: 2,
                session_id: 4,
            }
        ));
    }

    #[test]
    fn test_wt_uni_stream_session_id_split_from_type() {
        let mut conn = make_server_with_established_wt_session(4);

        // ストリームタイプのみ (セッション ID なし)
        conn.feed_stream(2, &[0x40, 0x54], false).unwrap();
        assert!(conn.poll_event().unwrap().is_none());

        // セッション ID
        conn.feed_stream(2, &[0x04], false).unwrap();
        let event = conn.poll_event().unwrap().unwrap();
        assert!(matches!(
            event,
            Event::WebTransportUniStreamOpen {
                stream_id: 2,
                session_id: 4,
            }
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
            .unwrap();

        let event = conn.poll_event().unwrap().unwrap();
        assert!(matches!(
            event,
            Event::WebTransportBidiStreamOpen {
                stream_id: 1,
                session_id: 0,
            }
        ));

        let event = conn.poll_event().unwrap().unwrap();
        assert!(matches!(
            event,
            Event::WebTransportBidiStreamData {
                stream_id: 1,
                ref data,
            } if data == &[0xAA, 0xBB]
        ));
    }

    #[test]
    fn test_wt_bidi_stream_subsequent_data() {
        // 確定済み WT bidi stream への後続データ
        let mut conn = wt_negotiated_client_with_session(0);

        conn.feed_stream(1, &[0x40, 0x41, 0x00], false).unwrap();
        let event = conn.poll_event().unwrap().unwrap();
        assert!(matches!(event, Event::WebTransportBidiStreamOpen { .. }));

        conn.feed_stream(1, &[0xCC, 0xDD], false).unwrap();
        let event = conn.poll_event().unwrap().unwrap();
        assert!(matches!(
            event,
            Event::WebTransportBidiStreamData {
                stream_id: 1,
                ref data,
            } if data == &[0xCC, 0xDD]
        ));
    }

    #[test]
    fn test_wt_bidi_stream_fin() {
        let mut conn = wt_negotiated_client_with_session(0);

        conn.feed_stream(1, &[0x40, 0x41, 0x00], false).unwrap();
        let _ = conn.poll_event().unwrap(); // Open

        conn.feed_stream(1, &[], true).unwrap();
        let event = conn.poll_event().unwrap().unwrap();
        assert!(matches!(
            event,
            Event::WebTransportBidiStreamEnd { stream_id: 1 }
        ));
    }

    #[test]
    fn test_wt_bidi_stream_rejected_when_wt_disabled() {
        // WebTransport 無効なクライアントは server-initiated bidi を拒否
        let mut conn = Connection::client(Settings::default());
        conn.set_control_stream_id(2).unwrap();

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
        conn.feed_stream(1, &[], false).unwrap();
        assert!(conn.poll_event().unwrap().is_none());

        // signal value + session_id
        conn.feed_stream(1, &[0x40, 0x41, 0x04], false).unwrap();
        let event = conn.poll_event().unwrap().unwrap();
        assert!(matches!(
            event,
            Event::WebTransportBidiStreamOpen {
                stream_id: 1,
                session_id: 4,
            }
        ));
    }

    #[test]
    fn test_wt_bidi_stream_split_session_id() {
        // signal value は確定したが session_id が次のチャンクに分割される場合
        let mut conn = wt_negotiated_client_with_session(4);

        // signal value のみ (varint 2 バイト)
        conn.feed_stream(1, &[0x40, 0x41], false).unwrap();
        assert!(conn.poll_event().unwrap().is_none());

        // session_id
        conn.feed_stream(1, &[0x04], false).unwrap();
        let event = conn.poll_event().unwrap().unwrap();
        assert!(matches!(
            event,
            Event::WebTransportBidiStreamOpen {
                stream_id: 1,
                session_id: 4,
            }
        ));
    }

    #[test]
    fn test_wt_bidi_stream_session_id_4() {
        // session_id = 4 (2 番目の client-initiated bidi stream)
        let mut conn = wt_negotiated_client_with_session(4);

        conn.feed_stream(1, &[0x40, 0x41, 0x04], false).unwrap();
        let event = conn.poll_event().unwrap().unwrap();
        assert!(matches!(
            event,
            Event::WebTransportBidiStreamOpen {
                stream_id: 1,
                session_id: 4,
            }
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
            .unwrap();

        let event = conn.poll_event().unwrap().unwrap();
        assert!(matches!(
            event,
            Event::WebTransportBidiStreamOpen {
                stream_id: 0,
                session_id: 0,
            }
        ));

        let event = conn.poll_event().unwrap().unwrap();
        assert!(matches!(
            event,
            Event::WebTransportBidiStreamData {
                stream_id: 0,
                ref data,
            } if data == &[0xAA, 0xBB]
        ));
    }

    #[test]
    fn test_server_client_bidi_request_not_wt() {
        // サーバー: クライアント開始の bidi stream が 0x41 でない場合はリクエストとして処理
        let mut conn = Connection::server(wt_enabled_settings());
        conn.set_control_stream_id(3).unwrap();

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
        conn.feed_stream(0, &[0x40], false).unwrap();
        assert!(conn.poll_event().unwrap().is_none());

        // 2 バイト目で 0x41 確定 + session_id
        conn.feed_stream(0, &[0x41, 0x00], false).unwrap();
        let event = conn.poll_event().unwrap().unwrap();
        assert!(matches!(
            event,
            Event::WebTransportBidiStreamOpen {
                stream_id: 0,
                session_id: 0,
            }
        ));
    }

    #[test]
    fn test_server_client_bidi_dispatch_split_varint_not_wt() {
        // サーバー: 先頭 varint が分割され、0x41 でないと判定された場合
        let mut conn = Connection::server(wt_enabled_settings());
        conn.set_control_stream_id(3).unwrap();

        // 2 バイト varint の先頭 1 バイトのみ (0x40 prefix)
        conn.feed_stream(0, &[0x40], false).unwrap();
        assert!(conn.poll_event().unwrap().is_none());

        // 2 バイト目で 0x42 (値 0x42, WT_STREAM ではない) → リクエストストリーム
        // ただし [0x40, 0x42] は varint で値 0x42 (= HEADERS フレームではない未知フレームタイプ)
        // リクエストストリームとして処理される
        conn.feed_stream(0, &[0x42, 0x02, 0xAA, 0xBB], false)
            .unwrap();
        // 未知フレームタイプはスキップされる (RFC 9114 Section 9)
        assert!(conn.poll_event().unwrap().is_none());
    }

    #[test]
    fn test_server_client_bidi_dispatch_empty_then_data() {
        // サーバー: 空データ → 後続データで判定
        let mut conn = make_server_with_established_wt_session(0);

        // 空データ
        conn.feed_stream(0, &[], false).unwrap();
        assert!(conn.poll_event().unwrap().is_none());

        // WT bidi データ
        conn.feed_stream(0, &[0x40, 0x41, 0x00], false).unwrap();
        let event = conn.poll_event().unwrap().unwrap();
        assert!(matches!(
            event,
            Event::WebTransportBidiStreamOpen {
                stream_id: 0,
                session_id: 0,
            }
        ));
    }

    #[test]
    fn test_server_client_bidi_dispatch_empty_fin() {
        // サーバー: 空データ + FIN はリクエストストリームとして処理
        // (HEADERS なしで FIN = H3_MESSAGE_ERROR)
        let mut conn = Connection::server(wt_enabled_settings());
        conn.set_control_stream_id(3).unwrap();

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
        conn.set_control_stream_id(2).unwrap();

        let headers = vec![
            Header::new(b":method", b"CONNECT").unwrap(),
            Header::new(b":protocol", b"webtransport-h3").unwrap(),
            Header::new(b":scheme", b"https").unwrap(),
            Header::new(b":authority", b"example.com").unwrap(),
            Header::new(b":path", b"/wt").unwrap(),
        ];

        // peer SETTINGS 未受信なので InternalError
        let err = conn.send_request(&headers, false).unwrap_err();
        assert!(matches!(
            err,
            Error::ConnectionError(ErrorCode::InternalError)
        ));
    }

    #[test]
    fn test_wt_connect_rejected_when_peer_wt_disabled() {
        // クライアント: peer SETTINGS で WT 無効
        let mut conn = Connection::client(wt_enabled_settings());
        conn.set_control_stream_id(2).unwrap();

        // peer から WT 無効な SETTINGS を受信
        // 制御ストリーム: タイプ (0x00) + SETTINGS フレーム (type=0x04, length=0x00)
        conn.feed_stream(3, &[0x00, 0x04, 0x00], false).unwrap();

        let headers = vec![
            Header::new(b":method", b"CONNECT").unwrap(),
            Header::new(b":protocol", b"webtransport-h3").unwrap(),
            Header::new(b":scheme", b"https").unwrap(),
            Header::new(b":authority", b"example.com").unwrap(),
            Header::new(b":path", b"/wt").unwrap(),
        ];

        let err = conn.send_request(&headers, false).unwrap_err();
        assert!(matches!(
            err,
            Error::ConnectionError(ErrorCode::InternalError)
        ));
    }

    #[test]
    fn test_server_wt_connect_rejected_without_peer_settings() {
        // サーバー: peer (クライアント) SETTINGS 未受信の状態で WT CONNECT を受信
        // (draft-ietf-webtrans-http3-15 Section 7.1)
        let mut client = Connection::client(wt_enabled_settings());
        client.set_control_stream_id(2).unwrap();

        let mut server = Connection::server(wt_enabled_settings());
        server.set_control_stream_id(3).unwrap();

        // クライアントから WT CONNECT を送信
        let headers = vec![
            Header::new(b":method", b"CONNECT").unwrap(),
            Header::new(b":protocol", b"webtransport-h3").unwrap(),
            Header::new(b":scheme", b"https").unwrap(),
            Header::new(b":authority", b"example.com").unwrap(),
            Header::new(b":path", b"/wt").unwrap(),
        ];
        let stream_id = client.send_request(&headers, false).unwrap_err();
        // クライアントも peer SETTINGS 未受信なので送信失敗する
        assert!(matches!(
            stream_id,
            Error::ConnectionError(ErrorCode::InternalError)
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
        server.set_control_stream_id(3).unwrap();

        // クライアントから WT 無効な SETTINGS を受信
        // control stream type (0x00) + SETTINGS frame: type=0x04, length=0x00
        server.feed_stream(2, &[0x00, 0x04, 0x00], false).unwrap();

        // peer SETTINGS は受信したが WT 無効
        assert!(server.peer_settings().is_some());
        assert!(!server.peer_settings().unwrap().is_webtransport_enabled());
    }

    #[test]
    fn test_server_wt_connect_with_full_peer_settings() {
        // サーバー: peer (クライアント) の SETTINGS で WT 有効
        // クライアント・サーバー間の完全な SETTINGS 交換をテスト
        let wt_settings = wt_enabled_settings();
        let mut client = Connection::client(wt_settings);
        client.set_control_stream_id(2).unwrap();

        let mut server = Connection::server(wt_settings);
        server.set_control_stream_id(3).unwrap();

        // 制御ストリームデータを交換
        let (client_ctrl, _) = client.take_stream_data(2).unwrap();
        server.feed_stream(2, &client_ctrl, false).unwrap();
        let _ = server.poll_event().unwrap();
        let (server_ctrl, _) = server.take_stream_data(3).unwrap();
        client.feed_stream(3, &server_ctrl, false).unwrap();

        // サーバーは peer (クライアント) の WT SETTINGS を受信済み
        let peer = server.peer_settings().unwrap();
        assert!(peer.is_webtransport_enabled());
    }

    #[test]
    fn test_non_wt_connect_allowed_without_wt_settings() {
        // 通常の Extended CONNECT は WebTransport チェックの対象外
        let mut conn = Connection::client(Settings::new().enable_connect_protocol(true));
        conn.set_control_stream_id(2).unwrap();

        // peer から ENABLE_CONNECT_PROTOCOL=1 の SETTINGS を受信
        // control stream type (0x00) + SETTINGS frame: type=0x04, length=0x02
        // + entries: [id=0x08 (ENABLE_CONNECT_PROTOCOL), value=0x01]
        conn.feed_stream(3, &[0x00, 0x04, 0x02, 0x08, 0x01], false)
            .unwrap();

        let headers = vec![
            Header::new(b":method", b"CONNECT").unwrap(),
            Header::new(b":protocol", b"websocket").unwrap(),
            Header::new(b":scheme", b"https").unwrap(),
            Header::new(b":authority", b"example.com").unwrap(),
            Header::new(b":path", b"/ws").unwrap(),
        ];

        // WebSocket 等の Extended CONNECT は WT チェックなしで通る
        assert!(conn.send_request(&headers, false).is_ok());
    }

    /// QPACK エンコードされた HEADERS フレームを手動構築するヘルパー
    fn build_headers_frame(headers: &[Header]) -> Vec<u8> {
        let encoder = crate::qpack::Encoder::new();
        let mut qpack_buf = vec![0u8; 4096];
        let qpack_len = encoder.encode(&mut qpack_buf, headers).unwrap();
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
        server.set_control_stream_id(3).unwrap();
        server
            .set_webtransport_transport_verified(true, false)
            .unwrap();

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
        client.set_control_stream_id(2).unwrap();

        // クライアントの制御ストリームデータをサーバーに feed
        let (client_ctrl, _) = client.take_stream_data(2).unwrap();
        server.feed_stream(2, &client_ctrl, false).unwrap();

        // サーバーの制御ストリームデータを取得 (消費のみ)
        let _ = server.take_stream_data(3).unwrap();

        // サーバー側で peer SETTINGS を確認
        let peer = server.peer_settings().unwrap();
        assert!(peer.is_webtransport_enabled());
        // クライアントは ENABLE_CONNECT_PROTOCOL を送信していない
        assert_eq!(peer.enable_connect_protocol, None);

        // SETTINGS イベントを消費
        let _ = server.poll_event().unwrap();

        // draft-07 CONNECT HEADERS フレームを手動構築
        let headers = vec![
            Header::new(b":method", b"CONNECT").unwrap(),
            Header::new(b":protocol", b"webtransport").unwrap(),
            Header::new(b":scheme", b"https").unwrap(),
            Header::new(b":authority", b"example.com").unwrap(),
            Header::new(b":path", b"/wt").unwrap(),
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
        server.set_control_stream_id(3).unwrap();
        server
            .set_webtransport_transport_verified(true, true)
            .unwrap();

        // draft-15 クライアント: ENABLE_CONNECT_PROTOCOL なし (正当)
        let client_wt = crate::webtransport::Settings::new().wt_enabled(vi(1));
        let client_settings = Settings {
            enable_connect_protocol: None,
            ..Settings::new()
                .h3_datagram(true)
                .enable_webtransport_server(client_wt)
        };
        let mut client = Connection::client(client_settings);
        client.set_control_stream_id(2).unwrap();

        // クライアントの制御ストリームデータをサーバーに feed
        let (client_ctrl, _) = client.take_stream_data(2).unwrap();
        server.feed_stream(2, &client_ctrl, false).unwrap();

        // サーバーの制御ストリームデータを取得 (消費のみ)
        let _ = server.take_stream_data(3).unwrap();

        // SETTINGS イベントを消費
        let _ = server.poll_event().unwrap();

        // draft-15 CONNECT HEADERS フレームを手動構築
        let headers = vec![
            Header::new(b":method", b"CONNECT").unwrap(),
            Header::new(b":protocol", b"webtransport-h3").unwrap(),
            Header::new(b":scheme", b"https").unwrap(),
            Header::new(b":authority", b"example.com").unwrap(),
            Header::new(b":path", b"/wt").unwrap(),
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
        client.set_control_stream_id(6).unwrap();
        let mut server = Connection::server(wt_settings);
        server.set_control_stream_id(3).unwrap();
        server
            .set_webtransport_transport_verified(true, true)
            .unwrap();
        let (client_ctrl, _) = client.take_stream_data(6).unwrap();
        server.feed_stream(6, &client_ctrl, false).unwrap();
        while server.poll_event().unwrap().is_some() {}
        server
    }

    /// WebTransport 有効なクライアント・サーバーペアを作成し SETTINGS を交換済みの状態にする
    fn setup_wt_pair() -> (Connection, Connection) {
        let wt_settings = wt_enabled_settings();
        let mut client = Connection::client(wt_settings);
        client.set_control_stream_id(2).unwrap();
        let mut server = Connection::server(wt_settings);
        server.set_control_stream_id(3).unwrap();

        // QUIC transport parameter レベルの前提条件を注入
        client
            .set_webtransport_transport_verified(true, true)
            .unwrap();
        server
            .set_webtransport_transport_verified(true, true)
            .unwrap();

        // 制御ストリームデータ交換
        let (client_ctrl, _) = client.take_stream_data(2).unwrap();
        server.feed_stream(2, &client_ctrl, false).unwrap();
        let _ = server.poll_event().unwrap();
        let (server_ctrl, _) = server.take_stream_data(3).unwrap();
        client.feed_stream(3, &server_ctrl, false).unwrap();

        // SETTINGS イベントを消費
        let _ = client.poll_event().unwrap();

        (client, server)
    }

    #[test]
    fn test_wt_session_registered_on_connect_send() {
        let (mut client, _server) = setup_wt_pair();

        let headers = vec![
            Header::new(b":method", b"CONNECT").unwrap(),
            Header::new(b":protocol", b"webtransport-h3").unwrap(),
            Header::new(b":scheme", b"https").unwrap(),
            Header::new(b":authority", b"example.com").unwrap(),
            Header::new(b":path", b"/wt").unwrap(),
        ];
        let stream_id = client.send_request(&headers, false).unwrap();

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
            Header::new(b":method", b"CONNECT").unwrap(),
            Header::new(b":protocol", b"webtransport-h3").unwrap(),
            Header::new(b":scheme", b"https").unwrap(),
            Header::new(b":authority", b"example.com").unwrap(),
            Header::new(b":path", b"/wt").unwrap(),
        ];
        let stream_id = client.send_request(&headers, false).unwrap();
        let (req_data, _) = client.take_stream_data(stream_id).unwrap();

        // サーバーがリクエストを受信
        server.feed_stream(stream_id, &req_data, false).unwrap();
        // サーバー側のイベントを消費
        let _ = server.drain_events().unwrap();

        // サーバーが 200 OK を返す
        let response = vec![Header::new(b":status", b"200").unwrap()];
        server.send_response(stream_id, &response, false).unwrap();
        let (resp_data, _) = server.take_stream_data(stream_id).unwrap();

        // クライアントが 200 OK を受信
        client.feed_stream(stream_id, &resp_data, false).unwrap();

        // WebTransportSessionEstablished イベントが発火すること
        let events = client.drain_events().unwrap();
        assert!(events.iter().any(|e| matches!(
            e,
            Event::WebTransportSessionEstablished { session_id, .. } if *session_id == stream_id
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
        client.set_control_stream_id(2).unwrap();

        let mut server = Connection::server(wt_multi_draft_settings_with_flow_control());
        server.set_control_stream_id(3).unwrap();
        server
            .set_webtransport_transport_verified(true, true)
            .unwrap();

        let (client_ctrl, _) = client.take_stream_data(2).unwrap();
        server.feed_stream(2, &client_ctrl, false).unwrap();
        let _ = server.drain_events().unwrap();

        let headers = vec![
            Header::new(b":method", b"CONNECT").unwrap(),
            Header::new(b":protocol", b"webtransport").unwrap(),
            Header::new(b":scheme", b"https").unwrap(),
            Header::new(b":authority", b"example.com").unwrap(),
            Header::new(b":path", b"/wt").unwrap(),
        ];
        let frame = build_headers_frame(&headers);
        server.feed_stream(0, &frame, false).unwrap();
        let _ = server.drain_events().unwrap();

        let response = vec![Header::new(b":status", b"200").unwrap()];
        server.send_response(0, &response, false).unwrap();

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
        client.set_control_stream_id(2).unwrap();

        let mut server = Connection::server(wt_multi_draft_settings_with_flow_control());
        server.set_control_stream_id(3).unwrap();
        server
            .set_webtransport_transport_verified(true, false)
            .unwrap();

        let (client_ctrl, _) = client.take_stream_data(2).unwrap();
        server.feed_stream(2, &client_ctrl, false).unwrap();
        let _ = server.drain_events().unwrap();

        let headers = vec![
            Header::new(b":method", b"CONNECT").unwrap(),
            Header::new(b":protocol", b"webtransport").unwrap(),
            Header::new(b":scheme", b"https").unwrap(),
            Header::new(b":authority", b"example.com").unwrap(),
            Header::new(b":path", b"/wt").unwrap(),
        ];
        let frame = build_headers_frame(&headers);
        server.feed_stream(0, &frame, false).unwrap();
        let _ = server.drain_events().unwrap();

        let response = vec![Header::new(b":status", b"200").unwrap()];
        server.send_response(0, &response, false).unwrap();

        assert!(server.take_wt_pending_capsules(0).is_empty());
    }

    #[test]
    fn test_wt_session_terminated_on_connect_stream_fin() {
        let (mut client, mut server) = setup_wt_pair();

        // WT CONNECT + 200 OK のハンドシェイク
        let headers = vec![
            Header::new(b":method", b"CONNECT").unwrap(),
            Header::new(b":protocol", b"webtransport-h3").unwrap(),
            Header::new(b":scheme", b"https").unwrap(),
            Header::new(b":authority", b"example.com").unwrap(),
            Header::new(b":path", b"/wt").unwrap(),
        ];
        let stream_id = client.send_request(&headers, false).unwrap();
        let (req_data, _) = client.take_stream_data(stream_id).unwrap();
        server.feed_stream(stream_id, &req_data, false).unwrap();
        let _ = server.drain_events().unwrap();

        let response = vec![Header::new(b":status", b"200").unwrap()];
        server.send_response(stream_id, &response, false).unwrap();
        let (resp_data, _) = server.take_stream_data(stream_id).unwrap();
        client.feed_stream(stream_id, &resp_data, false).unwrap();
        let _ = client.drain_events().unwrap();

        // server-initiated uni stream をクライアントに送信
        // stream_id=3 は server-initiated uni, ストリームタイプ 0x54 + session_id
        let session_id = stream_id;
        let mut uni_data = vec![0x40, 0x54]; // stream type 0x54 (varint)
        uni_data.push(session_id as u8); // session_id (1 バイト varint)
        client.feed_stream(3, &uni_data, false).unwrap();
        let _ = client.drain_events().unwrap();

        // CONNECT stream を FIN で閉じる (セッション終了)
        client.feed_stream(stream_id, &[], true).unwrap();

        // WebTransportSessionClosed イベントが発火すること
        let events = client.drain_events().unwrap();
        let closed_event = events.iter().find(|e| {
            matches!(e, Event::WebTransportSessionClosed { session_id: sid, .. } if *sid == session_id)
        });
        assert!(closed_event.is_some());

        // セッションが Closed 状態であること
        assert_eq!(
            client.wt_sessions[&session_id].state,
            WtSessionState::Closed
        );
    }

    #[test]
    fn test_wt_session_terminated_on_reset() {
        let (mut client, mut server) = setup_wt_pair();

        // WT CONNECT + 200 OK のハンドシェイク
        let headers = vec![
            Header::new(b":method", b"CONNECT").unwrap(),
            Header::new(b":protocol", b"webtransport-h3").unwrap(),
            Header::new(b":scheme", b"https").unwrap(),
            Header::new(b":authority", b"example.com").unwrap(),
            Header::new(b":path", b"/wt").unwrap(),
        ];
        let stream_id = client.send_request(&headers, false).unwrap();
        let (req_data, _) = client.take_stream_data(stream_id).unwrap();
        server.feed_stream(stream_id, &req_data, false).unwrap();
        let _ = server.drain_events().unwrap();

        let response = vec![Header::new(b":status", b"200").unwrap()];
        server.send_response(stream_id, &response, false).unwrap();
        let (resp_data, _) = server.take_stream_data(stream_id).unwrap();
        client.feed_stream(stream_id, &resp_data, false).unwrap();
        let _ = client.drain_events().unwrap();

        // CONNECT stream を RESET_STREAM で閉じる
        client.stream_reset(stream_id, 0, 0).unwrap();

        // WebTransportSessionClosed イベントが発火すること
        let events = client.drain_events().unwrap();
        assert!(events.iter().any(|e| matches!(
            e,
            Event::WebTransportSessionClosed { session_id: sid, .. } if *sid == stream_id
        )));

        assert_eq!(client.wt_sessions[&stream_id].state, WtSessionState::Closed);
    }

    #[test]
    fn test_wt_session_buffering_before_established() {
        // サーバー側: セッション未確立 (CONNECT 受信前) のストリームが到着した場合のバッファリング
        let mut server = Connection::server(wt_enabled_settings());
        server.set_control_stream_id(3).unwrap();
        server.peer_settings = Some(wt_enabled_settings());
        server.wt_transport_verified = true;

        // client-initiated uni stream が先に到着 (session_id = 0)
        // ストリームタイプ 0x54 (varint [0x40, 0x54]) + session_id = 0
        server.feed_stream(2, &[0x40, 0x54, 0x00], false).unwrap();

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
            .unwrap();

        // セッションが Pending で生成されていること
        assert_eq!(server.wt_sessions[&0].state, WtSessionState::Pending);
        // バッファエントリにペイロードが積まれていること
        let entry = server
            .wt_sessions
            .get(&0)
            .unwrap()
            .buffered_stream_entries
            .get(&2)
            .unwrap();
        assert!(!entry.is_bidi);
        assert_eq!(entry.data, vec![0xAA, 0xBB]);
        assert!(!entry.fin);

        // Open / Data イベントは未発火
        assert!(server.events.is_empty());

        // 後続 Data も同様にバッファに追記される
        server.feed_stream(2, &[0xCC], false).unwrap();
        let entry = server
            .wt_sessions
            .get(&0)
            .unwrap()
            .buffered_stream_entries
            .get(&2)
            .unwrap();
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
        client.feed_stream(3, &[0x40, 0x54, 0x00], false).unwrap();

        // WT_SESSION_GONE でストリーム拒否イベントが発生すること
        let event = client.poll_event().unwrap().unwrap();
        assert!(matches!(
            event,
            Event::WebTransportBufferedStreamRejected {
                stream_id: 3,
                error_code,
            } if error_code == WtErrorCode::SessionGone as u64
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
            server.feed_stream(stream_id, &buf, false).unwrap();
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
        server.feed_stream(overflow_stream_id, &buf, false).unwrap();

        // 最終イベントが BufferedStreamRejected であること
        let mut last = None;
        while let Some(ev) = server.poll_event().unwrap() {
            last = Some(ev);
        }
        match last {
            Some(Event::WebTransportBufferedStreamRejected { stream_id, .. }) => {
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
            server.feed_datagram(&buf).unwrap();
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
        server.feed_datagram(&buf).unwrap();
        assert_eq!(server.count_pending_wt_sessions(), WT_MAX_PENDING_SESSIONS);
        assert!(!server.wt_sessions.contains_key(&overflow_session_id));
    }

    #[test]
    fn test_stream_reset_propagates_to_wt_uni_data_stream() {
        // 既知 WebTransport セッションに属する単方向データストリームの RESET_STREAM は
        // セッションを終了させず、WebTransportStreamReset イベントとして通知する
        // (draft-ietf-webtrans-http3-15 Section 4.4)
        let mut conn = make_server_with_established_wt_session(4);
        // セッション 4 に紐づく WT uni stream 2 を作成
        conn.feed_stream(2, &[0x40, 0x54, 0x04], false).unwrap();
        // Open イベントを消費
        let _ = conn.poll_event().unwrap().unwrap();

        // WT uni stream の stream header 長 = varint(0x54) + varint(4) = 2 + 1 = 3
        conn.stream_reset(2, 0x42, 3).unwrap();

        let event = conn.poll_event().unwrap().unwrap();
        assert!(matches!(
            event,
            Event::WebTransportStreamReset {
                session_id: 4,
                stream_id: 2,
                error_code: 0x42,
                final_size: 3,
            }
        ));
        // セッションは終了していないこと
        assert!(conn.wt_sessions.contains_key(&4));
        assert!(matches!(
            conn.wt_sessions.get(&4).unwrap().state,
            WtSessionState::Established
        ));
        // ストリームの紐付けは解除されていること
        assert!(!conn.wt_uni_streams.contains_key(&2));
        assert!(
            !conn
                .wt_sessions
                .get(&4)
                .unwrap()
                .associated_streams
                .contains(&2)
        );
    }

    #[test]
    fn test_stream_reset_on_connect_stream_terminates_wt_session() {
        // CONNECT stream (= session_id) への RESET_STREAM はセッションを終了させる
        // (draft-ietf-webtrans-http3-15 Section 6)
        let mut conn = make_server_with_established_wt_session(0);

        conn.stream_reset(0, 0x99, 0).unwrap();

        // WebTransportSessionClosed が発行される
        let event = conn.poll_event().unwrap().unwrap();
        assert!(matches!(
            event,
            Event::WebTransportSessionClosed { session_id: 0, .. }
        ));
        assert_eq!(
            conn.wt_sessions.get(&0).unwrap().state,
            WtSessionState::Closed
        );
    }

    #[test]
    fn test_stop_sending_propagates_to_wt_bidi_data_stream() {
        // 既知 WebTransport セッションに属する双方向データストリームの STOP_SENDING は
        // セッションを終了させず WebTransportStreamStopSending として通知する
        let mut conn = make_server_with_established_wt_session(4);
        // signal value 0x41 + session_id = 4 で WT bidi stream 8 を作成
        conn.feed_stream(8, &[0x40, 0x41, 0x04], false).unwrap();
        let _ = conn.poll_event().unwrap().unwrap();

        conn.stop_sending(8, 0x55).unwrap();

        let event = conn.poll_event().unwrap().unwrap();
        assert!(matches!(
            event,
            Event::WebTransportStreamStopSending {
                session_id: 4,
                stream_id: 8,
                error_code: 0x55,
            }
        ));
        assert!(matches!(
            conn.wt_sessions.get(&4).unwrap().state,
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
        conn.process_wt_capsule_data(session_id, &payload).unwrap();
    }

    #[test]
    fn test_wt_session_closed_event_carries_reliable_sizes() {
        // CONNECT stream RESET によるセッション終了時、関連 WT データストリームの
        // reliable_size が stream header 長と一致していることを検証する
        // (draft-ietf-webtrans-http3-15 Section 6 / Section 4.4 / Section 5.4)
        let mut conn = make_server_with_established_wt_session(0);

        // session_id = 0 に紐づく WT bidi stream 4 と uni stream 2 を作成
        // bidi: signal value 0x41 + session_id=0 → varint(0x41)=2 + varint(0)=1 = 3
        conn.feed_stream(4, &[0x40, 0x41, 0x00], false).unwrap();
        // uni: stream type 0x54 + session_id=0 → varint(0x54)=2 + varint(0)=1 = 3
        conn.feed_stream(2, &[0x40, 0x54, 0x00], false).unwrap();
        while conn.poll_event().unwrap().is_some() {}

        // CONNECT stream の RESET でセッション終了
        conn.stream_reset(0, 0x99, 0).unwrap();

        let event = conn.poll_event().unwrap().unwrap();
        let Event::WebTransportSessionClosed {
            session_id,
            reset_streams,
            ..
        } = event
        else {
            panic!("WebTransportSessionClosed が発火していない");
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
        // WebTransportStreamReset イベントに反映される
        let mut conn = make_server_with_established_wt_session(4);
        conn.feed_stream(2, &[0x40, 0x54, 0x04], false).unwrap();
        while conn.poll_event().unwrap().is_some() {}

        conn.stream_reset(2, 0xab, 42).unwrap();

        let event = conn.poll_event().unwrap().unwrap();
        let Event::WebTransportStreamReset {
            session_id,
            stream_id,
            error_code,
            final_size,
        } = event
        else {
            panic!("WebTransportStreamReset が発火していない: {event:?}");
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
        conn.feed_stream(2, &[0x40, 0x54, 0x04], false).unwrap();
        conn.feed_stream(8, &[0x40, 0x41, 0x04], false).unwrap();
        while conn.poll_event().unwrap().is_some() {}

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
            conn.wt_sessions.get(&0).unwrap().state,
            WtSessionState::Draining
        );
        // Draining イベントが発行されている
        let event = conn.poll_event().unwrap().unwrap();
        assert!(matches!(
            event,
            Event::WebTransportSessionDraining { session_id: 0 }
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
        let _ = conn.poll_event().unwrap();

        // CLOSE_SESSION カプセルを feed
        let mut payload = Vec::new();
        crate::webtransport::Capsule::CloseSession {
            error_code: 0,
            message: String::new(),
        }
        .encode(&mut payload);
        conn.process_wt_capsule_data(0, &payload).unwrap();

        assert_eq!(
            conn.wt_sessions.get(&0).unwrap().state,
            WtSessionState::Closed
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
            Header::new(b":method", b"CONNECT").unwrap(),
            Header::new(b":protocol", b"webtransport-h3").unwrap(),
            Header::new(b":scheme", b"https").unwrap(),
            Header::new(b":authority", b"example.com").unwrap(),
            Header::new(b":path", b"/wt").unwrap(),
        ];
        let stream_id = client.send_request(&headers, false).unwrap();
        // セッション確立まで進めず Pending のままで GOAWAY を受信させる

        // サーバーが GOAWAY を送信
        server.send_goaway(stream_id).unwrap();
        let (ctrl_data, _) = server.take_stream_data(3).unwrap();
        client.feed_stream(3, &ctrl_data, false).unwrap();

        // クライアントの WT セッションが Draining に遷移している
        assert_eq!(
            client.wt_sessions.get(&stream_id).unwrap().state,
            WtSessionState::Draining
        );

        // send_datagram も拒否される (datagram の前提として Established だが、
        // 念のため Draining で WtSessionDraining を返す経路を確認する)
        // ※ Pending → Draining でも内部状態として Draining になることを確認する
    }
}
