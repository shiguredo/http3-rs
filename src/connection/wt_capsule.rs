//! WebTransport Capsule 処理 connection/mod.rs からの分離
//!
//! WebTransport CONNECT ストリーム上の Capsule デコード・処理を担う
//! `Connection` メソッド群。
//! (draft-ietf-webtrans-http3-16 Section 5.6, 6)

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
    /// (draft-ietf-webtrans-http3-16 Section 5.6)
    pub(crate) fn process_wt_capsule_data(
        &mut self,
        session_id: u64,
        data: &[u8],
    ) -> Result<(), Error> {
        // セッションの capsule_buf にデータを追加する。
        // ピアが巨大な length を宣言したまま少しずつ送るとバッファが増え続けるため
        // 上限を設ける (セッション確立後の経路にも DoS 対策が必要)。
        const MAX_ESTABLISHED_CAPSULE_BUF: usize = 64 * 1024;
        if let Some(session) = self.wt_sessions.get_mut(&session_id) {
            if session.capsule_buf.len() + data.len() > MAX_ESTABLISHED_CAPSULE_BUF {
                return Err(Error::StreamError(ErrorCode::MessageError));
            }
            session.capsule_buf.extend_from_slice(data);
        } else {
            return Ok(());
        }

        // Capsule を逐次デコード
        while let Some(session) = self.wt_sessions.get(&session_id) {
            if session.capsule_buf.is_empty() {
                break;
            }
            // 毎回クローンすると O(n^2) になるため、消費済みバイトだけを
            // 取り出してデコードする (バッファ全体のコピーを避ける)。
            let start = session.capsule_buf_start;
            let buf = session.capsule_buf[start..].to_vec();

            match crate::webtransport::Capsule::decode(&buf) {
                Ok(Some((capsule, consumed))) => {
                    let has_trailing = buf.len() > consumed;
                    let is_close_session =
                        matches!(capsule, crate::webtransport::Capsule::CloseSession { .. });
                    // バッファから消費済み部分を除去する。
                    // drain を毎回行うと O(n^2) になるため、読み出し位置を進め、
                    // 半分割以上を消費した時点でまとめて切り詰める。
                    if let Some(session) = self.wt_sessions.get_mut(&session_id) {
                        session.capsule_buf_start += consumed;
                        if session.capsule_buf_start * 2 >= session.capsule_buf.len() {
                            session.capsule_buf.drain(..session.capsule_buf_start);
                            session.capsule_buf_start = 0;
                        }
                    }

                    // Capsule を処理してイベントに変換
                    self.handle_wt_capsule(session_id, &capsule)?;

                    // WT_CLOSE_SESSION に続く同一 DATA フレーム内の追加バイトは
                    // H3_MESSAGE_ERROR で拒否する (draft-ietf-webtrans-http3-16 Section 6)。
                    // セッション除去で while ループが終了するため、ここで検出する。
                    if is_close_session && has_trailing {
                        return Err(Error::StreamError(ErrorCode::MessageError));
                    }
                }
                Ok(None) => {
                    // バッファ不足: 次の DATA フレームを待つ
                    // 未消費分が上限に近づいたら切り詰めてメモリを解放する
                    if let Some(session) = self.wt_sessions.get_mut(&session_id)
                        && session.capsule_buf_start > 0
                        && session.capsule_buf.len() > MAX_ESTABLISHED_CAPSULE_BUF
                    {
                        session.capsule_buf.drain(..session.capsule_buf_start);
                        session.capsule_buf_start = 0;
                    }
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
                // (draft-ietf-webtrans-http3-16 Section 6)
                //
                // 終了後は tombstone (`closed_wt_sessions`) 経由で追加データを
                // H3_MESSAGE_ERROR として拒否する (draft-ietf-webtrans-http3-16 Section 6)
                self.terminate_wt_session_with(
                    session_id,
                    WtErrorCode::SessionGone as u64,
                    *error_code,
                    message.clone(),
                );
            }
            Capsule::DrainSession => {
                // WT_DRAIN_SESSION: 内部状態を Draining へ遷移し、イベントで通知する
                // (draft-ietf-webtrans-http3-16 Section 4.7)
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
            Capsule::MaxData { maximum } => {
                // ピアが広告した送信側データ上限を自層に反映する
                // (draft-ietf-webtrans-http3-16 Section 5.6.4)。
                // 増加しない値は WT_FLOW_CONTROL_ERROR でセッションを閉じる。
                // フロー制御が有効でないセッションではカプセル自体を無視する
                // (draft-ietf-webtrans-http3-16 Section 5.1)。
                let fc_enabled = self
                    .wt_sessions
                    .get(&session_id)
                    .is_some_and(|s| s.flow_control_enabled);
                if fc_enabled {
                    let accepted = self
                        .wt_sessions
                        .get_mut(&session_id)
                        .is_some_and(|s| s.apply_max_data(*maximum));
                    if !accepted {
                        self.terminate_wt_session_with(
                            session_id,
                            WtErrorCode::FlowControlError as u64,
                            0,
                            String::new(),
                        );
                        return Ok(());
                    }
                    self.notify_wt_flow_control_capsule(session_id, capsule);
                }
            }
            Capsule::MaxStreams {
                bidirectional,
                maximum,
            } => {
                // ピアが広告した送信側ストリーム上限を自層に反映する
                // (draft-ietf-webtrans-http3-16 Section 5.6.2)。
                // 2^60 超過と増加しない値は WT_FLOW_CONTROL_ERROR でセッションを閉じる。
                let fc_enabled = self
                    .wt_sessions
                    .get(&session_id)
                    .is_some_and(|s| s.flow_control_enabled);
                if fc_enabled {
                    let accepted = self
                        .wt_sessions
                        .get_mut(&session_id)
                        .is_some_and(|s| s.apply_max_streams(*bidirectional, *maximum));
                    if !accepted {
                        self.terminate_wt_session_with(
                            session_id,
                            WtErrorCode::FlowControlError as u64,
                            0,
                            String::new(),
                        );
                        return Ok(());
                    }
                    self.notify_wt_flow_control_capsule(session_id, capsule);
                }
            }
            Capsule::DataBlocked { maximum } => {
                // ピアが「データ送信でブロックしている」と通知してきた。
                // 自層の送信上限の判定には影響しないため、記録して通知するだけにする
                // (draft-ietf-webtrans-http3-16 Section 5.6.4)。
                let _ = maximum;
                let fc_enabled = self
                    .wt_sessions
                    .get(&session_id)
                    .is_some_and(|s| s.flow_control_enabled);
                if fc_enabled {
                    self.notify_wt_flow_control_capsule(session_id, capsule);
                }
            }
            Capsule::StreamsBlocked { maximum, .. } => {
                // ピアが「ストリーム開設でブロックしている」と通知してきた。
                // 2^60 を超える値はセッションエラー
                // (draft-ietf-webtrans-http3-16 Section 5.6.3: "MUST close the
                // WebTransport session with a WT_FLOW_CONTROL_ERROR error code")。
                // それ以外は自層の送信上限の判定に影響しないため通知のみ行う。
                // 将来のドラフトで変更される可能性がある
                let fc_enabled = self
                    .wt_sessions
                    .get(&session_id)
                    .is_some_and(|s| s.flow_control_enabled);
                if fc_enabled {
                    if *maximum > crate::webtransport::MAX_STREAMS_LIMIT {
                        self.terminate_wt_session_with(
                            session_id,
                            WtErrorCode::FlowControlError as u64,
                            0,
                            String::new(),
                        );
                        return Ok(());
                    }
                    self.notify_wt_flow_control_capsule(session_id, capsule);
                }
            }
            Capsule::Unknown { .. } => {
                // 禁止 Capsule (WT_MAX_STREAM_DATA / WT_STREAM_DATA_BLOCKED) は
                // セッションエラーとして扱う
                // (draft-ietf-webtrans-http3-16 Section 5.4: "Endpoints MUST treat
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
                // その他の不明な Capsule は無視 (draft-ietf-webtrans-http3-16)
            }
        }

        Ok(())
    }

    /// フロー制御カプセルを上位層へ通知する
    ///
    /// 送信側上限の反映は接続層で完了しているため、この通知は
    /// アプリケーションが観測・ログ用途で使うためのもの
    /// (draft-ietf-webtrans-http3-16 Section 5.6)。
    fn notify_wt_flow_control_capsule(
        &mut self,
        session_id: u64,
        capsule: &crate::webtransport::Capsule,
    ) {
        self.events
            .push_back(Event::WebTransport(WebTransportEvent::Capsule {
                session_id,
                capsule: capsule.clone(),
            }));
    }

    /// WebTransport CONNECT ストリーム上の DATA フレーム処理 WebTransport 混在関数の抽出
    ///
    /// (draft-ietf-webtrans-http3-16 Section 5.6)
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
        // SETTINGS 未着の WT CONNECT ストリームへの DATA は保留する
        // (draft-ietf-webtrans-http3-16 Section 3.1 / 4.6 / 7.1)
        if self.deferred_wt_connects.contains_key(&stream_id) {
            // DoS 対策: 保留中の DATA に上限を設ける
            const MAX_DEFERRED_DATA_BYTES: usize = 64 * 1024;
            let stream = self
                .streams
                .get(&stream_id)
                .expect("deferred CONNECT stream must exist");
            let current_len = stream.received_body().len();
            if current_len + data.len() > MAX_DEFERRED_DATA_BYTES {
                return Err(Error::StreamError(ErrorCode::MessageError));
            }
            return Ok(true);
        }

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

    /// WebTransport CONNECT ストリームの StreamEnd 処理 WebTransport 混在関数の抽出
    ///
    /// (draft-ietf-webtrans-http3-16 Section 5.6, 6)
    /// FIN 到着時に未完成 Capsule が残っていれば malformed。
    /// WT セッションの FIN はセッション終了を意味する。
    pub(crate) fn handle_wt_stream_end(&mut self, stream_id: u64) -> Result<bool, Error> {
        // SETTINGS 未着の WT CONNECT ストリームの FIN は保留エントリを破棄する
        if self.deferred_wt_connects.remove(&stream_id).is_some() {
            return Ok(true);
        }
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
