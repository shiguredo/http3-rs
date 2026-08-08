//! WebTransport セッション管理 (0077: connection/mod.rs から分離)
//!
//! WebTransport の能力ネゴシエーション、セッションライフサイクル管理、
//! フロー制御判定を担う `Connection` メソッド群。
//! (draft-ietf-webtrans-http3-15 Section 3, 4.6, 4.7, 5, 6, 7.1)

use crate::error::{Error, ErrorCode, WtSetupError};
use crate::event::{Event, WebTransportEvent, WtStreamReset};
use crate::qpack::Header;
use crate::webtransport::DraftVersion;
use crate::webtransport::error::ErrorCode as WtErrorCode;

use super::wt_types::{AssocOutcome, WT_MAX_PENDING_SESSIONS, WtSession, WtSessionState};
use super::{Connection, Role};

/// RFC 9297 Section 3.2 の Capsule Protocol 併用禁止ヘッダー (Content-Length / Content-Type)
///
/// Capsule Protocol を使用するメッセージに付与してはならない (MUST NOT)。
/// Transfer-Encoding は接続固有ヘッダーとして全リクエストで既に拒否される。
fn has_forbidden_capsule_headers(headers: &[Header]) -> bool {
    headers
        .iter()
        .any(|h| h.name() == b"content-length" || h.name() == b"content-type")
}

/// RFC 9297 Section 3.2 の Capsule Protocol 併用禁止ステータス (204 / 205 / 206)
///
/// Capsule Protocol を使用するレスポンスに付与してはならない (MUST NOT)。
fn is_forbidden_capsule_status(headers: &[Header]) -> bool {
    headers.iter().any(|h| {
        h.name() == b":status"
            && (h.value() == b"204" || h.value() == b"205" || h.value() == b"206")
    })
}

impl Connection {
    pub(crate) fn peer_requires_initial_wt_capsules(&self) -> bool {
        self.peer_settings
            .as_ref()
            .and_then(|s| s.wt_settings.as_ref())
            .is_some_and(|wt| wt.requires_initial_capsule_flow_control_compat())
    }

    /// WebTransport transport parameter の検証結果を注入する
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

