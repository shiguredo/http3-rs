//! WebTransport Connection 層の型定義 connection/mod.rs からの分離
//!
//! `Connection` 内で WebTransport セッションのライフサイクルと
//! 関連ストリームを追跡するための型を定義する。
//! (draft-ietf-webtrans-http3-16 Section 3, 4.6, 6)

use std::collections::{HashMap, HashSet};

use crate::webtransport::flow_control::{
    DataFlowControl, DirectionalStreamFlowControl, FlowControlLimits, FlowControlState,
    SendBlockedState,
};

/// `Connection::associate_or_buffer_stream` の結果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AssocOutcome {
    /// 既存 Established セッションに即時関連付けた
    Established,
    /// Pending セッションにバッファリングした (確立時にイベント発火)
    Buffered,
    /// バッファ上限超過 (WT_BUFFERED_STREAM_REJECTED 相当)
    BufferOverflow,
}

/// サーバー側で許容する Pending WebTransport セッション数の上限
///
/// クライアントは未知の `session_id` で先行ストリーム / データグラムを送ってくることが
/// あるが、`session_id` が一意であるたびに新しい Pending セッションを生成すると、
/// 攻撃者が一意な `session_id` を量産するだけで Pending セッションを無限増殖させられる。
/// これを防ぐため接続単位で Pending セッション数に上限を設ける。
/// (draft-ietf-webtrans-http3-16 Section 4.6 / RFC 9297 Section 2.1 / nghttp3
///  lib/nghttp3_conn.c と整合)
pub(crate) const WT_MAX_PENDING_SESSIONS: usize = 16;

/// セッション確立前の先行ストリームごとに保持する受信ペイロードの上限 (バイト)
/// (draft-ietf-webtrans-http3-16 Section 4.6, DoS 対策)
pub(crate) const WT_MAX_BUFFERED_STREAM_BYTES: usize = 64 * 1024;

/// セッション確立前の先行 WebTransport ストリームごとに保持する受信状態
///
/// (draft-ietf-webtrans-http3-16 Section 4.6)
#[derive(Debug)]
pub(crate) struct BufferedStreamEntry {
    /// 双方向ストリームかどうか
    pub(crate) is_bidi: bool,
    /// 受信済みペイロード (Open 後 〜 FIN まで)
    pub(crate) data: Vec<u8>,
    /// FIN を受信済みかどうか
    pub(crate) fin: bool,
}

impl BufferedStreamEntry {
    pub(crate) fn new(is_bidi: bool) -> Self {
        Self {
            is_bidi,
            data: Vec::new(),
            fin: false,
        }
    }
}

