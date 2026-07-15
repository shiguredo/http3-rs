//! Sans I/O 統合テスト: WebTransport フロー制御の client/server シミュレーション
//!
//! 2 つの Session を使い、SETTINGS 交換からストリーム open/close、
//! WT_MAX_STREAMS カプセル送受信までの一連のフローを検証する。

use shiguredo_http3::webtransport::{Capsule, CapsuleProcessError, FlowControlLimits, Session};

/// ヘルパー: セッション間でカプセルを転送する
///
/// sender の pending_capsules を取り出して receiver の process_capsule に渡す。
fn transfer_capsules(
    sender: &mut Session,
    receiver: &mut Session,
) -> Result<Vec<Capsule>, CapsuleProcessError> {
    let capsules = sender.take_pending_capsules();
    for capsule in &capsules {
        receiver.process_capsule(capsule)?;
    }
    Ok(capsules)
}

/// クライアントがストリームを開き、閉じた後にサーバーが WT_MAX_STREAMS を送信し、
/// クライアントが追加ストリームを開けるようになるフロー
#[test]
fn test_stream_flow_control_full_cycle() {
    // --- セットアップ ---
    let mut server = Session::new(0);
    server.set_established();
    // サーバーは単方向ストリームを同時 4 本まで許可
    server.initialize_local_limits(FlowControlLimits {
        max_streams_uni: 4,
        max_streams_bidi: 0,
        max_data: 0,
    });

    let mut client = Session::new(0);
    client.set_established();
    // クライアントはサーバーの SETTINGS から remote_limits を設定
    client.remote_limits_mut().max_streams_uni = 4;

    // --- Phase 1: クライアントが 4 本のストリームを開く ---
    for _ in 0..4 {
        assert!(
            client.try_open_stream(false),
            "should be able to open stream"
        );
    }
    // 5 本目はブロック
    assert!(!client.try_open_stream(false), "should be blocked at limit");

    // WT_STREAMS_BLOCKED カプセルが生成されたはず
    let capsules = transfer_capsules(&mut client, &mut server).expect("test must succeed");
    assert!(
        capsules.iter().any(|c| matches!(
            c,
            Capsule::StreamsBlocked {
                bidirectional: false,
                ..
            }
        )),
        "WT_STREAMS_BLOCKED should be generated"
    );

    // --- Phase 2: サーバー側でストリーム受信を記録 ---
    for _ in 0..4 {
        assert!(server.check_received_stream(false));
        server.add_received_stream(false);
    }

    // --- Phase 3: ストリームが完全に閉じたことをサーバーに通知 ---
    // 3 本閉じる → しきい値 (4/2=2) を下回る → WT_MAX_STREAMS が生成される
    for _ in 0..3 {
        server.on_remote_stream_closed(false);
    }

    // サーバーの pending_capsules をクライアントに転送
    let capsules = transfer_capsules(&mut server, &mut client).expect("test must succeed");
    let max_streams_capsules: Vec<_> = capsules
        .iter()
        .filter(|c| {
            matches!(
                c,
                Capsule::MaxStreams {
                    bidirectional: false,
                    ..
                }
            )
        })
        .collect();
    assert!(
        !max_streams_capsules.is_empty(),
        "WT_MAX_STREAMS should be generated after stream closure"
    );

    // --- Phase 4: クライアントが追加ストリームを開ける ---
    assert!(
        client.can_create_unidirectional_stream(),
        "client should be able to open more streams after receiving WT_MAX_STREAMS"
    );
    assert!(
        client.try_open_stream(false),
        "client should succeed in opening stream"
    );
}

/// 双方向ストリームのフロー制御サイクル
#[test]
fn test_bidi_stream_flow_control_cycle() {
    let mut server = Session::new(0);
    server.set_established();
    server.initialize_local_limits(FlowControlLimits {
        max_streams_uni: 0,
        max_streams_bidi: 2,
        max_data: 0,
    });

    let mut client = Session::new(0);
    client.set_established();
    client.remote_limits_mut().max_streams_bidi = 2;

    // 2 本開いてブロック
    assert!(client.try_open_stream(true));
    assert!(client.try_open_stream(true));
    assert!(!client.try_open_stream(true));

    // サーバー側で受信を記録
    server.add_received_stream(true);
    server.add_received_stream(true);

    // 2 本とも閉じる
    server.on_remote_stream_closed(true);
    server.on_remote_stream_closed(true);

    // カプセル転送
    let capsules = transfer_capsules(&mut server, &mut client).expect("test must succeed");
    assert!(
        capsules.iter().any(|c| matches!(
            c,
            Capsule::MaxStreams {
                bidirectional: true,
                ..
            }
        )),
        "WT_MAX_STREAMS (bidi) should be generated"
    );

    // 新しいストリームを開ける
    assert!(client.try_open_stream(true));
}