    /// バッファリングされたストリーム/データグラムを配送する (0110: 共通化)
    ///
    /// `emit_header_events` と `send_response` の重複コードを共通化する。
    /// フロー制御違反があった場合は `true` を返す。
    /// (draft-ietf-webtrans-http3-15 Section 4.6, 5.4, 5.6)
    pub(crate) fn deliver_buffered_streams(&mut self, session_id: u64) -> bool {
        let Some(session) = self.wt_sessions.get_mut(&session_id) else {
            return false;
        };

        let buffered = session.take_buffered_streams();
        let mut buffered_events: Vec<Event> = Vec::new();
        let mut closed_buffered: Vec<(u64, bool)> = Vec::new();
        let mut fc_violation = false;

        for &buffered_stream_id in &buffered {
            let Some(entry) = session.take_buffered_stream_entry(buffered_stream_id) else {
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
                Event::WebTransport(WebTransportEvent::BidiStreamOpen {
                    stream_id: buffered_stream_id,
                    session_id,
                })
            } else {
                Event::WebTransport(WebTransportEvent::UniStreamOpen {
                    stream_id: buffered_stream_id,
                    session_id,
                })
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
                    Event::WebTransport(WebTransportEvent::BidiStreamData {
                        stream_id: buffered_stream_id,
                        data: entry_data,
                    })
                } else {
                    Event::WebTransport(WebTransportEvent::UniStreamData {
                        stream_id: buffered_stream_id,
                        data: entry_data,
                    })
                });
            }
            // End
            if entry_fin {
                session.on_remote_stream_closed(is_bidi);
                closed_buffered.push((buffered_stream_id, is_bidi));
                buffered_events.push(if is_bidi {
                    Event::WebTransport(WebTransportEvent::BidiStreamEnd {
                        stream_id: buffered_stream_id,
                    })
                } else {
                    Event::WebTransport(WebTransportEvent::UniStreamEnd {
                        stream_id: buffered_stream_id,
                    })
                });
            }
        }

        // イベントを配送
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

        // バッファリングされていたデータグラムを配送 (Section 4.6)
        if !fc_violation {
            let buffered_datagrams = session.take_buffered_datagrams();
            for payload in buffered_datagrams {
                self.events
                    .push_back(Event::WebTransport(WebTransportEvent::Datagram {
                        session_id,
                        payload,
                    }));
            }
        }

        fc_violation
    }

    /// WebTransport SETTINGS を受信していない場合は `None`。
    /// (draft-ietf-webtrans-http3-02/07/14/15 Section 3.1)
    pub(crate) fn peer_wt_draft_version(&self) -> Option<DraftVersion> {
        self.peer_settings
            .as_ref()
            .and_then(|s| s.wt_settings.as_ref())
            .and_then(|wt| wt.detect_draft_pattern())
    }

    /// ローカルとピアが共に広告している中で最も新しい WebTransport ドラフトを返す
    ///
    /// バージョンネゴシエーションは「両エンドポイントが広告する集合の交差から
    /// 最も新しいものを選ぶ」(draft-ietf-webtrans-http3-15 Section 7.1)。
    /// 将来のドラフトで変更される可能性がある
    pub(crate) fn negotiated_wt_draft_version(&self) -> Option<DraftVersion> {
        self.mutually_advertised_wt_drafts().into_iter().next()
    }

    /// ローカルとピアが共に広告している WebTransport ドラフトを新しい順に返す
    ///
    /// (draft-ietf-webtrans-http3-15 Section 7.1)
    /// 将来のドラフトで変更される可能性がある
    pub(crate) fn mutually_advertised_wt_drafts(&self) -> Vec<DraftVersion> {
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
    /// (draft-ietf-webtrans-http3-02/07/14/15 Section 3.1)
    pub(crate) fn is_wt_fully_negotiated(&self) -> bool {
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
        // (nghttp3 lib/nghttp3_conn.c の TODO コメント参照)。
        let local_draft = self.local_settings.webtransport_draft_pattern();
        if matches!(local_draft, Some(DraftVersion::Draft15)) && !peer.is_webtransport_enabled() {
            return false;
        }
        true
    }

    /// WebTransport フロー制御が両端で有効かどうかを判定する
    ///
    /// (draft-ietf-webtrans-http3-15 Section 5.1)
    pub(crate) fn is_wt_flow_control_enabled(&self) -> bool {
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
    /// セッションエントリを `wt_sessions` から除去し、終了済みセッション ID を
    /// tombstone に記録する (draft-ietf-webtrans-http3-16 Section 6)。
    pub(crate) fn terminate_wt_session_with(
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
                reset_streams.push(WtStreamReset {
                    stream_id: sid,
                    reliable_size: 0,
                });
            }

            self.events
                .push_back(Event::WebTransport(WebTransportEvent::SessionClosed {
                    session_id,
                    reset_streams,
                    error_code,
                    close_error_code,
                    close_message,
                }));
        }

        // セッションエントリを除去し、終了済みセッション ID を tombstone に記録する
        // 終了後に届く DATA / FIN / RESET / 新規ストリーム / データグラムの拒否・破棄と
        // zombie Pending セッションの再生成防止に使う
        // (draft-ietf-webtrans-http3-16 Section 6)
        if self.wt_sessions.remove(&session_id).is_some() {
            self.closed_wt_sessions.insert(session_id);
        }
    }

    /// WebTransport セッションを WT_SESSION_GONE で終了する
    ///
    /// (draft-ietf-webtrans-http3-15 Section 6)
    pub(crate) fn terminate_wt_session(&mut self, session_id: u64) {
        self.terminate_wt_session_with(
            session_id,
            WtErrorCode::SessionGone as u64,
            0,
            String::new(),
        );
    }

    /// WebTransport ストリームをセッションに関連付ける、またはバッファリングする
    ///
    /// (draft-ietf-webtrans-http3-15 Section 4.6, 6)
    pub(crate) fn associate_or_buffer_stream(
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
                WtSessionState::Draining => Err(()),
                WtSessionState::Closed => Err(()),
            }
        } else {
            // クライアントは自身が開始していない session_id を拒否する
            // (draft-ietf-webtrans-http3-15 Section 4.6)
            if self.role == Role::Client {
                return Err(());
            }

            // 終了済みセッションへの新規ストリームは拒否する
            // (draft-ietf-webtrans-http3-16 Section 6: 終了済みセッションには
            //  新規ストリームを送ってはならない (MUST NOT open any new streams)。
            //  zombie Pending セッションの再生成を防ぐ)
            if self.closed_wt_sessions.contains(&session_id) {
                return Err(());
            }

            // サーバーが GOAWAY を送信済みの場合、その境界以降の session_id に対する
            // 新規 WebTransport セッションは受け入れない
            // (draft-ietf-webtrans-http3-15 Section 4.7)。
            if let Some(last_id) = self.last_sent_goaway_id
                && session_id >= last_id.get()
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
    pub(crate) fn count_pending_wt_sessions(&self) -> usize {
        self.wt_sessions
            .values()
            .filter(|s| s.state == WtSessionState::Pending)
            .count()
    }

    /// 現在 active な WebTransport セッション数を数える (Pending + Established + Draining)
    ///
    /// draft-ietf-webtrans-http3-15 Section 5.1 / 5.2 の
    /// 「フロー制御無効時は同時に 1 セッションまで」の判定に使用する。
    /// 将来のドラフトで定義が変更される可能性がある。
    pub(crate) fn count_active_wt_sessions(&self) -> usize {
        self.wt_sessions
            .values()
            .filter(|s| {
                s.state == WtSessionState::Pending
                    || s.state == WtSessionState::Established
                    || s.state == WtSessionState::Draining
            })
            .count()
    }

    /// WebTransport セッションのデータ消費を通知する
    ///
    /// (draft-ietf-webtrans-http3-15 Section 5.6)
    pub fn wt_data_consumed(&mut self, session_id: u64, bytes: u64) {
        if let Some(session) = self.wt_sessions.get_mut(&session_id) {
            session.on_data_consumed(bytes);
        }
    }

    /// WebTransport セッションの送信待ちカプセルを取り出す
    ///
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
    pub fn wt_session_flow_control_enabled(&self, session_id: u64) -> bool {
        self.wt_sessions
            .get(&session_id)
            .is_some_and(|s| s.flow_control_enabled)
    }

    /// RESET_STREAM 受信時の WebTransport 伝播処理 (0077 Phase 5: 混在関数抽出)
    ///
    /// (draft-ietf-webtrans-http3-15 Section 4.4 / Section 6)
    /// CONNECT stream の RESET_STREAM はセッション終了、データストリームの
    /// RESET_STREAM はセッションに通知する。非 WT ストリームは `false` を返す。
    pub(crate) fn handle_wt_stream_reset(
        &mut self,
        stream_id: u64,
        error_code: u64,
        final_size: u64,
    ) -> bool {
        if self.wt_sessions.contains_key(&stream_id) {
            self.terminate_wt_session(stream_id);
            return true;
        }
        // 終了済みセッションの CONNECT ストリームへの RESET_STREAM は静かに無視する
        // (RFC 9000 Section 4.4: RESET_STREAM 受信時はストリームの状態を破棄し、
        //  以降のデータを無視する)
        if self.closed_wt_sessions.contains(&stream_id) {
            return true;
        }
        if let Some(session_id) = self
            .wt_uni_streams
            .remove(&stream_id)
            .or_else(|| self.wt_bidi_streams.remove(&stream_id))
        {
            if let Some(session) = self.wt_sessions.get_mut(&session_id) {
                session.disassociate_stream(stream_id);
            }
            self.events
                .push_back(Event::WebTransport(WebTransportEvent::StreamReset {
                    session_id,
                    stream_id,
                    error_code,
                    final_size,
                }));
            return true;
        }
        false
    }

    /// STOP_SENDING 受信時の WebTransport 伝播処理 (0077 Phase 5: 混在関数抽出)
    ///
    /// (draft-ietf-webtrans-http3-15 Section 4.4 / Section 6)
    /// CONNECT stream の STOP_SENDING はセッション終了、データストリームの
    /// STOP_SENDING はセッションに通知する。非 WT ストリームは `false` を返す。
    pub(crate) fn handle_wt_stop_sending(&mut self, stream_id: u64, error_code: u64) -> bool {
        if self.wt_sessions.contains_key(&stream_id) {
            self.terminate_wt_session(stream_id);
            return true;
        }
        // 終了済みセッションの CONNECT ストリームへの STOP_SENDING は静かに無視する
        // (draft-ietf-webtrans-http3-16 Section 6)
        if self.closed_wt_sessions.contains(&stream_id) {
            return true;
        }
        if let Some(session_id) = self
            .wt_uni_streams
            .get(&stream_id)
            .copied()
            .or_else(|| self.wt_bidi_streams.get(&stream_id).copied())
        {
            self.events
                .push_back(Event::WebTransport(WebTransportEvent::StreamStopSending {
                    session_id,
                    stream_id,
                    error_code,
                }));
            return true;
        }
        false
    }

    /// WebTransport CONNECT リクエストの前提条件検証 (0077 Phase 5: 混在関数抽出)
    ///
    /// (draft-ietf-webtrans-http3-15 Section 3.1, 4.6)
    /// クライアントが WT CONNECT を送信する前に peer の WebTransport サポートを確認する。
    /// 非 WT CONNECT の場合は `Ok(())` を返す。
    pub(crate) fn validate_wt_connect_request(
        &self,
        headers: &[crate::qpack::Header],
    ) -> Result<(), Error> {
        if !super::is_webtransport_connect(headers) {
            return Ok(());
        }

        // RFC 9297 Section 3.2: Capsule Protocol を使用するメッセージに
        // Content-Length / Content-Type ヘッダーを付与してはならない (MUST NOT)。
        // 送信側も違反メッセージを生成しない
        if has_forbidden_capsule_headers(headers) {
            return Err(Error::StreamError(ErrorCode::MessageError));
        }

        let peer = self
            .peer_settings
            .as_ref()
            .ok_or(Error::WtSetup(WtSetupError::PeerSettingsNotReceived))?;
        if !peer.is_webtransport_enabled() {
            return Err(Error::WtSetup(WtSetupError::WebTransportNotEnabled));
        }
        let draft = self
            .peer_wt_draft_version()
            .ok_or(Error::WtSetup(WtSetupError::UnknownDraftVersion))?;
        if draft.requires_enable_connect_protocol() && peer.enable_connect_protocol != Some(true) {
            return Err(Error::WtSetup(WtSetupError::EnableConnectProtocolMissing));
        }
        if peer.h3_datagram != Some(true) {
            return Err(Error::WtSetup(WtSetupError::H3DatagramNotEnabled));
        }
        if !self.wt_transport_verified {
            return Err(Error::WtSetup(WtSetupError::TransportNotVerified));
        }
        if draft.requires_reset_stream_at() && !self.wt_reset_stream_at_supported {
            return Err(Error::WtSetup(WtSetupError::ResetStreamAtNotSupported));
        }
        let expected_proto = draft.protocol_value().as_bytes();
        let proto_ok = headers
            .iter()
            .any(|h| h.name() == b":protocol" && h.value() == expected_proto);
        if !proto_ok {
            return Err(Error::WtSetup(WtSetupError::ProtocolMismatch));
        }
        if !self.is_wt_flow_control_enabled() && self.count_active_wt_sessions() >= 1 {
            return Err(Error::StreamError(ErrorCode::RequestRejected));
        }

        Ok(())
    }

    /// サーバー側: WebTransport CONNECT リクエストの前提条件検証 (0077 Phase 5: 混在関数抽出)
    ///
    /// (draft-ietf-webtrans-http3-15 Section 3.1, 7.1)
    /// 非 WT CONNECT の場合は `Ok(())` を返す。
    pub(crate) fn validate_wt_connect_request_server(
        &self,
        stream_id: u64,
        headers: &[crate::qpack::Header],
    ) -> Result<(), Error> {
        if self.role != Role::Server || !super::is_webtransport_connect(headers) {
            return Ok(());
        }

        // RFC 9297 Section 3.2: Capsule Protocol を使用するメッセージに
        // Content-Length / Content-Type ヘッダーを付与してはならない (MUST NOT)。
        // 違反は malformed として H3_MESSAGE_ERROR で拒否する。
        // (Transfer-Encoding は接続固有ヘッダーとして全リクエストで既に拒否される)
        if has_forbidden_capsule_headers(headers) {
            return Err(Error::StreamError(ErrorCode::MessageError));
        }

        if !self.local_settings.is_webtransport_enabled() {
            return Err(Error::StreamError(ErrorCode::MessageError));
        }
        let peer = self
            .peer_settings
            .as_ref()
            .ok_or(Error::StreamError(ErrorCode::MessageError))?;
        let local_draft = self.local_settings.webtransport_draft_pattern();
        if matches!(
            local_draft,
            Some(crate::webtransport::DraftVersion::Draft15)
        ) && !peer.is_webtransport_enabled()
        {
            return Err(Error::StreamError(ErrorCode::MessageError));
        }
        if peer.h3_datagram != Some(true) {
            return Err(Error::StreamError(ErrorCode::MessageError));
        }
        if !self.wt_transport_verified {
            return Err(Error::StreamError(ErrorCode::MessageError));
        }
        let draft = self.negotiated_wt_draft_version();
        if draft.is_some_and(|d| d.requires_reset_stream_at()) && !self.wt_reset_stream_at_supported
        {
            return Err(Error::StreamError(ErrorCode::MessageError));
        }
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
        let scheme_is_https = headers
            .iter()
            .any(|h| h.name() == b":scheme" && h.value() == b"https");
        if !scheme_is_https {
            return Err(Error::StreamError(ErrorCode::MessageError));
        }
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

        Ok(())
    }

    /// サーバー側: WebTransport CONNECT セッションの Pending 登録 (0077 Phase 5: 混在関数抽出)
    ///
    /// (draft-ietf-webtrans-http3-15 Section 3, 3.3, 4.6)
    /// 非 WT CONNECT の場合は何もしない。
    pub(crate) fn register_wt_connect_session(
        &mut self,
        stream_id: u64,
        headers: &[crate::qpack::Header],
    ) {
        if self.role != Role::Server || !super::is_webtransport_connect(headers) {
            return;
        }

        let session = self
            .wt_sessions
            .entry(stream_id)
            .or_insert_with(WtSession::new);
        for h in headers {
            if h.name() == b"wt-available-protocols" {
                if let Ok(value) = std::str::from_utf8(h.value()) {
                    session.available_protocols =
                        crate::webtransport::ConnectRequest::parse_available_protocols(value);
                }
                break;
            }
        }
        if let Some(stream) = self.streams.get_mut(&stream_id) {
            stream.set_connect();
            // WT CONNECT ストリームの DATA は Capsule データであり recv_body に累積しない
            stream.set_wt_connect();
        }
    }

    /// クライアント側: WebTransport CONNECT の 2xx レスポンス処理 (0077 Phase 5: 混在関数抽出)
    ///
    /// (draft-ietf-webtrans-http3-15 Section 3, 3.3, 5.1, 5.5, 5.6)
    /// WT-Protocol 検証、セッション確立、フロー制御初期化、バッファリング配送を行う。
    /// 非 WT セッションまたは非 2xx の場合は何もしない。
    pub(crate) fn handle_wt_connect_response(
        &mut self,
        stream_id: u64,
        headers: &[crate::qpack::Header],
    ) -> Result<(), Error> {
        if self.role != Role::Client || !super::is_success_status(headers) {
            return Ok(());
        }

        // RFC 9297 Section 3.2: Capsule Protocol を使用するレスポンスに
        // Content-Length / Content-Type ヘッダーを付与してはならない (MUST NOT)。
        // 違反は malformed として H3_MESSAGE_ERROR で拒否する。
        // (WT セッションのレスポンスのみが対象)
        if self.wt_sessions.contains_key(&stream_id) && has_forbidden_capsule_headers(headers) {
            return Err(Error::StreamError(ErrorCode::MessageError));
        }

        // RFC 9297 Section 3.2: HTTP status codes 204 (No Content) / 205 (Reset
        // Content) / 206 (Partial Content) は Capsule Protocol を使用する
        // レスポンスに付与してはならない (MUST NOT)。違反は malformed。
        // (WT セッションのレスポンスのみが対象。通常 HTTP の 204/205/206 は
        //  この制約の対象外)
        if self.wt_sessions.contains_key(&stream_id) && is_forbidden_capsule_status(headers) {
            return Err(Error::StreamError(ErrorCode::MessageError));
        }

        // WT-Protocol 検証 (Section 3.3)
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
                    selected.is_some()
                } else {
                    match &selected {
                        None => true,
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
            self.terminate_wt_session_with(
                stream_id,
                WtErrorCode::AlpnError as u64,
                0,
                String::new(),
            );
            return Ok(());
        }

        let fc_enabled = self.is_wt_flow_control_enabled();
        let queue_initial_capsules = fc_enabled && self.peer_requires_initial_wt_capsules();
        let mut session_established = false;

        if let Some(session) = self.wt_sessions.get_mut(&stream_id)
            && session.state == WtSessionState::Pending
        {
            session.flow_control_enabled = fc_enabled;
            if let Some(wt) = &self.local_settings.wt_settings {
                session.initialize_flow_control(wt, queue_initial_capsules);
            }
            session.state = WtSessionState::Established;
            session_established = true;
        }

        if session_established {
            if let Some(stream) = self.streams.get_mut(&stream_id) {
                stream.set_connect();
            }
            self.events
                .push_back(Event::WebTransport(WebTransportEvent::SessionEstablished {
                    session_id: stream_id,
                    flow_control_enabled: fc_enabled,
                }));
            let fc_violation = self.deliver_buffered_streams(stream_id);
            if fc_violation {
                self.terminate_wt_session_with(
                    stream_id,
                    WtErrorCode::FlowControlError as u64,
                    0,
                    String::new(),
                );
            }
        }

        Ok(())
    }

    /// サーバー側: WebTransport 2xx レスポンスの WT-Protocol 検証 (0077 Phase 5: 混在関数抽出)
    ///
    /// (draft-ietf-webtrans-http3-15 Section 3.3)
    /// 非 WT セッションまたは非 2xx の場合は `Ok(())` を返す。
    pub(crate) fn validate_wt_response_protocol(
        &self,
        stream_id: u64,
        headers: &[crate::qpack::Header],
    ) -> Result<(), Error> {
        if !super::is_success_status(headers) {
            return Ok(());
        }
        let Some(session) = self.wt_sessions.get(&stream_id) else {
            return Ok(());
        };

        // RFC 9297 Section 3.2: Capsule Protocol を使用するレスポンスに
        // Content-Length / Content-Type ヘッダーを付与してはならない (MUST NOT)。
        // 送信側も違反メッセージを生成しない (受信側は malformed として拒否する)
        if has_forbidden_capsule_headers(headers) {
            return Err(Error::StreamError(ErrorCode::MessageError));
        }

        // RFC 9297 Section 3.2: HTTP status codes 204 / 205 / 206 は Capsule
        // Protocol を使用するレスポンスに付与してはならない (MUST NOT)。
        // 送信側も違反メッセージを生成しない
        if is_forbidden_capsule_status(headers) {
            return Err(Error::StreamError(ErrorCode::MessageError));
        }

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
            if selected.is_some() {
                return Err(Error::ConnectionError(ErrorCode::InternalError));
            }
        } else {
            match &selected {
                None => {
                    return Err(Error::ConnectionError(ErrorCode::InternalError));
                }
                Some(proto) => {
                    if !session.available_protocols.contains(proto) {
                        return Err(Error::ConnectionError(ErrorCode::InternalError));
                    }
                }
            }
        }

        Ok(())
    }

    /// サーバー側: WebTransport CONNECT に対する 2xx レスポンス送信時のセッション確立
    /// (0077 Phase 5: 混在関数抽出)
    ///
    /// (draft-ietf-webtrans-http3-15 Section 3, 5.1, 5.5, 5.6)
    /// 非 WT セッションまたは非 2xx の場合は何もしない。
    pub(crate) fn establish_wt_session_server(
        &mut self,
        stream_id: u64,
        headers: &[crate::qpack::Header],
    ) {
        if self.role != Role::Server || !super::is_success_status(headers) {
            return;
        }

        let fc_enabled = self.is_wt_flow_control_enabled();
        let queue_initial_capsules = fc_enabled && self.peer_requires_initial_wt_capsules();
        let mut session_established = false;

        if let Some(session) = self.wt_sessions.get_mut(&stream_id)
            && session.state == WtSessionState::Pending
        {
            session.flow_control_enabled = fc_enabled;
            if let Some(wt) = &self.local_settings.wt_settings {
                session.initialize_flow_control(wt, queue_initial_capsules);
            }
            session.state = WtSessionState::Established;
            session_established = true;
        }

        if session_established {
            self.events
                .push_back(Event::WebTransport(WebTransportEvent::SessionEstablished {
                    session_id: stream_id,
                    flow_control_enabled: fc_enabled,
                }));
            let fc_violation = self.deliver_buffered_streams(stream_id);
            if fc_violation {
                self.terminate_wt_session_with(
                    stream_id,
                    WtErrorCode::FlowControlError as u64,
                    0,
                    String::new(),
                );
            }
        }
    }
}
