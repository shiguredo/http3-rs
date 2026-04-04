# フロー制御のネゴシエーションと初期値注入が接続層に結線されていない

Created: 2026-04-05
Model: Opus 4.6

## 概要

`webtransport::Settings` に `flow_control_enabled_with_peer()` (両端が宣言した場合のみ有効) と
`initialize_local_limits()` が実装されているが、接続層 (`Connection`) からどちらも呼び出されていない。

`Session::new()` で `flow_control_enabled: true` がハードコードされており、
両端がフロー制御を宣言していない場合でもフロー制御が有効として動作する。

## 根拠

- `draft-ietf-webtrans-http3-15 Section 5.1`: フロー制御は**両エンドポイント**がいずれかの SETTINGS (`SETTINGS_WT_INITIAL_MAX_STREAMS_UNI`, `SETTINGS_WT_INITIAL_MAX_STREAMS_BIDI`, `SETTINGS_WT_INITIAL_MAX_DATA`) を 0 以外の値で送信した場合にのみ有効
- フロー制御無効時、クライアントは最大 1 セッションのみ許可される
- `initialize_local_limits()` が呼ばれないため、受信側フロー制御 (`DirectionalStreamFlowControl`, `DataFlowControl`) の初期値が正しく設定されない
- `queue_initial_flow_control_capsules()` も接続層から到達していない

## 必要な変更

1. セッション確立時に `flow_control_enabled_with_peer(&local_wt_settings, &peer_wt_settings)` を評価する
2. 結果を `Session::set_flow_control_enabled()` で注入する
3. フロー制御有効時、`initialize_local_limits()` を SETTINGS の初期値で呼び出す
4. フロー制御有効時、`queue_initial_flow_control_capsules()` で初期カプセルを生成する
5. フロー制御無効時、複数セッションの同時使用を制限する

## 優先度

P1 — フロー制御の有効/無効判定が仕様と異なり、複数セッション制限も機能しない。
Capsule 処理の接続層統合 (issue 0018) が前提条件。

## 解決方法

Completed: 2026-04-05

1. `WtSession` に `flow_control_enabled: bool` フィールドを追加 (デフォルト `false`)
2. `Connection::is_wt_flow_control_enabled()` ヘルパーを追加 — ローカルとピアの WebTransport SETTINGS から `flow_control_enabled_with_peer` を評価
3. セッション確立時 (クライアント/サーバー両方) で `is_wt_flow_control_enabled()` の結果を `WtSession::flow_control_enabled` に注入
4. `handle_wt_capsule` でフロー制御カプセル受信時に `flow_control_enabled` を確認し、有効時のみ `WebTransportCapsule` イベントを生成、無効時は無視 (Section 5.1)
5. `Event::WebTransportCapsule` バリアントを追加 — 上位層が `Session::process_capsule` で処理するためのイベント

注: `initialize_local_limits()` と `queue_initial_flow_control_capsules()` の呼び出しは上位層 (`webtransport::Session`) の責務として残した。接続層は `WebTransportCapsule` イベント経由でカプセルを上位に渡す設計。

## 参考

- `src/webtransport/settings.rs:212`: `flow_control_enabled_with_peer`
- `src/webtransport/session.rs:340`: `flow_control_enabled: true` のハードコード
- `src/webtransport/session.rs:504-509`: `initialize_local_limits`
