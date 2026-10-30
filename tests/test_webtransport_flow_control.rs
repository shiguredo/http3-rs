//! Sans I/O 統合テスト: WebTransport フロー制御の client/server シミュレーション
//!
//! 2 つの Session を使い、SETTINGS 交換からストリーム open/close、
//! WT_MAX_STREAMS カプセル送受信までの一連のフローを検証する。

use shiguredo_http3::webtransport::{
    Capsule, CapsuleProcessError, Error, ErrorCode, FlowControlLimits, Session,
};

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
    // クライアントはサーバーの SETTINGS から得た初期値を送信側上限に設定する
    client.initialize_remote_limits(FlowControlLimits {
        max_streams_uni: 4,
        ..FlowControlLimits::default()
    });
    client.set_established();

    // --- Phase 1: クライアントが 4 本のストリームを開く ---
    for _ in 0..4 {
        assert!(
            client.try_open_stream(false),
            "上限まではストリームを開設できること"
        );
    }
    // 5 本目はブロック
    assert!(
        !client.try_open_stream(false),
        "上限に達したらストリーム開設がブロックされること"
    );

    // WT_STREAMS_BLOCKED カプセルが生成されたはず
    let capsules =
        transfer_capsules(&mut client, &mut server).expect("テスト用のカプセル転送に成功すること");
    assert!(
        capsules.iter().any(|c| matches!(
            c,
            Capsule::StreamsBlocked {
                bidirectional: false,
                ..
            }
        )),
        "WT_STREAMS_BLOCKED が生成されること"
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
    let capsules =
        transfer_capsules(&mut server, &mut client).expect("テスト用のカプセル転送に成功すること");
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
        "ストリームクローズ後に WT_MAX_STREAMS が生成されること"
    );

    // --- Phase 4: クライアントが追加ストリームを開ける ---
    assert!(
        client.can_create_unidirectional_stream(),
        "WT_MAX_STREAMS 受信後にクライアントが追加ストリームを開設できること"
    );
    assert!(
        client.try_open_stream(false),
        "クライアントのストリーム開設が成功すること"
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
    client.initialize_remote_limits(FlowControlLimits {
        max_streams_bidi: 2,
        ..FlowControlLimits::default()
    });
    client.set_established();

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
    let capsules =
        transfer_capsules(&mut server, &mut client).expect("テスト用のカプセル転送に成功すること");
    assert!(
        capsules.iter().any(|c| matches!(
            c,
            Capsule::MaxStreams {
                bidirectional: true,
                ..
            }
        )),
        "WT_MAX_STREAMS (双方向) が生成されること"
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
    client.initialize_remote_limits(FlowControlLimits {
        max_data: 1000,
        ..FlowControlLimits::default()
    });
    client.set_established();

    // --- Phase 1: クライアントが 1000 バイト送信 ---
    assert!(client.try_send_data(600));
    assert!(client.try_send_data(400));
    // これ以上は送れない
    assert!(!client.try_send_data(1));

    // WT_DATA_BLOCKED が生成されたはず
    let capsules =
        transfer_capsules(&mut client, &mut server).expect("テスト用のカプセル転送に成功すること");
    assert!(
        capsules
            .iter()
            .any(|c| matches!(c, Capsule::DataBlocked { .. })),
        "WT_DATA_BLOCKED が生成されること"
    );

    // --- Phase 2: サーバー側でデータ受信を記録 ---
    assert!(server.check_received_data(1000));
    server.add_received_data(1000);

    // --- Phase 3: サーバーがデータを消費 → WT_MAX_DATA が生成される ---
    server.on_data_consumed(800);

    let capsules =
        transfer_capsules(&mut server, &mut client).expect("テスト用のカプセル転送に成功すること");
    let max_data_capsules: Vec<_> = capsules
        .iter()
        .filter(|c| matches!(c, Capsule::MaxData { .. }))
        .collect();
    assert!(
        !max_data_capsules.is_empty(),
        "データ消費後に WT_MAX_DATA が生成されること"
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
    client.initialize_remote_limits(FlowControlLimits {
        max_streams_uni: concurrent,
        ..FlowControlLimits::default()
    });
    client.set_established();

    for cycle in 0..5 {
        // concurrent 本開く
        for _ in 0..concurrent {
            assert!(
                client.try_open_stream(false),
                "サイクル {}: ストリームを開設できること",
                cycle
            );
        }
        // ブロック
        assert!(
            !client.try_open_stream(false),
            "サイクル {}: 上限でブロックされること",
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
        transfer_capsules(&mut server, &mut client).expect("テスト用のカプセル転送に成功すること");
        // STREAMS_BLOCKED も転送
        transfer_capsules(&mut client, &mut server).expect("テスト用のカプセル転送に成功すること");
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
    client.initialize_remote_limits(*server.local_limits());
    client.set_established();

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
    let capsules =
        transfer_capsules(&mut server, &mut client).expect("テスト用のカプセル転送に成功すること");

    // WT_MAX_DATA と WT_MAX_STREAMS (単方向) が生成されたはず
    assert!(
        capsules
            .iter()
            .any(|c| matches!(c, Capsule::MaxData { .. })),
        "WT_MAX_DATA が生成されること"
    );
    assert!(
        capsules.iter().any(|c| matches!(
            c,
            Capsule::MaxStreams {
                bidirectional: false,
                ..
            }
        )),
        "WT_MAX_STREAMS (単方向) が生成されること"
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
    // フロー制御無効時は SETTINGS 初期値を設定しない (既定値 0 のまま)
    client.initialize_remote_limits(FlowControlLimits::default());
    client.set_flow_control_enabled(false);
    client.set_established();

    // サーバー側: ストリーム close してもカプセルは生成されない
    for _ in 0..4 {
        server.add_received_stream(false);
    }
    for _ in 0..4 {
        server.on_remote_stream_closed(false);
    }
    assert!(
        server.take_pending_capsules().is_empty(),
        "フロー制御無効時はカプセルが生成されないこと"
    );

    // クライアント側: フロー制御無効時は制限 0 でも開設できる
    assert!(client.try_open_stream(false)); // フロー制御無効時は制限なし
    assert_eq!(client.flow_state().streams_uni_opened, 1);
    assert!(
        client.take_pending_capsules().is_empty(),
        "フロー制御無効時は STREAMS_BLOCKED が生成されないこと"
    );
}

/// STREAMS_BLOCKED の重複防止と MAX_STREAMS 受信後のリセット
#[test]
fn test_streams_blocked_dedup_and_reset() {
    let mut client = Session::new(0);
    client.initialize_remote_limits(FlowControlLimits {
        max_streams_uni: 1,
        ..FlowControlLimits::default()
    });
    client.set_established();

    // 1 本開いてブロック
    assert!(client.try_open_stream(false));
    assert!(!client.try_open_stream(false)); // STREAMS_BLOCKED 生成
    assert!(!client.try_open_stream(false)); // 重複送信なし

    let capsules = client.take_pending_capsules();
    let blocked_count = capsules
        .iter()
        .filter(|c| matches!(c, Capsule::StreamsBlocked { .. }))
        .count();
    assert_eq!(
        blocked_count, 1,
        "同じ maximum に対する STREAMS_BLOCKED は 1 回だけ送信されること"
    );

    // WT_MAX_STREAMS を受信 → BLOCKED 状態リセット
    client
        .process_capsule(&Capsule::MaxStreams {
            bidirectional: false,
            maximum: 3,
        })
        .expect("WT_MAX_STREAMS の受信に成功すること");

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
        "WT_MAX_STREAMS 受信後は新しい STREAMS_BLOCKED が 1 回だけ送信されること"
    );
    // maximum が新しい制限値
    if let Some(Capsule::StreamsBlocked { maximum, .. }) = capsules
        .iter()
        .find(|c| matches!(c, Capsule::StreamsBlocked { .. }))
    {
        assert_eq!(*maximum, 3);
    }
}

/// ピアの SETTINGS 初期値が送信側上限と単調性検証の基準になること
///
/// サーバーが SETTINGS で宣言した初期値をクライアントの
/// `initialize_remote_limits` に渡し、初期値を下回る
/// WT_MAX_STREAMS / WT_MAX_DATA がセッションエラーになることを確認する。
/// (draft-ietf-webtrans-http3-16 Section 5.5, 5.6.2, 5.6.4)
#[test]
fn test_peer_settings_initial_limits_are_monotonic_baseline() {
    let mut server = Session::new(0);
    server.initialize_local_limits(FlowControlLimits {
        max_streams_uni: 100,
        max_streams_bidi: 50,
        max_data: 8192,
    });
    server.set_established();

    let mut client = Session::new(0);
    // ピアの SETTINGS 初期値 (サーバーの local_limits) を送信側初期値として設定する
    client.initialize_remote_limits(*server.local_limits());
    client.set_established();

    // 送信側上限が SETTINGS 初期値に従うこと
    assert!(client.can_create_unidirectional_stream());
    assert!(client.can_create_bidirectional_stream());
    assert!(client.can_send_data(8192));
    assert!(!client.can_send_data(8193));

    // 初期値より小さい WT_MAX_STREAMS (単方向) は WT_FLOW_CONTROL_ERROR
    let result = client.process_capsule(&Capsule::MaxStreams {
        bidirectional: false,
        maximum: 50,
    });
    assert_eq!(
        result,
        Err(CapsuleProcessError::Session(Error::Protocol(
            ErrorCode::FlowControlError
        ))),
        "SETTINGS 初期値 (100) 未満の WT_MAX_STREAMS はセッションエラーになること"
    );

    // 初期値より小さい WT_MAX_DATA も WT_FLOW_CONTROL_ERROR
    let result = client.process_capsule(&Capsule::MaxData { maximum: 4096 });
    assert_eq!(
        result,
        Err(CapsuleProcessError::Session(Error::Protocol(
            ErrorCode::FlowControlError
        ))),
        "SETTINGS 初期値 (8192) 未満の WT_MAX_DATA はセッションエラーになること"
    );

    // 初期値より大きい値は受理される
    client
        .process_capsule(&Capsule::MaxStreams {
            bidirectional: false,
            maximum: 101,
        })
        .expect("SETTINGS 初期値より大きい WT_MAX_STREAMS は受理されること");
    client
        .process_capsule(&Capsule::MaxData { maximum: 8193 })
        .expect("SETTINGS 初期値より大きい WT_MAX_DATA は受理されること");
    assert_eq!(client.remote_limits().max_streams_uni, 101);
    assert_eq!(client.remote_limits().max_data, 8193);

    // 双方向ストリームの初期値も基準になっていること
    let result = client.process_capsule(&Capsule::MaxStreams {
        bidirectional: true,
        maximum: 50,
    });
    assert_eq!(
        result,
        Err(CapsuleProcessError::Session(Error::Protocol(
            ErrorCode::FlowControlError
        ))),
        "SETTINGS 初期値 (50) 以下の WT_MAX_STREAMS (双方向) はセッションエラーになること"
    );
}
