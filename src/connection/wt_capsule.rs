//! WebTransport Capsule 処理 (0077: connection/mod.rs から分離)
//!
//! WebTransport CONNECT ストリーム上の Capsule デコード・処理を担う
//! `Connection` メソッド群。
//! (draft-ietf-webtrans-http3-15 Section 5.6, 6)

use crate::error::{Error, ErrorCode};
use crate::event::{Event, WebTransportEvent};
use crate::webtransport::error::ErrorCode as WtErrorCode;

use super::Connection;
use super::wt_types::WtSessionState;

impl Connection {
    /// WebTransport CONNECT ストリーム上のデータを Capsule としてデコード・処理する
    ///
    /// DATA フレームのペイロードを Capsule デコードバッファに追加し、
    /// 完全な Capsule が得られるまでデコードを試みる。
    /// (draft-ietf-webtrans-http3-15 Section 5.6)
    pub(crate) fn process_wt_capsule_data(
        &mut self,
        session_id: u64,
        data: &[u8],
    ) -> Result<(), Error> {
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
    pub(crate) fn handle_wt_capsule(
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
                    self.events.push_back(Event::WebTransport(
                        WebTransportEvent::SessionDraining { session_id },
                    ));
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
                    self.events
                        .push_back(Event::WebTransport(WebTransportEvent::Capsule {
                            session_id,
                            capsule: capsule.clone(),
                        }));
                }
                // フロー制御無効時は無視 (Section 5.1)
            }
            Capsule::Unknown { .. } => {
                // 禁止 Capsule (WT_MAX_STREAM_DATA / WT_STREAM_DATA_BLOCKED) は
                // セッションエラーとして扱う
                // (draft-ietf-webtrans-http3-15 Section 5.4: "Endpoints MUST treat
                // receipt of a WT_MAX_STREAM_DATA or a WT_STREAM_DATA_BLOCKED
                // capsule as a session error.")
                // 将来のドラフトで変更される可能性がある
                if capsule.is_prohibited_in_http3() {
                    self.terminate_wt_session_with(
                        session_id,
                        WtErrorCode::SessionGone as u64,
                        0,
                        "prohibited capsule received".to_string(),
                    );
                }
                // その他の不明な Capsule は無視 (draft-ietf-webtrans-http3-15)
            }
        }

        Ok(())
    }

    /// WebTransport CONNECT ストリーム上の DATA フレーム処理 (0077 Phase 5: 混在関数抽出)
    ///
    /// (draft-ietf-webtrans-http3-15 Section 5.6)
    /// WT セッションの DATA フレームを処理する。非 WT ストリームは `false` を返す。
    ///
    /// draft 別の扱い:
    /// - draft-07/14/15: Pending 中は楽観的カプセル送信としてバッファリングする
    ///   (draft-ietf-webtrans-http3-16 Section 3.2)
    /// - draft-02: Chrome 互換のため Pending 中の DATA は黙って破棄
    pub(crate) fn handle_wt_data_frame(
        &mut self,
        stream_id: u64,
        data: &[u8],
    ) -> Result<bool, Error> {
        let Some(session) = self.wt_sessions.get(&stream_id) else {
            // 終了済みセッション (tombstone) の CONNECT ストリームへの追加 DATA は
            // H3_MESSAGE_ERROR で拒否する
            // (draft-ietf-webtrans-http3-16 Section 6: WT_CLOSE_SESSION 後の
            //  追加データは H3_MESSAGE_ERROR)
            if self.closed_wt_sessions.contains(&stream_id) {
                return Err(Error::StreamError(ErrorCode::MessageError));
            }
            return Ok(false);
        };

        match session.state {
            WtSessionState::Established | WtSessionState::Draining => {
                // Draining 中もカプセル受信は継続する (Section 4.7)
                self.process_wt_capsule_data(stream_id, data)?;
            }
            WtSessionState::Pending => {
                let peer_draft = self.peer_wt_draft_version();
                if !matches!(peer_draft, Some(crate::webtransport::DraftVersion::Draft02)) {
                    // draft-07/14/15: 楽観的カプセル送信としてバッファリングする
                    // (draft-ietf-webtrans-http3-16 Section 3.2)
                    // サーバー側のみ: クライアントは楽観的送信を送信方向にのみ行う
                    if self.role == crate::connection::Role::Server {
                        // DoS 対策: バッファ上限を超えたら H3_MESSAGE_ERROR でリセットする
                        const PENDING_CAPSULE_BUF_LIMIT: usize = 64 * 1024;
                        let session = self
                            .wt_sessions
                            .get_mut(&stream_id)
                            .expect("session must exist");
                        if session.capsule_buf.len() + data.len() > PENDING_CAPSULE_BUF_LIMIT {
                            return Err(Error::StreamError(ErrorCode::MessageError));
                        }
                        session.capsule_buf.extend_from_slice(data);
                        return Ok(true);
                    }
                    return Err(Error::StreamError(ErrorCode::MessageError));
                }
                // draft-02: Pending 中の DATA は破棄する
            }
            WtSessionState::Closed => {
                return Err(Error::StreamError(ErrorCode::MessageError));
            }
        }

        Ok(true)
    }

    /// WebTransport CONNECT ストリームの StreamEnd 処理 (0077 Phase 5: 混在関数抽出)
    ///
    /// (draft-ietf-webtrans-http3-15 Section 5.6, 6)
    /// FIN 到着時に未完成 Capsule が残っていれば malformed。
    /// WT セッションの FIN はセッション終了を意味する。
    pub(crate) fn handle_wt_stream_end(&mut self, stream_id: u64) -> Result<bool, Error> {
        if let Some(session) = self.wt_sessions.get(&stream_id) {
            if session.state == WtSessionState::Pending {
                // Pending 状態でカプセルバッファが残っている場合は、
                // バッファを破棄してセッションを終了する
                // (draft-ietf-webtrans-http3-16 Section 3.2 / Section 6:
                //  CONNECT ストリームのクローズはセッション終了を意味する)
                self.wt_sessions.remove(&stream_id);
                self.closed_wt_sessions.insert(stream_id);
                return Ok(true);
            }
            if !session.capsule_buf.is_empty() {
                return Err(Error::StreamError(ErrorCode::MessageError));
            }
            self.terminate_wt_session(stream_id);
            return Ok(true);
        }
        // 終了済みセッション (tombstone) の CONNECT ストリームの FIN は受理して何もしない
        // (WT_CLOSE_SESSION を含む DATA と FIN が同一バッファに連続するのは正常な
        //  終了手順であり、FIN を H3_MESSAGE_ERROR にしてはならない。
        //  draft-ietf-webtrans-http3-16 Section 6)
        if self.closed_wt_sessions.contains(&stream_id) {
            return Ok(true);
        }
        Ok(false)
    }
}