/// データフロー制御の一連のサイクル
#[test]
fn test_data_flow_control_full_cycle() {
    let mut server = Session::new(0);
    server.set_established();
    server.initialize_local_limits(FlowControlLimits {
        max_streams_uni: 0,
        max_streams_bidi: 0,
        max_data: 1000,
    });

    let mut client = Session::new(0);
    client.set_established();
    client.remote_limits_mut().max_data = 1000;

    // --- Phase 1: クライアントが 1000 バイト送信 ---
    assert!(client.try_send_data(600));
    assert!(client.try_send_data(400));
    // これ以上は送れない
    assert!(!client.try_send_data(1));

    // WT_DATA_BLOCKED が生成されたはず
    let capsules = transfer_capsules(&mut client, &mut server).expect("test must succeed");
    assert!(
        capsules
            .iter()
            .any(|c| matches!(c, Capsule::DataBlocked { .. })),
        "WT_DATA_BLOCKED should be generated"
    );

    // --- Phase 2: サーバー側でデータ受信を記録 ---
    assert!(server.check_received_data(1000));
    server.add_received_data(1000);

    // --- Phase 3: サーバーがデータを消費 → WT_MAX_DATA が生成される ---
    server.on_data_consumed(800);

    let capsules = transfer_capsules(&mut server, &mut client).expect("test must succeed");
    let max_data_capsules: Vec<_> = capsules
        .iter()
        .filter(|c| matches!(c, Capsule::MaxData { .. }))
        .collect();
    assert!(
        !max_data_capsules.is_empty(),
        "WT_MAX_DATA should be generated after data consumption"
    );

    // --- Phase 4: クライアントが追加データを送れる ---
    assert!(client.can_send_data(1));
    assert!(client.try_send_data(100));
}

/// 複数サイクルにわたるストリームフロー制御
///
/// ストリームを開く → 閉じる → WT_MAX_STREAMS 受信 → また開く を繰り返す。
#[test]
fn test_stream_flow_control_multiple_cycles() {
    let concurrent = 2u64;

    let mut server = Session::new(0);
    server.set_established();
    server.initialize_local_limits(FlowControlLimits {
        max_streams_uni: concurrent,
        max_streams_bidi: 0,
        max_data: 0,
    });

    let mut client = Session::new(0);
    client.set_established();
    client.remote_limits_mut().max_streams_uni = concurrent;

    for cycle in 0..5 {
        // concurrent 本開く
        for _ in 0..concurrent {
            assert!(
                client.try_open_stream(false),
                "cycle {}: should open stream",
                cycle
            );
        }
        // ブロック
        assert!(
            !client.try_open_stream(false),
            "cycle {}: should be blocked",
            cycle
        );

        // サーバー側受信記録
        for _ in 0..concurrent {
            server.add_received_stream(false);
        }

        // 全部閉じる
        for _ in 0..concurrent {
            server.on_remote_stream_closed(false);
        }

        // カプセル転送
        transfer_capsules(&mut server, &mut client).expect("test must succeed");
        // STREAMS_BLOCKED も転送
        transfer_capsules(&mut client, &mut server).expect("test must succeed");
    }

    // 5 サイクル後、合計 10 本のストリームを開いているはず
    assert_eq!(client.flow_state().streams_uni_opened, concurrent * 5);
}

