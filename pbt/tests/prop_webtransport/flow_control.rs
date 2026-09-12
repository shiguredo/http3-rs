//! WebTransport フロー制御の PBT (draft-ietf-webtrans-http3-16 Section 5.6)
//!
//! 実接続 (`ServerConnection`) 上に確立したセッションに対して
//! ピアの広告値と送信試行をランダムに与え、上限判定の不変条件を検証する。
//! 旧 `webtransport::Session` を対象にしていた性質を、本番経路である
//! `Connection` の公開 API に対して検証し直したもの。

use pbt::strategies::sample_varint_raw_in;
use shiguredo_http3::webtransport::{Capsule, MAX_STREAMS_LIMIT};

use super::fixture::{established_server, feed_capsule};

// =============================================================================
// WT_MAX_DATA の単調増加制約 (draft-16 Section 5.6.4)
// =============================================================================

/// Property: ピアの初期値より大きい WT_MAX_DATA は受理される
#[test]
fn prop_max_data_increase_is_accepted() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WT_FLOW_CONTROL_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(128, |ctx| {
        let (mut server, stream_id) = established_server();
        let maximum = sample_varint_raw_in(ctx, 1..=1_000_000);
        feed_capsule(&mut server, stream_id, &Capsule::MaxData { maximum });

        assert_eq!(
            server.wt_remote_max_data(stream_id),
            Some(maximum),
            "増加する WT_MAX_DATA が受理されていない: maximum={maximum}"
        );
        Ok(())
    })?;
    Ok(())
}

/// Property: 2 回目の WT_MAX_DATA が直前値以下ならセッションを閉じる
///
/// 送信側上限は最初の WT_MAX_DATA で確定するため、非増加の判定は 2 回目以降に
/// 適用される (draft-ietf-webtrans-http3-16 Section 5.6.4: "does not increase")。
#[test]
fn prop_max_data_non_increase_closes_session() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WT_FLOW_CONTROL_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(128, |ctx| {
        let (mut server, stream_id) = established_server();
        let first = sample_varint_raw_in(ctx, 1..=1_000_000);
        feed_capsule(&mut server, stream_id, &Capsule::MaxData { maximum: first });
        assert_eq!(server.wt_remote_max_data(stream_id), Some(first));

        // 直前値以下を広告する (同値を含む)
        let second = sample_varint_raw_in(ctx, 0..=first);
        feed_capsule(
            &mut server,
            stream_id,
            &Capsule::MaxData { maximum: second },
        );

        assert!(
            server.wt_session_closed(stream_id),
            "増加しない WT_MAX_DATA ({second} <= {first}) でセッションが閉じていない"
        );
        Ok(())
    })?;
    Ok(())
}

/// Property: 2 回目の WT_MAX_DATA は直前の値より大きい場合のみ受理される
#[test]
fn prop_max_data_second_increase_monotonic() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WT_FLOW_CONTROL_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(128, |ctx| {
        let (mut server, stream_id) = established_server();
        let first = sample_varint_raw_in(ctx, 1..=1_000_000);
        feed_capsule(&mut server, stream_id, &Capsule::MaxData { maximum: first });
        assert_eq!(server.wt_remote_max_data(stream_id), Some(first));

        let second_delta = sample_varint_raw_in(ctx, 1..=1_000_000);
        let second = first + second_delta;
        feed_capsule(
            &mut server,
            stream_id,
            &Capsule::MaxData { maximum: second },
        );
        assert_eq!(
            server.wt_remote_max_data(stream_id),
            Some(second),
            "単調増加する 2 回目の WT_MAX_DATA が受理されていない"
        );

        // 直前値以下を送ると閉じる
        let lower = sample_varint_raw_in(ctx, 0..=second);
        feed_capsule(&mut server, stream_id, &Capsule::MaxData { maximum: lower });
        assert!(
            !server.can_send_wt_data(stream_id, 1),
            "非増加の WT_MAX_DATA ({lower}) でセッションが閉じていない"
        );
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// WT_MAX_STREAMS の単調増加制約 (draft-16 Section 5.6.2)
// =============================================================================

/// Property: ピアの初期値より大きい WT_MAX_STREAMS は双方向・単方向とも受理される
#[test]
fn prop_max_streams_increase_is_accepted() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WT_FLOW_CONTROL_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(128, |ctx| {
        let (mut server, stream_id) = established_server();
        let maximum = sample_varint_raw_in(ctx, 1..=1_000);
        let bidirectional = noprop::sample_usize_in(ctx, 0..=1) == 1;
        feed_capsule(
            &mut server,
            stream_id,
            &Capsule::MaxStreams {
                bidirectional,
                maximum,
            },
        );

        let can_open = if bidirectional {
            server.can_open_wt_bidi_stream(stream_id)
        } else {
            server.can_open_wt_uni_stream(stream_id)
        };
        assert!(
            can_open,
            "増加する WT_MAX_STREAMS ({maximum}) が受理されていない"
        );
        Ok(())
    })?;
    Ok(())
}

/// Property: 2 回目の WT_MAX_STREAMS が直前値以下ならセッションを閉じる
///
/// 送信側上限は最初の WT_MAX_STREAMS で確定するため、非増加の判定は 2 回目以降に
/// 適用される (draft-ietf-webtrans-http3-16 Section 5.6.2: "does not increase")。
#[test]
fn prop_max_streams_non_increase_closes_session() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WT_FLOW_CONTROL_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(128, |ctx| {
        let (mut server, stream_id) = established_server();
        let bidirectional = noprop::sample_usize_in(ctx, 0..=1) == 1;
        let first = sample_varint_raw_in(ctx, 1..=1_000);
        feed_capsule(
            &mut server,
            stream_id,
            &Capsule::MaxStreams {
                bidirectional,
                maximum: first,
            },
        );

        let second = sample_varint_raw_in(ctx, 0..=first);
        feed_capsule(
            &mut server,
            stream_id,
            &Capsule::MaxStreams {
                bidirectional,
                maximum: second,
            },
        );

        assert!(
            server.wt_session_closed(stream_id),
            "増加しない WT_MAX_STREAMS ({second} <= {first}) でセッションが閉じていない"
        );
        Ok(())
    })?;
    Ok(())
}

