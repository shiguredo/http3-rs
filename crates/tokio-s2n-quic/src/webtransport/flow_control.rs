//! WebTransport 送信側フロー制御
//!
//! ピアが SETTINGS と WT_MAX_STREAMS / WT_MAX_DATA カプセルで広告した送信上限を
//! 追跡し、ストリーム開設とデータ送信が上限を超えないよう検証する。
//! 受信側フロー制御 (ピアが開くストリーム / 送ってくるデータの上限) は
//! sans-I/O 層が管理するため、本モジュールは送信側だけを扱う。
//!
//! 根拠:
//! - draft-ietf-webtrans-http3-16 Section 5.5 (SETTINGS による初期値)
//! - draft-ietf-webtrans-http3-16 Section 5.6.2 (WT_MAX_STREAMS)
//! - draft-ietf-webtrans-http3-16 Section 5.6.4 (WT_MAX_DATA)
//!
//! 将来のドラフトで変更される可能性がある。

use shiguredo_http3::Settings as H3Settings;
use shiguredo_http3::webtransport::Capsule;
use shiguredo_http3::webtransport::CapsuleValidationError;
use shiguredo_http3::webtransport::FlowControlLimits;
use shiguredo_http3::webtransport::capsule::MAX_STREAMS_LIMIT;

use crate::error::FlowControlResource;

/// ピアの SETTINGS から送信側フロー制御の初期値を取り出す
///
/// ピアが SETTINGS_WT_INITIAL_MAX_STREAMS_UNI / SETTINGS_WT_INITIAL_MAX_STREAMS_BIDI /
/// SETTINGS_WT_INITIAL_MAX_DATA で宣言した値が、こちらが開設できるストリーム数と
/// 送信できるデータ量の初期上限になる
/// (draft-ietf-webtrans-http3-16 Section 5.5.1, 5.5.2, 5.5.3)。
///
/// ピアが WebTransport の SETTINGS を送っていない場合、および各値が未設定の場合は
/// 0 (ストリーム開設・データ送信は不可) になる
/// (draft-ietf-webtrans-http3-16 Section 5.5 の各デフォルト値)。
pub(crate) fn peer_wt_send_limits(peer_settings: Option<&H3Settings>) -> FlowControlLimits {
    let Some(wt) = peer_settings.and_then(|settings| settings.wt_settings.as_ref()) else {
        return FlowControlLimits::new();
    };
    FlowControlLimits {
        max_streams_uni: wt.wt_initial_max_streams_uni.get(),
        max_streams_bidi: wt.wt_initial_max_streams_bidi.get(),
        max_data: wt.wt_initial_max_data.get(),
    }
}

/// ピアが広告した送信側上限と送信実績
pub(crate) struct WtSendFlowControl {
    /// フロー制御が有効かどうか
    ///
    /// 両端が SETTINGS でフロー制御の利用を宣言した場合のみ `true` になる
    /// (draft-ietf-webtrans-http3-16 Section 5.1)。無効時は上限を適用せず、
    /// フロー制御カプセルも無視する (MUST)。
    enabled: bool,
    /// ピアが広告した上限
    limits: FlowControlLimits,
    /// 開設済みの双方向ストリーム数
    ///
    /// 上限は「セッションの生存期間中に開設できる累計数」であり、
    /// 閉じたストリームも数に含む (draft-ietf-webtrans-http3-16 Section 5.6.2)。
    opened_bidi: u64,
    /// 開設済みの単方向ストリーム数 (双方向と同じく累計数)
    opened_uni: u64,
    /// 送信済みの Stream Body データ量 (バイト)
    ///
    /// カプセル・ストリームヘッダー (Signal Value / Stream Type / Session ID) は
    /// 含めない (draft-ietf-webtrans-http3-16 Section 5.4)。
    data_sent: u64,
}

impl WtSendFlowControl {
    /// 送信側フロー制御状態を作成する
    ///
    /// `enabled` は両端の SETTINGS でフロー制御が成立したかどうか、
    /// `limits` は [`peer_wt_send_limits`] で取り出したピア広告値。
    pub(crate) fn new(enabled: bool, limits: FlowControlLimits) -> Self {
        Self {
            enabled,
            limits,
            opened_bidi: 0,
            opened_uni: 0,
            data_sent: 0,
        }
    }

