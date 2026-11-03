//! WebTransport SETTINGS の draft-17 コードポイント対応テスト
//!
//! draft-15 / 16 の `SETTINGS_WT_ENABLED` (0x2c7cf000) と draft-17 の
//! `SETTINGS_WT_ENABLED` (0x20b8933b) を区別して保持・検出できることを、
//! 制御ストリームの生バイト列から公開 API 経由で検証する
//! (draft-ietf-webtrans-http3-17 Section 3.1, 7.1, 9.2)。
//! 接続単位の版選択規則は本ファイルの対象外であり、CONNECT の受理・拒否は
//! tests/test_webtransport_draft_connect.rs で検証する。

use shiguredo_http3::webtransport::DraftVersion;
use shiguredo_http3::{ClientConnection, Error, ErrorCode, ServerConnection, Settings, VarInt};

/// draft-15 / 16 の SETTINGS_WT_ENABLED コードポイント
const WT_ENABLED_DRAFT15_16: u64 = 0x2c7cf000;
/// draft-17 の SETTINGS_WT_ENABLED コードポイント
const WT_ENABLED_DRAFT17: u64 = 0x20b8933b;

fn vi(value: u64) -> VarInt {
    VarInt::new(value).expect("テスト用の VarInt 値が範囲に収まること")
}

/// 制御ストリームの生バイト列 (制御ストリームタイプ + SETTINGS フレーム) を組み立てる
///
/// `Settings` を経由せず、制御ストリームタイプ (0x00)、SETTINGS フレームタイプ
/// (0x04)、ペイロード長、各エントリの `(ID, 値)` の VarInt を手で並べる。
/// これにより、公開 API の SETTINGS デコード経路を wire 表現から通しで検証する。
fn build_control_stream_with_settings(entries: &[(u64, u64)]) -> Vec<u8> {
    let mut payload = Vec::new();
    for (id, value) in entries {
        shiguredo_http3::varint::encode_into_vec(
            &mut payload,
            VarInt::new(*id).expect("テスト用の SETTINGS ID が VarInt 範囲に収まること"),
        );
        shiguredo_http3::varint::encode_into_vec(
            &mut payload,
            VarInt::new(*value).expect("テスト用の SETTINGS 値が VarInt 範囲に収まること"),
        );
    }

    let mut frame = Vec::new();
    // SETTINGS フレームタイプは 0x04 (RFC 9114 Section 7.2.4)
    shiguredo_http3::varint::encode_into_vec(&mut frame, VarInt::from_static(0x04));
    shiguredo_http3::varint::encode_into_vec(
        &mut frame,
        VarInt::new(payload.len() as u64)
            .expect("テスト用の SETTINGS ペイロード長が VarInt 範囲に収まること"),
    );
    frame.extend_from_slice(&payload);

    let mut control = vec![0x00];
    control.extend_from_slice(&frame);
    control
}

/// サーバー SETTINGS を受信済みのクライアントを作成する
fn make_client_with_server_settings(entries: &[(u64, u64)]) -> ClientConnection {
    let mut client = ClientConnection::new(Settings::new());
    client
        .set_control_stream_id(2)
        .expect("テスト用の制御ストリーム設定に成功すること");
    let control = build_control_stream_with_settings(entries);
    client
        .feed_stream(3, &control, false)
        .expect("テスト用のサーバー SETTINGS 受信に成功すること");
    while let Some(_ev) = client
        .poll_event()
        .expect("テスト用のイベント取得に成功すること")
    {}
    client
}

/// クライアント SETTINGS を受信済みのサーバーを作成する
fn make_server_with_client_settings(entries: &[(u64, u64)]) -> ServerConnection {
    let mut server = ServerConnection::new(Settings::new());
    server
        .set_control_stream_id(3)
        .expect("テスト用の制御ストリーム設定に成功すること");
    let control = build_control_stream_with_settings(entries);
    server
        .feed_stream(2, &control, false)
        .expect("テスト用のクライアント SETTINGS 受信に成功すること");
    while let Some(_ev) = server
        .poll_event()
        .expect("テスト用のイベント取得に成功すること")
    {}
    server
}

