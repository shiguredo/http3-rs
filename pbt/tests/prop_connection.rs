//! Property-Based Testing for HTTP/3 Connection (RFC 9114)
//!
//! get_stream_data のループ終了性とデータ完全性、および 0-RTT 再開時の
//! WebTransport SETTINGS の減少検出が選択したドラフト版に依存することを検証する。

use pbt::strategies::sample_len;
use shiguredo_http3::frame::encode_frame;
use shiguredo_http3::webtransport::{DraftVersion, Settings as WtSettings};
use shiguredo_http3::{
    ClientConnection, Error, ErrorCode, Frame, Header, Settings, SettingsPayload, VarInt,
};

/// リクエスト送信後の take_stream_data ループ用ヘルパー
///
/// take_stream_data ループを実行し、収集したデータと反復回数を返す。
/// ループはデータ交付 → FIN 交付 (`(空, fin=true)`) → None の順で終了する。
/// FIN の 1 回交付は決定的テスト (tests/test_connection.rs) で担保し、
/// この PBT はループの終了性とデータ完全性だけを検証する。
fn drain_stream_data(
    conn: &mut ClientConnection,
    stream_id: u64,
    max_iterations: usize,
) -> (Vec<u8>, usize) {
    let mut collected = Vec::new();
    let mut iterations = 0;

    while let Some((chunk, _fin)) = conn.take_stream_data(stream_id) {
        collected.extend_from_slice(&chunk);
        iterations += 1;
        if iterations >= max_iterations {
            break;
        }
    }

    (collected, iterations)
}

// =============================================================================
// get_stream_data Termination Properties
// =============================================================================

/// Property: 任意サイズのボディで get_stream_data ループが有限回で終了する
#[test]
fn prop_get_stream_data_terminates() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_CONNECTION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let body_len = sample_len(ctx, 0..=8191);
        let body = vec![0xABu8; body_len];

        let mut client = ClientConnection::with_default_settings();
        client.set_control_stream_id(2).expect("test must succeed");

        let request_headers = vec![
            Header::new(b":method", b"POST").expect("test must succeed"),
            Header::new(b":path", b"/").expect("test must succeed"),
            Header::new(b":scheme", b"https").expect("test must succeed"),
            Header::new(b":authority", b"example.com").expect("test must succeed"),
        ];

        // ボディ付きリクエストを送信 (fin=false でヘッダーのみ先に送信)
        let stream_id = client
            .send_request(&request_headers, false)
            .expect("test must succeed");
        client
            .send_body(stream_id, &body, true)
            .expect("test must succeed");

        let (_collected, iterations) = drain_stream_data(&mut client, stream_id, 10);

        assert!(
            iterations < 10,
            "get_stream_data ループが 10 回以内に終了しなかった (iterations={})",
            iterations
        );
        Ok(())
    })?;
    Ok(())
}

/// Property: ループで収集したデータ長が元のフレームデータ長と一致する (データ欠損なし)
#[test]
fn prop_get_stream_data_all_data_collected() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_CONNECTION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    // sample_len は 0/1/上限を 1/5 で選ぶ。3 境界なら空の p=1/15、
    // 256 ケースでの見逃しは (14/15)^256 ≈ 2.4e-8。
    let empty_body = std::cell::Cell::new(0usize);
    let nonempty_body = std::cell::Cell::new(0usize);
    runner.run(256, |ctx| {
        let body_len = sample_len(ctx, 0..=8191);
        let body = vec![0xCDu8; body_len];

        let mut client = ClientConnection::with_default_settings();
        client.set_control_stream_id(2).expect("test must succeed");

        let request_headers = vec![
            Header::new(b":method", b"POST").expect("test must succeed"),
            Header::new(b":path", b"/").expect("test must succeed"),
            Header::new(b":scheme", b"https").expect("test must succeed"),
            Header::new(b":authority", b"example.com").expect("test must succeed"),
        ];

        let stream_id = client
            .send_request(&request_headers, false)
            .expect("test must succeed");
        client
            .send_body(stream_id, &body, true)
            .expect("test must succeed");

        let (collected, _iterations) = drain_stream_data(&mut client, stream_id, 10);

        // 収集データにはフレームヘッダーも含まれるため、ボディ長以上であることを検証
        assert!(
            collected.len() >= body_len,
            "収集データ長 {} がボディ長 {} より小さい",
            collected.len(),
            body_len
        );
        if body_len == 0 {
            empty_body.set(empty_body.get() + 1);
        } else {
            nonempty_body.set(nonempty_body.get() + 1);
        }
        Ok(())
    })?;
    assert!(empty_body.get() > 0, "空ボディを未到達\n{runner}");
    assert!(nonempty_body.get() > 0, "非空ボディを未到達\n{runner}");
    Ok(())
}