    /// ストリーム開設を試行する
    ///
    /// ピアが広告した上限に達している場合は
    /// [`crate::Error::FlowControlBlocked`] を返す。呼び出し側は QUIC ストリームを
    /// 開く前に本メソッドを呼ぶこと (上限超過の開設はピアに
    /// WT_FLOW_CONTROL_ERROR を発生させる)。
    pub(crate) fn try_open_stream(&mut self, bidirectional: bool) -> crate::Result<()> {
        if !self.enabled {
            // フロー制御無効時はストリーム数上限を適用しない
            // (draft-ietf-webtrans-http3-16 Section 5.1)
            if bidirectional {
                self.opened_bidi = self.opened_bidi.saturating_add(1);
            } else {
                self.opened_uni = self.opened_uni.saturating_add(1);
            }
            return Ok(());
        }

        let (opened, limit, resource) = if bidirectional {
            (
                &mut self.opened_bidi,
                self.limits.max_streams_bidi,
                FlowControlResource::BidirectionalStream,
            )
        } else {
            (
                &mut self.opened_uni,
                self.limits.max_streams_uni,
                FlowControlResource::UnidirectionalStream,
            )
        };

        if *opened >= limit {
            return Err(crate::Error::FlowControlBlocked { resource });
        }
        *opened = opened.saturating_add(1);
        Ok(())
    }

    /// データ送信を試行する
    ///
    /// ピアが広告した `WT_MAX_DATA` を超える場合は
    /// [`crate::Error::FlowControlBlocked`] を返す。
    ///
    /// 呼び出し側は送信が成功したデータ量だけを渡すこと。送信失敗時に
    /// 計上が残っても、その時点でセッションは破棄されるため問題にならない
    /// (過大計上は安全側に倒れる)。
    pub(crate) fn try_send_data(&mut self, bytes: u64) -> crate::Result<()> {
        if !self.enabled {
            // フロー制御無効時はデータ量上限を適用しない
            // (draft-ietf-webtrans-http3-16 Section 5.1)
            self.data_sent = self.data_sent.saturating_add(bytes);
            return Ok(());
        }

        if self.limits.max_data.saturating_sub(self.data_sent) < bytes {
            return Err(crate::Error::FlowControlBlocked {
                resource: FlowControlResource::Data,
            });
        }
        self.data_sent = self.data_sent.saturating_add(bytes);
        Ok(())
    }

