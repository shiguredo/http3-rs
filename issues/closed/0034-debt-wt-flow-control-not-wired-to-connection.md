# WebTransport フロー制御が Connection に配線されていない

Created: 2026-04-05
Completed: 2026-04-06
Model: Opus 4.6

## 優先度

P3（技術負債記録）

## 概要

`Session` にはフロー制御メソッド (`check_received_data()` / `add_received_data()` / `check_received_stream()` / `on_remote_stream_closed()`) が実装済みだが、`Connection` の WebTransport データ経路から呼び出されていない。受信側の limit 超過検知やウィンドウ更新が自動で効かない状態。

## 根拠

`Session` のフロー制御 API (`src/webtransport/session.rs` L643-710):

- `check_received_data()` — `WT_MAX_DATA` 超過を検証
- `check_received_stream()` — `WT_MAX_STREAMS` 超過を検証
- `on_remote_stream_closed()` — ウィンドウ更新の `WT_MAX_STREAMS` カプセル生成

`Connection` の WT データ経路 (`src/connection/mod.rs` L1198, L1854 付近) はそのままイベント化しており、上記メソッドを呼んでいない。

draft-ietf-webtrans-http3-15 Section 5.4, 5.6 で定義される `WT_FLOW_CONTROL_ERROR` の検出が Sans I/O 層で行えない。

## 対応を保留する理由

- draft-15 ベースの機能であり RFC 化されていない
- draft 更新時にフロー制御の仕様が変更される可能性がある
- Session 側の実装自体は完了しており、配線のみが残タスク

## 対応方針（将来）

draft が安定した段階で:

1. `handle_wt_bidi_stream()` / `handle_wt_uni_stream()` のデータ受信パスで `check_received_data()` / `add_received_data()` を呼ぶ
2. ストリーム受け入れ時に `check_received_stream()` / `add_received_stream()` を呼ぶ
3. ストリーム完了時に `on_remote_stream_closed()` を呼び、生成されたカプセルを送信キューに積む
4. 違反時は `WT_FLOW_CONTROL_ERROR` でセッション終了

## 参照

- draft-ietf-webtrans-http3-15 Section 5.4, 5.6
- `src/webtransport/session.rs` L643-710
- `src/connection/mod.rs` L1198, L1854

## 解決方法

`WtSession` (Connection 層) にフロー制御状態を追加し、データ経路に配線した。

### 追加フィールド
- `recv_stream_fc_uni` / `recv_stream_fc_bidi` (`DirectionalStreamFlowControl`)
- `recv_data_fc` (`DataFlowControl`)
- `pending_capsules` (Connection 層が生成した WT_MAX_STREAMS / WT_MAX_DATA)

### 配線箇所
1. **セッション確立時**: ローカル SETTINGS の `wt_initial_max_*` でフロー制御を初期化。バッファされたストリームの FC チェックも実施。
2. **ストリーム受け入れ時** (`resolve_wt_uni_stream_session_id` / `resolve_wt_bidi_stream_header`): `check_received_stream` + `add_received_stream`
3. **データ受信時** (`handle_wt_bidi_stream` / `handle_unidirectional_stream`): `check_received_data` + `add_received_data`
4. **ストリーム FIN 時**: `on_remote_stream_closed` で WT_MAX_STREAMS 更新カプセルを生成
5. **違反時**: `WT_FLOW_CONTROL_ERROR` でセッション終了

### 公開 API
- `wt_data_consumed(session_id, bytes)`: アプリ層がデータ消費を通知 → WT_MAX_DATA 更新
- `take_wt_pending_capsules(session_id)`: 生成されたカプセルの取得
- `wt_session_flow_control_enabled(session_id)`: FC 有効性の取得

### Event 変更
- `WebTransportSessionEstablished` に `flow_control_enabled: bool` フィールドを追加

### 型の可視性変更
- `DirectionalStreamFlowControl` / `DataFlowControl` を `pub(crate)` に変更し、Connection 層で再利用