/// WebTransport セッションの Connection 層での状態
///
/// `Connection` 内でセッションのライフサイクル・関連ストリーム・
/// 送受信双方のフロー制御を追跡する。
/// (draft-ietf-webtrans-http3-16 Section 3, 4.6, 5.6, 6)
#[derive(Debug)]
pub(crate) struct WtSession {
    /// セッション状態
    pub(crate) state: WtSessionState,
    /// セッションに関連する全ストリーム ID (uni + bidi)
    pub(crate) associated_streams: HashSet<u64>,
    /// セッション確立前のバッファリングされたストリーム (Section 4.6)
    ///
    /// `buffered_streams` は順序保持のための stream_id ベクタ。
    /// `buffered_stream_entries` は同じ stream_id をキーに受信ペイロード/FIN を保持する。
    /// (draft-ietf-webtrans-http3-16 Section 4.6 — Open / Data / End を確立後に
    ///  順序を保って一括発火するために必要)
    pub(crate) buffered_streams: Vec<u64>,
    pub(crate) buffered_stream_entries: HashMap<u64, BufferedStreamEntry>,
    /// セッション確立前のバッファリングされたデータグラム (Section 4.6)
    pub(crate) buffered_datagrams: Vec<Vec<u8>>,
    /// CONNECT ストリーム上の Capsule デコードバッファ (Section 5.6)
    ///
    /// Capsule が複数の DATA フレームにまたがる場合のバッファリング用。
    ///
    /// `capsule_buf_start` はデコード済みで未切り詰めの先頭位置。毎回 `drain` すると
    /// O(n^2) になるため読み出し位置を進め、半分割以上を消費した時点で切り詰める。
    pub(crate) capsule_buf: Vec<u8>,
    /// `capsule_buf` のうちデコード済みの先頭バイト数
    pub(crate) capsule_buf_start: usize,
    /// リクエスト時の WT-Available-Protocols (Section 3.3)
    ///
    /// クライアントが送信した WT-Available-Protocols の値を保持する。
    /// レスポンス受信時に WT-Protocol を検証するために使用する。
    pub(crate) available_protocols: Vec<String>,
    /// フロー制御が有効かどうか (Section 5.1)
    ///
    /// 両端がフロー制御を宣言した場合のみ `true`。
    /// セッション確立時に `flow_control_enabled_with_peer` で決定される。
    pub(crate) flow_control_enabled: bool,
    /// 受信側ストリームフロー制御 (単方向)
    /// (draft-ietf-webtrans-http3-16 Section 5.6)
    /// フロー制御有効時にセッション確立時点で初期化される。
    pub(crate) recv_stream_fc_uni: Option<DirectionalStreamFlowControl>,
    /// 受信側ストリームフロー制御 (双方向)
    /// (draft-ietf-webtrans-http3-16 Section 5.6)
    pub(crate) recv_stream_fc_bidi: Option<DirectionalStreamFlowControl>,
    /// 受信側データフロー制御
    /// (draft-ietf-webtrans-http3-16 Section 5.4)
    pub(crate) recv_data_fc: Option<DataFlowControl>,
    /// ストリームごとの計上済み受信ボディ量 (ストリーム ID → ボディバイト数)
    ///
    /// RESET_STREAM 受信時に final_size から既計上分を引いて残差を求めるために
    /// 使う (二重計上防止。draft-ietf-webtrans-http3-16 Section 5.4)。
    /// ストリームの FIN / RESET / セッション終了時に掃除する。
    pub(crate) stream_received_data: HashMap<u64, u64>,
    /// ピアが広告した送信側リミット (WT_MAX_STREAMS / WT_MAX_DATA)
    ///
    /// ピアから受信したフロー制御カプセルで更新される。未受信の間は 0 として扱い、
    /// `remote_limits_known` が false の間は送信を許可しない
    /// (draft-ietf-webtrans-http3-16 Section 5.5)。
    pub(crate) remote_limits: FlowControlLimits,
    /// ピアから送信側リミットを受信済みかどうか
    ///
    /// カプセルの値は「増加しない場合は `WT_FLOW_CONTROL_ERROR`」なので
    /// (draft-ietf-webtrans-http3-16 Section 5.6.2, 5.6.4)、未受信と 0 を
    /// 区別する必要がある。
    pub(crate) remote_limits_known: bool,
    /// 送信側の計上状態 (開いたストリーム数 / 送信済みデータ量)
    ///
    /// `flow_control_enabled == false` でも開設・送信の可否判定に使うため、
    /// 常に保持する。
    pub(crate) flow_state: FlowControlState,
    /// 送信側のブロック通知の重複送信防止状態
    /// (draft-ietf-webtrans-http3-16 Section 5.6.2, 5.6.4)
    pub(crate) send_blocked: SendBlockedState,
    /// Connection 層で生成された送信待ちカプセル
    /// (WT_MAX_STREAMS, WT_MAX_DATA, WT_STREAMS_BLOCKED, WT_DATA_BLOCKED)
    ///
    /// セッション確立前に積まれた初期カプセルと、ピアの消費に応じて生成される
    /// 更新カプセルの両方を含む。アプリケーション層が
    /// `Connection::take_wt_flow_control_capsules()` で取り出して
    /// CONNECT ストリームへ送出する。
    pub(crate) pending_capsules: Vec<crate::webtransport::Capsule>,
}