    /// ピアから受信したフロー制御カプセルを検証して上限を更新する
    ///
    /// 値が増加していない場合は WT_FLOW_CONTROL_ERROR でセッションを閉じる必要が
    /// あるため `Err` を返す (draft-ietf-webtrans-http3-16 Section 5.6.2, 5.6.4)。
    ///
    /// フロー制御が無効な場合はすべて無視する (MUST)
    /// (draft-ietf-webtrans-http3-16 Section 5.1)。
    pub(crate) fn process_capsule(
        &mut self,
        capsule: &Capsule,
    ) -> Result<(), CapsuleValidationError> {
        if !self.enabled {
            return Ok(());
        }

        match capsule {
            Capsule::MaxStreams {
                bidirectional,
                maximum,
            } => {
                if *bidirectional {
                    Capsule::validate_max_streams(*maximum, self.limits.max_streams_bidi)?;
                    self.limits.max_streams_bidi = *maximum;
                } else {
                    Capsule::validate_max_streams(*maximum, self.limits.max_streams_uni)?;
                    self.limits.max_streams_uni = *maximum;
                }
            }
            Capsule::MaxData { maximum } => {
                // `Maximum Data` は varint で受信しているため VarInt 上限を超える値は
                // 到達しない。`validate_max_data` は減少の検出に使う。
                Capsule::validate_max_data(*maximum, self.limits.max_data)?;
                self.limits.max_data = *maximum;
            }
            Capsule::StreamsBlocked { maximum, .. } => {
                // 2^60 を超える値は WT_FLOW_CONTROL_ERROR
                // (draft-ietf-webtrans-http3-16 Section 5.6.3)
                if *maximum > MAX_STREAMS_LIMIT {
                    return Err(CapsuleValidationError::MaxStreamsExceedsLimit);
                }
            }
            // WT_DATA_BLOCKED は情報目的のみで、上限の更新は不要
            // (draft-ietf-webtrans-http3-16 Section 5.6.5)
            Capsule::DataBlocked { .. } => {}
            // フロー制御カプセル以外は本モジュールの責務ではない
            Capsule::CloseSession { .. } | Capsule::DrainSession | Capsule::Unknown { .. } => {}
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shiguredo_http3::VarInt;
    use shiguredo_http3::webtransport;

    /// テスト用: 指定した初期上限を持つピア SETTINGS を構築する
    fn peer_settings(uni: u64, bidi: u64, data: u64) -> H3Settings {
        let value = |v: u64| VarInt::new(v).expect("テスト用の値が VarInt 範囲内であること");
        let wt = webtransport::Settings::new()
            .wt_enabled(VarInt::from_static(1))
            .wt_initial_max_streams_uni(value(uni))
            .wt_initial_max_streams_bidi(value(bidi))
            .wt_initial_max_data(value(data));
        H3Settings {
            wt_settings: Some(wt),
            ..H3Settings::default()
        }
    }

    #[test]
    fn test_peer_wt_send_limits_reads_settings() {
        // ピア SETTINGS の WT 初期値が送信側上限として取り出されること
        // (draft-ietf-webtrans-http3-16 Section 5.5)
        let settings = peer_settings(3, 5, 1024);
        let limits = peer_wt_send_limits(Some(&settings));
        assert_eq!(
            limits.max_streams_uni, 3,
            "単方向ストリーム上限が反映されること"
        );
        assert_eq!(
            limits.max_streams_bidi, 5,
            "双方向ストリーム上限が反映されること"
        );
        assert_eq!(limits.max_data, 1024, "データ上限が反映されること");
    }

    #[test]
    fn test_peer_wt_send_limits_without_wt_settings_is_zero() {
        // ピアが WebTransport の SETTINGS を送っていない場合は全上限 0
        let limits = peer_wt_send_limits(None);
        assert_eq!(limits.max_streams_uni, 0);
        assert_eq!(limits.max_streams_bidi, 0);
        assert_eq!(limits.max_data, 0);
    }

    #[test]
    fn test_try_open_stream_up_to_limit() {
        // 上限に達するまで開設でき、上限到達後は FlowControlBlocked になること
        // (draft-ietf-webtrans-http3-16 Section 5.6.2)
        let mut fc =
            WtSendFlowControl::new(true, peer_wt_send_limits(Some(&peer_settings(2, 1, 0))));

        fc.try_open_stream(true)
            .expect("双方向ストリームを上限内で開設できること");
        let result = fc.try_open_stream(true);
        assert!(
            matches!(
                result,
                Err(crate::Error::FlowControlBlocked {
                    resource: FlowControlResource::BidirectionalStream
                })
            ),
            "双方向ストリーム上限超過で FlowControlBlocked になること: {result:?}"
        );

        fc.try_open_stream(false)
            .expect("単方向ストリームを上限内で開設できること");
        fc.try_open_stream(false)
            .expect("単方向ストリームを 2 本目まで開設できること");
        let result = fc.try_open_stream(false);
        assert!(
            matches!(
                result,
                Err(crate::Error::FlowControlBlocked {
                    resource: FlowControlResource::UnidirectionalStream
                })
            ),
            "単方向ストリーム上限超過で FlowControlBlocked になること: {result:?}"
        );
    }

    #[test]
    fn test_try_open_stream_unlimited_when_flow_control_disabled() {
        // フロー制御無効時は SETTINGS の値 (0) に関係なく開設できること
        // (draft-ietf-webtrans-http3-16 Section 5.1)
        let mut fc = WtSendFlowControl::new(false, FlowControlLimits::new());
        for _ in 0..1000 {
            fc.try_open_stream(true)
                .expect("フロー制御無効時は上限を適用しないこと");
        }
        assert_eq!(fc.opened_bidi, 1000);
    }

    #[test]
    fn test_try_send_data_up_to_limit() {
        // WT_MAX_DATA の範囲内でのみ送信でき、超過は FlowControlBlocked になること
        // (draft-ietf-webtrans-http3-16 Section 5.6.4)
        let mut fc =
            WtSendFlowControl::new(true, peer_wt_send_limits(Some(&peer_settings(0, 0, 100))));

        fc.try_send_data(60)
            .expect("上限内のデータ送信に成功すること");
        fc.try_send_data(40)
            .expect("ちょうど上限までのデータ送信に成功すること");
        let result = fc.try_send_data(1);
        assert!(
            matches!(
                result,
                Err(crate::Error::FlowControlBlocked {
                    resource: FlowControlResource::Data
                })
            ),
            "データ上限超過で FlowControlBlocked になること: {result:?}"
        );
    }

    #[test]
    fn test_process_capsule_updates_limits() {
        // 増加する WT_MAX_STREAMS / WT_MAX_DATA で上限が更新されること
        // (draft-ietf-webtrans-http3-16 Section 5.6.2, 5.6.4)
        let mut fc =
            WtSendFlowControl::new(true, peer_wt_send_limits(Some(&peer_settings(1, 1, 100))));

        fc.process_capsule(&Capsule::MaxStreams {
            bidirectional: true,
            maximum: 4,
        })
        .expect("増加する WT_MAX_STREAMS が受理されること");
        assert_eq!(fc.limits.max_streams_bidi, 4);

        fc.process_capsule(&Capsule::MaxStreams {
            bidirectional: false,
            maximum: 4,
        })
        .expect("増加する WT_MAX_STREAMS が受理されること");
        assert_eq!(fc.limits.max_streams_uni, 4);

        fc.process_capsule(&Capsule::MaxData { maximum: 200 })
            .expect("増加する WT_MAX_DATA が受理されること");
        assert_eq!(fc.limits.max_data, 200);
    }

    #[test]
    fn test_process_capsule_rejects_non_increasing_max_streams() {
        // SETTINGS 初期値 (または前回値) 以下に戻す WT_MAX_STREAMS は拒否すること
        // (draft-ietf-webtrans-http3-16 Section 5.6.2 の MUST)
        let mut fc =
            WtSendFlowControl::new(true, peer_wt_send_limits(Some(&peer_settings(1, 10, 0))));

        // SETTINGS 初期値と同じ値は「増加していない」ため拒否する
        let result = fc.process_capsule(&Capsule::MaxStreams {
            bidirectional: true,
            maximum: 10,
        });
        assert_eq!(
            result,
            Err(CapsuleValidationError::MaxStreamsDecreased),
            "増加しない WT_MAX_STREAMS は拒否すること"
        );

        fc.process_capsule(&Capsule::MaxStreams {
            bidirectional: true,
            maximum: 20,
        })
        .expect("増加する値は受理されること");
        let result = fc.process_capsule(&Capsule::MaxStreams {
            bidirectional: true,
            maximum: 15,
        });
        assert_eq!(
            result,
            Err(CapsuleValidationError::MaxStreamsDecreased),
            "前回値より小さい WT_MAX_STREAMS は拒否すること"
        );
    }

    #[test]
    fn test_process_capsule_rejects_non_increasing_max_data() {
        // SETTINGS 初期値 (または前回値) 以下に戻す WT_MAX_DATA は拒否すること
        // (draft-ietf-webtrans-http3-16 Section 5.6.4 の MUST)
        let mut fc =
            WtSendFlowControl::new(true, peer_wt_send_limits(Some(&peer_settings(0, 0, 100))));

        let result = fc.process_capsule(&Capsule::MaxData { maximum: 100 });
        assert_eq!(
            result,
            Err(CapsuleValidationError::MaxDataDecreased),
            "増加しない WT_MAX_DATA は拒否すること"
        );

        let result = fc.process_capsule(&Capsule::MaxData { maximum: 50 });
        assert_eq!(
            result,
            Err(CapsuleValidationError::MaxDataDecreased),
            "前回値より小さい WT_MAX_DATA は拒否すること"
        );
    }

    #[test]
    fn test_process_capsule_rejects_streams_blocked_over_limit() {
        // 2^60 を超える WT_STREAMS_BLOCKED は拒否すること
        // (draft-ietf-webtrans-http3-16 Section 5.6.3)
        let mut fc = WtSendFlowControl::new(true, FlowControlLimits::new());
        let result = fc.process_capsule(&Capsule::StreamsBlocked {
            bidirectional: false,
            maximum: MAX_STREAMS_LIMIT + 1,
        });
        assert_eq!(result, Err(CapsuleValidationError::MaxStreamsExceedsLimit));

        // 2^60 ちょうどは許容する
        fc.process_capsule(&Capsule::StreamsBlocked {
            bidirectional: false,
            maximum: MAX_STREAMS_LIMIT,
        })
        .expect("2^60 ちょうどの WT_STREAMS_BLOCKED は許容されること");
    }

    #[test]
    fn test_process_capsule_ignored_when_flow_control_disabled() {
        // フロー制御無効時はフロー制御カプセルを無視すること (MUST)
        // (draft-ietf-webtrans-http3-16 Section 5.1)
        let mut fc = WtSendFlowControl::new(false, FlowControlLimits::new());
        fc.process_capsule(&Capsule::MaxData { maximum: 10 })
            .expect("フロー制御無効時は WT_MAX_DATA を無視すること");
        fc.process_capsule(&Capsule::MaxStreams {
            bidirectional: true,
            maximum: 10,
        })
        .expect("フロー制御無効時は WT_MAX_STREAMS を無視すること");
        assert_eq!(fc.limits.max_data, 0);
        assert_eq!(fc.limits.max_streams_bidi, 0);
    }

    #[test]
    fn test_process_capsule_ignores_non_flow_control_capsules() {
        // WT_DATA_BLOCKED は情報目的のみで、セッション終端カプセルも本モジュールでは扱わない
        let mut fc = WtSendFlowControl::new(true, FlowControlLimits::new());
        fc.process_capsule(&Capsule::DataBlocked { maximum: 0 })
            .expect("WT_DATA_BLOCKED は無視されること");
        fc.process_capsule(&Capsule::DrainSession)
            .expect("WT_DRAIN_SESSION は本モジュールでは扱わないこと");
    }
}