/// ストリームとデータのフロー制御を同時に使用するシナリオ
#[test]
fn test_mixed_stream_and_data_flow_control() {
    let mut server = Session::new(0);
    server.set_established();
    server.initialize_local_limits(FlowControlLimits {
        max_streams_uni: 3,
        max_streams_bidi: 2,
        max_data: 500,
    });

    let mut client = Session::new(0);
    client.set_established();
    client.remote_limits_mut().max_streams_uni = 3;
    client.remote_limits_mut().max_streams_bidi = 2;
    client.remote_limits_mut().max_data = 500;

    // ストリームを開く
    assert!(client.try_open_stream(false)); // uni 1
    assert!(client.try_open_stream(true)); // bidi 1
    assert!(client.try_open_stream(false)); // uni 2

    // データを送信
    assert!(client.try_send_data(200));
    assert!(client.try_send_data(300));
    assert!(!client.try_send_data(1)); // データブロック

    // サーバー側で受信
    server.add_received_stream(false);
    server.add_received_stream(true);
    server.add_received_stream(false);
    server.add_received_data(500);

    // サーバー側でデータ消費とストリーム close
    server.on_data_consumed(500);
    server.on_remote_stream_closed(false);
    server.on_remote_stream_closed(false);

    // カプセル転送
    let capsules = transfer_capsules(&mut server, &mut client).expect("test must succeed");

    // WT_MAX_DATA と WT_MAX_STREAMS (uni) が生成されたはず
    assert!(
        capsules
            .iter()
            .any(|c| matches!(c, Capsule::MaxData { .. })),
        "WT_MAX_DATA should be generated"
    );
    assert!(
        capsules.iter().any(|c| matches!(
            c,
            Capsule::MaxStreams {
                bidirectional: false,
                ..
            }
        )),
        "WT_MAX_STREAMS (uni) should be generated"
    );

    // クライアントは追加データとストリームを送れる
    assert!(client.try_send_data(100));
    assert!(client.try_open_stream(false));
}

/// フロー制御無効時は WT_MAX_STREAMS / WT_STREAMS_BLOCKED が生成されない
#[test]
fn test_flow_control_disabled_no_capsules() {
    let mut server = Session::new(0);
    server.set_established();
    server.set_flow_control_enabled(false);
    server.initialize_local_limits(FlowControlLimits {
        max_streams_uni: 4,
        max_streams_bidi: 0,
        max_data: 0,
    });

    let mut client = Session::new(0);
    client.set_established();
    client.set_flow_control_enabled(false);
    client.remote_limits_mut().max_streams_uni = 0; // 制限 0 でもブロックしない想定

    // サーバー側: ストリーム close してもカプセルは生成されない
    for _ in 0..4 {
        server.add_received_stream(false);
    }
    for _ in 0..4 {
        server.on_remote_stream_closed(false);
    }
    assert!(
        server.take_pending_capsules().is_empty(),
        "no capsules should be generated when flow control is disabled"
    );

    // クライアント側: ブロックしても STREAMS_BLOCKED は生成されない
    assert!(!client.try_open_stream(false)); // 制限 0 なのでブロック
    assert!(
        client.take_pending_capsules().is_empty(),
        "no STREAMS_BLOCKED when flow control is disabled"
    );
}

/// STREAMS_BLOCKED の重複防止と MAX_STREAMS 受信後のリセット
#[test]
fn test_streams_blocked_dedup_and_reset() {
    let mut client = Session::new(0);
    client.set_established();
    client.remote_limits_mut().max_streams_uni = 1;

    // 1 本開いてブロック
    assert!(client.try_open_stream(false));
    assert!(!client.try_open_stream(false)); // STREAMS_BLOCKED 生成
    assert!(!client.try_open_stream(false)); // 重複送信なし

    let capsules = client.take_pending_capsules();
    let blocked_count = capsules
        .iter()
        .filter(|c| matches!(c, Capsule::StreamsBlocked { .. }))
        .count();
    assert_eq!(blocked_count, 1, "only one STREAMS_BLOCKED per maximum");

    // WT_MAX_STREAMS を受信 → BLOCKED 状態リセット
    client
        .process_capsule(&Capsule::MaxStreams {
            bidirectional: false,
            maximum: 3,
        })
        .expect("test must succeed");

    // 2 本開ける
    assert!(client.try_open_stream(false));
    assert!(client.try_open_stream(false));
    // 再度ブロック → 新しい STREAMS_BLOCKED
    assert!(!client.try_open_stream(false));

    let capsules = client.take_pending_capsules();
    let blocked_count = capsules
        .iter()
        .filter(|c| matches!(c, Capsule::StreamsBlocked { .. }))
        .count();
    assert_eq!(
        blocked_count, 1,
        "new STREAMS_BLOCKED after MAX_STREAMS reset"
    );
    // maximum が新しい制限値
    if let Some(Capsule::StreamsBlocked { maximum, .. }) = capsules
        .iter()
        .find(|c| matches!(c, Capsule::StreamsBlocked { .. }))
    {
        assert_eq!(*maximum, 3);
    }
}