/// Property: FIN 交付後に get_stream_data が None を返す
///
/// drain_stream_data はデータ交付 → FIN 交付 → None の順でループを抜けるため、
/// ここでの検証対象は「FIN 交付後の None」である。
/// (FIN はデータ全消費後の追加呼び出しで交付されるため、全データ消費直後の
///  get_stream_data は (空, fin=true) を返す)
#[test]
fn prop_get_stream_data_returns_none_after_consume() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_CONNECTION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let body_len = sample_len(ctx, 0..=8191);
        let body = vec![0xEFu8; body_len];

        let mut client = ClientConnection::with_default_settings();
        client.set_control_stream_id(2).expect("test must succeed");

        let request_headers = vec![
            Header::new(b":method", b"POST").expect("test must succeed"),
            Header::new(b":path", b"/").expect("test must succeed"),
            Header::new(b":scheme", b"https").expect("test must succeed"),
            Header::new(b":authority", b"example.com").expect("test must succeed"),
        ];

        let stream_id = client
            .send_request(&request_headers, false)
            .expect("test must succeed");
        client
            .send_body(stream_id, &body, true)
            .expect("test must succeed");

        // データと FIN をすべて消費
        let (_collected, _iterations) = drain_stream_data(&mut client, stream_id, 10);

        // FIN 交付後は None が返ることを検証
        let result = client.get_stream_data(stream_id);
        assert!(
            result.is_none(),
            "FIN 交付後に get_stream_data が None ではなく {:?} を返した",
            result.map(|(d, f)| (d.len(), f))
        );
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// WebTransport 0-RTT 再開時の WT SETTINGS 減少検出
// (draft-ietf-webtrans-http3-16 Section 3.2,
//  draft-ietf-webtrans-http3-17 Section 3.2)
// =============================================================================

/// 指定した版と初期上限を持つ WebTransport SETTINGS を構築する
///
/// テストで扱う版は、旧コードポイントの draft-14 / draft-15 / 16 と
/// 新コードポイントの draft-17 である。
fn wt_settings(draft: DraftVersion, uni: u64, bidi: u64, data: u64) -> WtSettings {
    let v = |x: u64| VarInt::new(x).expect("生成値が VarInt の範囲内であること");
    let settings = WtSettings::new()
        .wt_initial_max_streams_uni(v(uni))
        .wt_initial_max_streams_bidi(v(bidi))
        .wt_initial_max_data(v(data));
    match draft {
        DraftVersion::Draft17 => settings.wt_enabled_draft17(VarInt::from_static(1)),
        DraftVersion::Draft14 => settings.wt_max_sessions_draft14(VarInt::from_static(1)),
        _ => settings.wt_enabled(VarInt::from_static(1)),
    }
}

/// SETTINGS フレームを制御ストリームのバイト列にする
fn settings_control_stream(settings: &Settings) -> Vec<u8> {
    let payload = SettingsPayload::from_settings(settings);
    let mut buf = vec![0u8; 4096];
    let len = encode_frame(&mut buf, &Frame::Settings(payload))
        .expect("テスト用の SETTINGS フレームエンコードに成功すること");
    buf.truncate(len);

    // 単方向ストリームの先頭に制御ストリームタイプ (0x00) を置く
    let mut stream = vec![0x00];
    stream.extend_from_slice(&buf);
    stream
}