#[test]
fn old_codepoint_only_is_detected_as_draft15() {
    // 旧コードポイントのみを広告するサーバーは draft-15 / 16 として検出する
    let client = make_client_with_server_settings(&[(WT_ENABLED_DRAFT15_16, 1)]);

    let peer = client
        .peer_settings()
        .expect("ピア SETTINGS が保持されること");
    let peer_wt = peer
        .wt_settings
        .as_ref()
        .expect("WebTransport SETTINGS が保持されること");
    assert_eq!(peer_wt.wt_enabled, vi(1));
    assert_eq!(peer_wt.wt_enabled_draft17, VarInt::ZERO);
    assert_eq!(
        peer.webtransport_draft_pattern(),
        Some(DraftVersion::Draft15)
    );
}

#[test]
fn new_codepoint_only_is_detected_as_draft17() {
    // 新コードポイントのみを広告するサーバーは draft-17 として検出する
    let client = make_client_with_server_settings(&[(WT_ENABLED_DRAFT17, 1)]);

    let peer = client
        .peer_settings()
        .expect("ピア SETTINGS が保持されること");
    let peer_wt = peer
        .wt_settings
        .as_ref()
        .expect("WebTransport SETTINGS が保持されること");
    assert_eq!(peer_wt.wt_enabled, VarInt::ZERO);
    assert_eq!(peer_wt.wt_enabled_draft17, vi(1));
    assert!(peer.is_webtransport_enabled());
    assert_eq!(
        peer.webtransport_draft_pattern(),
        Some(DraftVersion::Draft17)
    );
}

#[test]
fn new_codepoint_zero_is_not_enabled() {
    // 新コードポイントの値 0 は WebTransport 無効を意味し、検出しない
    // (draft-ietf-webtrans-http3-17 Section 9.2 の Default: 0)
    let client = make_client_with_server_settings(&[(WT_ENABLED_DRAFT17, 0)]);

    let peer = client
        .peer_settings()
        .expect("ピア SETTINGS が保持されること");
    let peer_wt = peer
        .wt_settings
        .as_ref()
        .expect("値 0 の WebTransport SETTINGS も保持されること");
    assert_eq!(peer_wt.wt_enabled_draft17, VarInt::ZERO);
    assert!(!peer.is_webtransport_enabled());
    assert_eq!(peer.webtransport_draft_pattern(), None);
}

#[test]
fn server_retains_old_and_new_codepoints_from_client() {
    // 新旧コードポイントを同時に含む SETTINGS でも、異なる ID のため
    // 接続エラーにならず、両方の値が独立に保持される
    // (RFC 9114 Section 7.2.4 の重複禁止は同一 ID のみが対象)
    let server =
        make_server_with_client_settings(&[(WT_ENABLED_DRAFT15_16, 1), (WT_ENABLED_DRAFT17, 1)]);

    let peer = server
        .peer_settings()
        .expect("ピア SETTINGS が保持されること");
    let peer_wt = peer
        .wt_settings
        .as_ref()
        .expect("WebTransport SETTINGS が保持されること");
    assert_eq!(peer_wt.wt_enabled, vi(1));
    assert_eq!(peer_wt.wt_enabled_draft17, vi(1));
    assert_eq!(
        peer.webtransport_draft_pattern(),
        Some(DraftVersion::Draft17),
        "新旧併送時は新コードポイントの draft-17 を優先して検出すること"
    );
}

#[test]
fn client_rejects_new_codepoint_two() {
    // クライアントは新コードポイントの値 2 を H3_SETTINGS_ERROR にする
    // (draft-ietf-webtrans-http3-17 Section 3.1)
    let mut client = ClientConnection::new(Settings::new());
    client
        .set_control_stream_id(2)
        .expect("テスト用の制御ストリーム設定に成功すること");

    let control = build_control_stream_with_settings(&[(WT_ENABLED_DRAFT17, 2)]);
    let err = client.feed_stream(3, &control, false).unwrap_err();
    assert!(
        matches!(err, Error::ConnectionError(ErrorCode::SettingsError)),
        "新コードポイントの値 2 は H3_SETTINGS_ERROR になること: {err:?}"
    );
}