/// WebTransport セッションの Connection 層での状態
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WtSessionState {
    /// CONNECT 送信/受信済みだがレスポンス未処理
    Pending,
    /// 確立済み (200 OK 受信)
    Established,
    /// グレースフルシャットダウン中
    /// (draft-ietf-webtrans-http3-16 Section 4.7)
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
    pub(crate) fn new() -> Self {
        Self {
            state: WtSessionState::Pending,
            associated_streams: HashSet::new(),
            buffered_streams: Vec::new(),
            buffered_stream_entries: HashMap::new(),
            buffered_datagrams: Vec::new(),
            capsule_buf: Vec::new(),
            capsule_buf_start: 0,
            available_protocols: Vec::new(),
            flow_control_enabled: false,
            recv_stream_fc_uni: None,
            recv_stream_fc_bidi: None,
            recv_data_fc: None,
            stream_received_data: HashMap::new(),
            remote_limits: FlowControlLimits::new(),
            remote_limits_known: false,
            flow_state: FlowControlState::new(),
            send_blocked: SendBlockedState::default(),
            pending_capsules: Vec::new(),
        }
    }

    /// フロー制御を初期化する (セッション確立時に呼ぶ)
    ///
    /// ローカルの SETTINGS から初期リミットを読み取り、受信側フロー制御を設定する。
    /// (draft-ietf-webtrans-http3-16 Section 5.5, 5.6)
    pub(crate) fn initialize_flow_control(
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
    pub(crate) fn check_received_stream(&self, bidirectional: bool) -> bool {
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
    pub(crate) fn add_received_stream(&mut self, bidirectional: bool) {
        if bidirectional {
            if let Some(fc) = &mut self.recv_stream_fc_bidi {
                fc.on_stream_received();
            }
        } else if let Some(fc) = &mut self.recv_stream_fc_uni {
            fc.on_stream_received();
        }
    }

    /// 受信データのフロー制御チェック
    ///
    /// `false` の場合は WT_FLOW_CONTROL_ERROR で終了すべき。
    pub(crate) fn check_received_data(&self, bytes: u64) -> bool {
        if !self.flow_control_enabled {
            return true;
        }
        self.recv_data_fc
            .as_ref()
            .is_none_or(|fc| fc.check_received(bytes))
    }

    /// 受信データ量を加算
    pub(crate) fn add_received_data(&mut self, bytes: u64) {
        if let Some(fc) = &mut self.recv_data_fc {
            fc.on_data_received(bytes);
        }
    }

    /// ピアが開いたストリームが完全に閉じたことを通知
    ///
    /// 必要に応じて WT_MAX_STREAMS カプセルを `pending_capsules` に追加する。
    pub(crate) fn on_remote_stream_closed(&mut self, bidirectional: bool) {
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
    pub(crate) fn on_data_consumed(&mut self, bytes: u64) {
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
    pub(crate) fn take_pending_capsules(&mut self) -> Vec<crate::webtransport::Capsule> {
        std::mem::take(&mut self.pending_capsules)
    }

    // ------------------------------------------------------------------
    // 送信側フロー制御 (draft-ietf-webtrans-http3-16 Section 5.6)
    //
    // ピアが広告した上限を `remote_limits` に保持し、ストリーム開設とデータ送信の
    // 前に検証する。上限に達した場合は WT_STREAMS_BLOCKED / WT_DATA_BLOCKED を
    // 一度だけ生成する。送信側リミットが未受信 (`remote_limits == None`) の場合は
    // フロー制御が有効でも送信を許可しない (初期値 0 と同じ扱い。draft-16 §5.5)。
    // ------------------------------------------------------------------

    /// ピアの送信側リミットを未受信の状態に戻す
    ///
    /// セッション確立時に呼ぶ。以後 `WT_MAX_STREAMS` / `WT_MAX_DATA` カプセルの
    /// 受信で上限が確定する。未確定の間は送信を許可しない (初期値 0 と同じ扱い。
    /// draft-ietf-webtrans-http3-16 Section 5.5)。
    pub(crate) fn clear_remote_limits(&mut self) {
        self.remote_limits = FlowControlLimits::new();
        self.remote_limits_known = false;
        self.flow_state = FlowControlState::new();
        self.send_blocked = SendBlockedState::default();
    }

    /// ピアが広告した送信側リミットを取得する (未受信の場合は `None`)
    pub(crate) fn remote_limits(&self) -> Option<&FlowControlLimits> {
        self.remote_limits_known.then_some(&self.remote_limits)
    }

    /// 送信側の計上状態を取得する
    pub(crate) fn flow_state(&self) -> &FlowControlState {
        &self.flow_state
    }

    /// 単方向ストリームを開設できるかどうか
    ///
    /// フロー制御が無効な場合は常に true。有効な場合はピアが広告した上限と
    /// 開設済み本数を比較する。
    pub(crate) fn can_create_uni_stream(&self) -> bool {
        if !self.flow_control_enabled {
            return true;
        }
        self.remote_limits_known
            && self.flow_state.streams_uni_opened < self.remote_limits.max_streams_uni
    }

    /// 双方向ストリームを開設できるかどうか
    pub(crate) fn can_create_bidi_stream(&self) -> bool {
        if !self.flow_control_enabled {
            return true;
        }
        self.remote_limits_known
            && self.flow_state.streams_bidi_opened < self.remote_limits.max_streams_bidi
    }

    /// データを送信できるかどうか
    pub(crate) fn can_send_data(&self, bytes: u64) -> bool {
        if !self.flow_control_enabled {
            return true;
        }
        self.remote_limits_known
            && self
                .remote_limits
                .max_data
                .saturating_sub(self.flow_state.data_sent)
                >= bytes
    }

    /// ストリーム開設を試行する
    ///
    /// 開設可能なら計上して `true` を返す。上限に達している場合は
    /// WT_STREAMS_BLOCKED を一度だけ生成して `false` を返す。
    /// (draft-ietf-webtrans-http3-16 Section 5.6.2)
    pub(crate) fn try_open_stream(&mut self, bidirectional: bool) -> bool {
        if !self.flow_control_enabled {
            // フロー制御無効時は上限なし (draft-ietf-webtrans-http3-16 Section 5.1)
            if bidirectional {
                self.flow_state.streams_bidi_opened =
                    self.flow_state.streams_bidi_opened.saturating_add(1);
            } else {
                self.flow_state.streams_uni_opened =
                    self.flow_state.streams_uni_opened.saturating_add(1);
            }
            return true;
        }

        // 送信側リミット未受信は初期値 0 と同じ扱い (draft-ietf-webtrans-http3-16 Section 5.5)
        if !self.remote_limits_known {
            return false;
        }

        let (opened, limit, last_blocked) = if bidirectional {
            (
                self.flow_state.streams_bidi_opened,
                self.remote_limits.max_streams_bidi,
                self.send_blocked.last_streams_blocked_bidi,
            )
        } else {
            (
                self.flow_state.streams_uni_opened,
                self.remote_limits.max_streams_uni,
                self.send_blocked.last_streams_blocked_uni,
            )
        };

        if opened < limit {
            if bidirectional {
                self.flow_state.streams_bidi_opened = opened.saturating_add(1);
            } else {
                self.flow_state.streams_uni_opened = opened.saturating_add(1);
            }
            return true;
        }

        // ブロック: 同じ maximum に対して重複送信しない
        if last_blocked != Some(limit) {
            if bidirectional {
                self.send_blocked.last_streams_blocked_bidi = Some(limit);
            } else {
                self.send_blocked.last_streams_blocked_uni = Some(limit);
            }
            self.pending_capsules
                .push(crate::webtransport::Capsule::StreamsBlocked {
                    bidirectional,
                    maximum: limit,
                });
        }
        false
    }

    /// データ送信を試行する
    ///
    /// 送信可能なら計上して `true` を返す。上限に達している場合は
    /// WT_DATA_BLOCKED を一度だけ生成して `false` を返す。
    /// (draft-ietf-webtrans-http3-16 Section 5.6.4)
    pub(crate) fn try_send_data(&mut self, bytes: u64) -> bool {
        if !self.flow_control_enabled {
            self.flow_state.data_sent = self.flow_state.data_sent.saturating_add(bytes);
            return true;
        }

        if !self.remote_limits_known {
            return false;
        }
        let limit = self.remote_limits.max_data;

        if limit.saturating_sub(self.flow_state.data_sent) >= bytes {
            self.flow_state.data_sent = self.flow_state.data_sent.saturating_add(bytes);
            return true;
        }

        if self.send_blocked.last_data_blocked != Some(limit) {
            self.send_blocked.last_data_blocked = Some(limit);
            self.pending_capsules
                .push(crate::webtransport::Capsule::DataBlocked { maximum: limit });
        }
        false
    }

    /// 受信した WT_MAX_STREAMS を適用する
    ///
    /// 現在値から増加しない場合は `WT_FLOW_CONTROL_ERROR` として拒否する
    /// (draft-ietf-webtrans-http3-16 Section 5.6.2: "does not increase")。
    /// 2^60 を超える値も同じエラーで拒否する。
    pub(crate) fn apply_max_streams(&mut self, bidirectional: bool, maximum: u64) -> bool {
        if maximum > crate::webtransport::MAX_STREAMS_LIMIT {
            return false;
        }
        // 最初の WT_MAX_STREAMS は上限を確定させる (未受信の間は 0 と同じ扱い)
        if !self.remote_limits_known {
            if bidirectional {
                self.remote_limits.max_streams_bidi = maximum;
            } else {
                self.remote_limits.max_streams_uni = maximum;
            }
            self.remote_limits_known = true;
            if bidirectional {
                self.send_blocked.last_streams_blocked_bidi = None;
            } else {
                self.send_blocked.last_streams_blocked_uni = None;
            }
            return true;
        }
        let current = if bidirectional {
            &mut self.remote_limits.max_streams_bidi
        } else {
            &mut self.remote_limits.max_streams_uni
        };
        if maximum <= *current {
            return false;
        }
        *current = maximum;
        // 新しい上限を受け取ったので BLOCKED 状態をリセットする
        if bidirectional {
            self.send_blocked.last_streams_blocked_bidi = None;
        } else {
            self.send_blocked.last_streams_blocked_uni = None;
        }
        true
    }

    /// 受信した WT_MAX_DATA を適用する
    ///
    /// 現在値から増加しない場合は `WT_FLOW_CONTROL_ERROR` として拒否する
    /// (draft-ietf-webtrans-http3-16 Section 5.6.4: "does not increase")。
    pub(crate) fn apply_max_data(&mut self, maximum: u64) -> bool {
        // 最初の WT_MAX_DATA は上限を確定させる (未受信の間は 0 と同じ扱い)
        if !self.remote_limits_known {
            self.remote_limits.max_data = maximum;
            self.remote_limits_known = true;
            self.send_blocked.last_data_blocked = None;
            return true;
        }
        if maximum <= self.remote_limits.max_data {
            return false;
        }
        self.remote_limits.max_data = maximum;
        self.send_blocked.last_data_blocked = None;
        true
    }

    /// ストリームの計上済み受信ボディ量を加算する
    ///
    /// `add_received_data_and_track` からのみ呼ばれる (計上と追跡の対を
    /// 構造的に保証するため)。
    fn add_stream_received_data(&mut self, stream_id: u64, bytes: u64) {
        let entry = self.stream_received_data.entry(stream_id).or_insert(0);
        *entry = entry.saturating_add(bytes);
    }

    /// データ FC の計上と per-stream 追跡を同時に行う
    ///
    /// 受信経路では必ず両方を対で呼ぶ (RESET 時の残差計算が
    /// 正しくなるための不変条件)。
    pub(crate) fn add_received_data_and_track(&mut self, stream_id: u64, bytes: u64) {
        self.add_received_data(bytes);
        self.add_stream_received_data(stream_id, bytes);
    }

    /// ストリームの計上済み受信ボディ量を取得する
    pub(crate) fn get_stream_received_data(&self, stream_id: u64) -> u64 {
        self.stream_received_data
            .get(&stream_id)
            .copied()
            .unwrap_or(0)
    }

    /// ストリームの計上済み受信ボディ量の追跡を掃除する
    ///
    /// ストリームの FIN / RESET 時に呼ぶ (無制限の成長を防ぐ)。
    pub(crate) fn remove_stream_received_data(&mut self, stream_id: u64) {
        self.stream_received_data.remove(&stream_id);
    }

    /// ストリームをセッションに関連付ける
    pub(crate) fn associate_stream(&mut self, stream_id: u64) {
        self.associated_streams.insert(stream_id);
    }

    /// ストリームの関連付けを解除する
    pub(crate) fn disassociate_stream(&mut self, stream_id: u64) {
        self.associated_streams.remove(&stream_id);
    }

    /// 受信ストリームをバッファリング (Section 4.6)
    ///
    /// バッファ上限を超えた場合は `false` を返す。
    /// 呼び出し元は `WT_BUFFERED_STREAM_REJECTED` で RESET_STREAM を送信すること。
    pub(crate) fn buffer_stream(&mut self, stream_id: u64, is_bidi: bool) -> bool {
        if self.buffered_streams.len() >= crate::webtransport::flow_control::MAX_BUFFERED_STREAMS {
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
    pub(crate) fn append_buffered_stream_data(&mut self, stream_id: u64, data: &[u8]) -> bool {
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
    pub(crate) fn mark_buffered_stream_fin(&mut self, stream_id: u64) {
        if let Some(entry) = self.buffered_stream_entries.get_mut(&stream_id) {
            entry.fin = true;
        }
    }

    /// バッファリングされたストリーム ID を順序付きで取り出す (セッション確立後に呼び出す)
    pub(crate) fn take_buffered_streams(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.buffered_streams)
    }

    /// バッファリングされたストリーム受信状態を取り出す (セッション確立後に呼び出す)
    pub(crate) fn take_buffered_stream_entry(
        &mut self,
        stream_id: u64,
    ) -> Option<BufferedStreamEntry> {
        self.buffered_stream_entries.remove(&stream_id)
    }

    /// バッファリングされたストリームのエントリを除去する (RESET_STREAM / セッション終了時)
    pub(crate) fn remove_buffered_stream(&mut self, stream_id: u64) -> bool {
        self.buffered_streams.retain(|&id| id != stream_id);
        self.buffered_stream_entries.remove(&stream_id).is_some()
    }

    /// バッファリングされたストリームのエントリを復元する (deliver_buffered_streams の中断時)
    pub(crate) fn restore_buffered_stream(&mut self, stream_id: u64, entry: BufferedStreamEntry) {
        self.buffered_streams.push(stream_id);
        self.buffered_stream_entries.insert(stream_id, entry);
    }

    /// 受信データグラムをバッファリング (Section 4.6)
    ///
    /// バッファ上限を超えた場合は `false` を返す。
    /// 呼び出し元はデータグラムを破棄すること。
    pub(crate) fn buffer_datagram(&mut self, data: Vec<u8>) -> bool {
        if self.buffered_datagrams.len()
            >= crate::webtransport::flow_control::MAX_BUFFERED_DATAGRAMS
        {
            return false;
        }
        self.buffered_datagrams.push(data);
        true
    }

    /// バッファリングされたデータグラムを取り出す (セッション確立後に呼び出す)
    pub(crate) fn take_buffered_datagrams(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.buffered_datagrams)
    }
}