/// Property: 2^60 を超える WT_MAX_STREAMS は WT_FLOW_CONTROL_ERROR として拒否される
#[test]
fn prop_max_streams_above_limit_closes_session() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WT_FLOW_CONTROL_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(64, |ctx| {
        let (mut server, stream_id) = established_server();
        let excess = sample_varint_raw_in(ctx, 1..=1_000_000);
        let maximum = MAX_STREAMS_LIMIT + excess;
        let bidirectional = noprop::sample_usize_in(ctx, 0..=1) == 1;
        feed_capsule(
            &mut server,
            stream_id,
            &Capsule::MaxStreams {
                bidirectional,
                maximum,
            },
        );

        assert!(
            server.wt_session_closed(stream_id),
            "2^60 を超える WT_MAX_STREAMS ({maximum}) が拒否されていない"
        );
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// 送信上限の境界 (draft-16 Section 5.6.2, 5.6.4)
// =============================================================================

/// Property: 広告された上限ちょうどまで送信でき、1 バイト超えると送信できない
#[test]
fn prop_data_send_boundary() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WT_FLOW_CONTROL_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(128, |ctx| {
        let (mut server, stream_id) = established_server();
        let limit = sample_varint_raw_in(ctx, 1..=1_000_000);
        feed_capsule(&mut server, stream_id, &Capsule::MaxData { maximum: limit });

        assert!(
            server.can_send_wt_data(stream_id, limit),
            "上限ちょうどの送信が許可されていない: limit={limit}"
        );
        if limit < u64::MAX {
            assert!(
                !server.can_send_wt_data(stream_id, limit + 1),
                "上限を超える送信が許可されている: limit={limit}"
            );
        }

        // 上限まで送ると残りは 0 になる
        assert!(server.wt_data_sent(stream_id, limit));
        assert!(
            !server.can_send_wt_data(stream_id, 1),
            "上限まで送信した後に追加送信が許可されている: limit={limit}"
        );
        Ok(())
    })?;
    Ok(())
}

/// Property: 広告された上限まで開設でき、超えると開設できない
#[test]
fn prop_stream_open_boundary() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WT_FLOW_CONTROL_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(64, |ctx| {
        let (mut server, stream_id) = established_server();
        // 開設本数を抑えるため上限は小さく取る
        let limit = sample_varint_raw_in(ctx, 1..=50);
        feed_capsule(
            &mut server,
            stream_id,
            &Capsule::MaxStreams {
                bidirectional: true,
                maximum: limit,
            },
        );

        // 上限に達するまで開設できる (発行済みの本数は 0)
        for opened in 0..limit {
            assert!(
                server.wt_stream_opened(stream_id, true),
                "上限内の開設が拒否された: opened={opened} limit={limit}"
            );
        }
        assert!(
            !server.wt_stream_opened(stream_id, true),
            "上限を超える開設が許可されている: limit={limit}"
        );
        Ok(())
    })?;
    Ok(())
}

/// Property: 上限に達した開設試行は WT_STREAMS_BLOCKED を 1 回だけ生成する
#[test]
fn prop_streams_blocked_dedup() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WT_FLOW_CONTROL_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(64, |ctx| {
        let (mut server, stream_id) = established_server();
        let limit = sample_varint_raw_in(ctx, 1..=50);
        feed_capsule(
            &mut server,
            stream_id,
            &Capsule::MaxStreams {
                bidirectional: true,
                maximum: limit,
            },
        );
        for _ in 0..limit {
            assert!(server.wt_stream_opened(stream_id, true));
        }

        // 同じ上限に対して複数回失敗しても WT_STREAMS_BLOCKED は 1 件
        let attempts = noprop::sample_usize_in(ctx, 1..=5);
        for _ in 0..attempts {
            assert!(!server.wt_stream_opened(stream_id, true));
        }
        let capsules = server.take_wt_flow_control_capsules(stream_id);
        let blocked_count = capsules
            .iter()
            .filter(|c| {
                matches!(
                    c,
                    Capsule::StreamsBlocked {
                        bidirectional: true,
                        ..
                    }
                )
            })
            .count();
        assert!(
            blocked_count <= 1,
            "WT_STREAMS_BLOCKED が重複生成されている: count={blocked_count} attempts={attempts}"
        );
        Ok(())
    })?;
    Ok(())
}