#[test]
fn server_accepts_new_codepoint_boundary_values() {
    // SETTINGS_WT_ENABLED > 1 を H3_SETTINGS_ERROR とする MUST はクライアントの
    // 受信動作に対するものであり、サーバーには適用しない。draft 版に対応する
    // サーバーはクライアントが送る値を版選択に使うため、値は保持するが
    // 接続エラーにはしない
    // (draft-ietf-webtrans-http3-17 Section 3.1 の draft 限定段落, Section 7.1)
    for raw in [0u64, 1, 2] {
        let server = make_server_with_client_settings(&[(WT_ENABLED_DRAFT17, raw)]);

        let peer = server
            .peer_settings()
            .expect("ピア SETTINGS が保持されること");
        let peer_wt = peer
            .wt_settings
            .as_ref()
            .expect("WebTransport SETTINGS が保持されること");
        assert_eq!(
            peer_wt.wt_enabled_draft17,
            vi(raw),
            "新コードポイントの値 {raw} がそのまま保持されること"
        );
    }
}

// =========================================================================
// 複数ドラフト版の広告と広告集合
// (draft-ietf-webtrans-http3-17 Section 7.1, 9.2)
// =========================================================================

/// 制御ストリームの生バイト列から SETTINGS の (ID, 値) 一覧を取り出す
///
/// SETTINGS のエントリ順は送信側の実装依存であり、版の選択結果に影響しない
/// ことを確認できるよう、ID と値の組だけを返す。
fn parse_settings_entries(control: &[u8]) -> Vec<(u64, u64)> {
    let (stream_type, stream_type_len) =
        shiguredo_http3::varint::decode(control).expect("制御ストリームタイプを取得できること");
    assert_eq!(
        stream_type.get(),
        0x00,
        "制御ストリームタイプは 0x00 であること"
    );
    let mut pos = stream_type_len;

    let (frame_type, frame_type_len) = shiguredo_http3::varint::decode(&control[pos..])
        .expect("SETTINGS フレームタイプを取得できること");
    assert_eq!(
        frame_type.get(),
        0x04,
        "最初のフレームは SETTINGS であること"
    );
    pos += frame_type_len;

    let (payload_len, payload_len_size) = shiguredo_http3::varint::decode(&control[pos..])
        .expect("SETTINGS ペイロード長を取得できること");
    pos += payload_len_size;
    let payload_end = pos + payload_len.get() as usize;
    assert!(
        payload_end <= control.len(),
        "SETTINGS ペイロードがストリーム長に収まること"
    );

    let mut entries = Vec::new();
    while pos < payload_end {
        let (id, id_len) =
            shiguredo_http3::varint::decode(&control[pos..]).expect("SETTINGS ID を取得できること");
        pos += id_len;
        let (value, value_len) =
            shiguredo_http3::varint::decode(&control[pos..]).expect("SETTINGS 値を取得できること");
        pos += value_len;
        entries.push((id.get(), value.get()));
    }
    entries
}

/// サーバー設定を構築し、制御ストリームの SETTINGS バイト列を取り出す
fn server_control_with_drafts(versions: &[DraftVersion]) -> Vec<u8> {
    let wt = shiguredo_http3::webtransport::Settings::for_draft_versions(
        versions,
        shiguredo_http3::webtransport::connect::ServerSettingsParams::default().max_sessions,
    );
    let mut server = ServerConnection::new(Settings::new().enable_webtransport_server(wt));
    server
        .set_control_stream_id(3)
        .expect("テスト用の制御ストリーム設定に成功すること");
    let (control, _) = server
        .take_stream_data(3)
        .expect("テスト用のサーバー SETTINGS 取得に成功すること");
    control
}

#[test]
fn for_draft_versions_advertises_old_and_new_codepoints_once() {
    // 新旧両対応の端点は、異なる SETTINGS_WT_ENABLED のコードポイントを
    // それぞれ 1 回だけ広告する
    // (draft-ietf-webtrans-http3-17 Section 7.1)
    let control = server_control_with_drafts(&[DraftVersion::Draft15, DraftVersion::Draft17]);
    let entries = parse_settings_entries(&control);

    let old = entries
        .iter()
        .filter(|(id, _)| *id == WT_ENABLED_DRAFT15_16)
        .collect::<Vec<_>>();
    let new = entries
        .iter()
        .filter(|(id, _)| *id == WT_ENABLED_DRAFT17)
        .collect::<Vec<_>>();
    assert_eq!(old.len(), 1, "旧コードポイントは 1 回だけ広告されること");
    assert_eq!(new.len(), 1, "新コードポイントは 1 回だけ広告されること");
    assert_eq!(*old[0], (WT_ENABLED_DRAFT15_16, 1));
    assert_eq!(*new[0], (WT_ENABLED_DRAFT17, 1));
}