/// Property: 0-RTT 再開時の WT SETTINGS の減少検出は選択版が旧コードポイントの
/// ときだけ働き、draft-17 では前回値と現在値の大小関係によらず検出しない
///
/// 前回値と現在値の大小関係、および選択される版の組み合わせをランダムに生成し、
/// 次の対応を検証する:
/// - draft-14 / 15 / 16: 現在値のいずれかが前回値より小さければ H3_SETTINGS_ERROR
/// - draft-17: 大小関係によらず受理し、現在の SETTINGS の値だけを使う
#[test]
fn prop_previous_wt_settings_reduction_only_for_old_draft() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_CONNECTION_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        // 両端が同じコードポイントを広告するため、この版が接続の選択結果になる
        let draft = noprop::sample_choice(
            ctx,
            &[
                DraftVersion::Draft14,
                DraftVersion::Draft15,
                DraftVersion::Draft17,
            ],
        );
        // 前回接続の版は現在の版と独立に選ぶ。前回値のコードポイントが
        // 現在の選択版の判定に影響しないことを確かめる。
        let previous_draft =
            noprop::sample_choice(ctx, &[DraftVersion::Draft15, DraftVersion::Draft17]);
        let prev_uni = noprop::sample_u64_in(ctx, 0..=1000);
        let prev_bidi = noprop::sample_u64_in(ctx, 0..=1000);
        let prev_data = noprop::sample_u64_in(ctx, 0..=100_000);

        // 現在値は、前回値との大小関係 (省略 / 同値 / 減少 / 増加) を必ず含む
        // 候補から選ぶ。省略はデフォルト値 0 として扱われる。残りの確率で
        // 前回値と無関係な値も選び、大小関係の組み合わせを広げる。
        let mut new_value = |prev: u64, max: u64| {
            let random = noprop::sample_u64_in(ctx, 0..=max);
            noprop::sample_choice(
                ctx,
                &[
                    0,
                    prev,
                    prev.saturating_sub(1),
                    prev.saturating_add(1),
                    random,
                ],
            )
        };
        let new_uni = new_value(prev_uni, 1000);
        let new_bidi = new_value(prev_bidi, 1000);
        let new_data = new_value(prev_data, 100_000);

        let local_wt = wt_settings(draft, 0, 0, 0);
        let mut client =
            ClientConnection::new(Settings::new().enable_webtransport_client(local_wt));
        client
            .set_control_stream_id(2)
            .expect("テスト用の制御ストリーム設定に成功すること");
        client.set_previous_wt_settings(wt_settings(
            previous_draft,
            prev_uni,
            prev_bidi,
            prev_data,
        ));

        let peer_wt = wt_settings(draft, new_uni, new_bidi, new_data);
        let peer_settings = Settings::new().enable_webtransport_server(peer_wt);
        let result = client.feed_stream(3, &settings_control_stream(&peer_settings), false);

        let decreased = new_uni < prev_uni || new_bidi < prev_bidi || new_data < prev_data;
        // draft-17 だけが前回値を破棄する。旧コードポイントの版は前回の版に
        // かかわらず減少検出の対象になる
        let expect_error = draft != DraftVersion::Draft17 && decreased;
        if expect_error {
            assert!(
                matches!(
                    result,
                    Err(Error::ConnectionError(ErrorCode::SettingsError))
                ),
                "旧コードポイントの版で初期上限が減少した場合は H3_SETTINGS_ERROR になること: {result:?}"
            );
        } else {
            assert!(
                result.is_ok(),
                "選択版 {draft:?} で初期上限の減少を検出しないこと: {result:?}"
            );
            assert_eq!(
                client.negotiated_wt_draft_version(),
                Some(draft),
                "現在の SETTINGS のコードポイントから選択版が決まること"
            );
        }
        Ok(())
    })?;
    Ok(())
}