/// Property: 上限に達したデータ送信試行は WT_DATA_BLOCKED を 1 回だけ生成する
#[test]
fn prop_data_blocked_dedup() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WT_FLOW_CONTROL_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(64, |ctx| {
        let (mut server, stream_id) = established_server();
        let limit = sample_varint_raw_in(ctx, 1..=1_000_000);
        feed_capsule(&mut server, stream_id, &Capsule::MaxData { maximum: limit });
        assert!(server.wt_data_sent(stream_id, limit));

        let attempts = noprop::sample_usize_in(ctx, 1..=5);
        for _ in 0..attempts {
            assert!(!server.wt_data_sent(stream_id, 1));
        }
        let capsules = server.take_wt_flow_control_capsules(stream_id);
        let blocked_count = capsules
            .iter()
            .filter(|c| matches!(c, Capsule::DataBlocked { .. }))
            .count();
        assert!(
            blocked_count <= 1,
            "WT_DATA_BLOCKED が重複生成されている: count={blocked_count} attempts={attempts}"
        );
        Ok(())
    })?;
    Ok(())
}

/// Property: 上限を増やすと BLOCKED 状態がリセットされ、再び送信できる
#[test]
fn prop_limit_increase_resets_blocked() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WT_FLOW_CONTROL_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(64, |ctx| {
        let (mut server, stream_id) = established_server();
        let first = sample_varint_raw_in(ctx, 1..=1_000_000);
        feed_capsule(&mut server, stream_id, &Capsule::MaxData { maximum: first });
        assert!(server.wt_data_sent(stream_id, first));
        assert!(!server.wt_data_sent(stream_id, 1));
        let _ = server.take_wt_flow_control_capsules(stream_id);

        // 上限を増やすと再び送信できる
        let second = first + sample_varint_raw_in(ctx, 1..=1_000_000);
        feed_capsule(
            &mut server,
            stream_id,
            &Capsule::MaxData { maximum: second },
        );
        let extra = sample_varint_raw_in(ctx, 1..=(second - first));
        assert!(
            server.wt_data_sent(stream_id, extra),
            "上限増加後の送信が拒否された: extra={extra}"
        );
        // 増加後は BLOCKED が再生成され得る (リセットされているため)
        assert!(!server.wt_data_sent(stream_id, second - first - extra + 1));
        let capsules = server.take_wt_flow_control_capsules(stream_id);
        assert!(
            capsules
                .iter()
                .any(|c| matches!(c, Capsule::DataBlocked { .. })),
            "上限増加後に WT_DATA_BLOCKED が再生成されていない"
        );
        Ok(())
    })?;
    Ok(())
}

/// Property: フロー制御カプセルの取り出しは冪等 (2 回目は空)
#[test]
fn prop_take_capsules_is_idempotent() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WT_FLOW_CONTROL_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(64, |ctx| {
        let (mut server, stream_id) = established_server();
        // セッション確立直後の初期カプセルは 3 件 (Safari 形クライアント)
        let first = server.take_wt_flow_control_capsules(stream_id);
        assert_eq!(first.len(), 3, "初期カプセルが 3 件でない: {first:?}");
        let second = server.take_wt_flow_control_capsules(stream_id);
        assert!(second.is_empty(), "2 回目が空でない: {second:?}");

        // 追加の広告をしてもう一度取り出せる
        feed_capsule(
            &mut server,
            stream_id,
            &Capsule::MaxData {
                maximum: sample_varint_raw_in(ctx, 1..=1_000_000),
            },
        );
        // 上限の更新自体はカプセルを生成しない (BLOCKED ではないため)
        assert!(server.take_wt_flow_control_capsules(stream_id).is_empty());
        Ok(())
    })?;
    Ok(())
}

/// Property: WT_STREAMS_BLOCKED の maximum は常に 2^60 以下
#[test]
fn prop_streams_blocked_within_limit() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_WT_FLOW_CONTROL_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(64, |ctx| {
        let (mut server, stream_id) = established_server();
        let limit = sample_varint_raw_in(ctx, 1..=1_000);
        feed_capsule(
            &mut server,
            stream_id,
            &Capsule::MaxStreams {
                bidirectional: false,
                maximum: limit,
            },
        );
        for _ in 0..limit {
            assert!(server.wt_stream_opened(stream_id, false));
        }
        assert!(!server.wt_stream_opened(stream_id, false));

        for capsule in server.take_wt_flow_control_capsules(stream_id) {
            if let Capsule::StreamsBlocked { maximum, .. } = capsule {
                assert!(
                    maximum <= MAX_STREAMS_LIMIT,
                    "WT_STREAMS_BLOCKED の maximum が上限を超えている: {maximum}"
                );
            }
        }
        Ok(())
    })?;
    Ok(())
}