#[test]
fn for_draft_versions_with_zero_max_sessions_is_not_advertised() {
    // max_sessions が 0 の場合、draft-07 / draft-14 のコードポイントは送信せず、
    // 広告としても扱わない (draft-ietf-webtrans-http3-17 Section 7.1)
    let wt = shiguredo_http3::webtransport::Settings::for_draft_versions(
        &[DraftVersion::Draft07, DraftVersion::Draft14],
        vi(0),
    );
    assert!(
        wt.advertised_draft_versions().is_empty(),
        "max_sessions が 0 なら旧 draft を広告しないこと"
    );
    assert!(!wt.is_enabled());
    assert_eq!(wt.iter().count(), 0, "0 値のコードポイントも送信しないこと");
}

#[test]
fn for_draft_versions_advertises_legacy_codepoints() {
    // draft-02 / 07 / 14 は SETTINGS_WT_ENABLED 以外の ID で広告する
    let control = server_control_with_drafts(&[
        DraftVersion::Draft02,
        DraftVersion::Draft07,
        DraftVersion::Draft14,
    ]);
    let entries = parse_settings_entries(&control);
    assert!(
        entries.contains(&(0x2b603742, 1)),
        "draft-02 の ID を広告すること"
    );
    assert!(
        entries.contains(&(0xc671706a, 1)),
        "draft-07 の ID を max_sessions で広告すること"
    );
    assert!(
        entries.contains(&(0x14e9cd29, 1)),
        "draft-14 の ID を max_sessions で広告すること"
    );
    // SETTINGS_WT_ENABLED は広告しない
    assert!(
        !entries
            .iter()
            .any(|(id, _)| *id == WT_ENABLED_DRAFT15_16 || *id == WT_ENABLED_DRAFT17)
    );
}

#[test]
fn for_draft_versions_does_not_advertise_flow_control_initial_values() {
    // 版の広告とフロー制御の初期値は別の関心事であり、Safari 26.4 は応答
    // SETTINGS に SETTINGS_WT_INITIAL_MAX_* が含まれると CONNECT をリセットする
    let control = server_control_with_drafts(&[
        DraftVersion::Draft07,
        DraftVersion::Draft15,
        DraftVersion::Draft17,
    ]);
    let entries = parse_settings_entries(&control);
    assert!(
        !entries
            .iter()
            .any(|(id, _)| *id == 0x2b61 || *id == 0x2b64 || *id == 0x2b65),
        "SETTINGS_WT_INITIAL_MAX_* を広告しないこと"
    );
}

#[test]
fn advertised_draft_versions_reflects_wire_settings() {
    // 受信した SETTINGS の広告集合を、生バイト列から構築した接続で確認する
    let control = build_control_stream_with_settings(&[
        (WT_ENABLED_DRAFT15_16, 1),
        (WT_ENABLED_DRAFT17, 1),
        (0x2b603742, 1),
    ]);
    let mut client = ClientConnection::new(Settings::new());
    client
        .set_control_stream_id(2)
        .expect("テスト用の制御ストリーム設定に成功すること");
    client
        .feed_stream(3, &control, false)
        .expect("テスト用のサーバー SETTINGS 受信に成功すること");
    while let Some(_ev) = client
        .poll_event()
        .expect("テスト用のイベント取得に成功すること")
    {}
    let peer_wt = client
        .peer_settings()
        .expect("ピア SETTINGS が保持されること")
        .wt_settings
        .as_ref()
        .expect("WebTransport SETTINGS が保持されること");
    assert_eq!(
        peer_wt.advertised_draft_versions(),
        vec![
            DraftVersion::Draft17,
            DraftVersion::Draft15,
            DraftVersion::Draft02
        ]
    );
}

#[test]
fn advertised_draft_versions_is_empty_without_wt_codepoints() {
    // WebTransport の SETTINGS を受信しても、版のコードポイントがなければ
    // 広告集合は空になる
    let client = make_client_with_server_settings(&[(0x2b61, 1024)]);
    let peer_wt = client
        .peer_settings()
        .expect("ピア SETTINGS が保持されること")
        .wt_settings
        .as_ref()
        .expect("WebTransport SETTINGS が保持されること");
    assert!(peer_wt.advertised_draft_versions().is_empty());
    assert!(!peer_wt.is_enabled());
}
