# `webtransport/session.rs` と `connect.rs` の責務分離を行う

- Priority: Medium
- Created: 2026-06-15
- Completed:
- Model: Opus 4.7
- Branch: feature/refactor-split-webtransport-session-and-connect
- Polished:

## 目的

`src/webtransport/session.rs` (1774 行) と `connect.rs` (1270 行) が責務集約状態にあり、テスト分離・並列レビュー・将来の機能追加に対する障害になっている。session.rs は `DirectionalStreamFlowControl` / `DataFlowControl` / `Session` 本体等が同居し、connect.rs は `DraftVersion` / `ConnectError` / `ConnectRequest` / `ConnectResponse` / SF パーサが同居している。サブモジュール化する。

## 優先度根拠

Medium。CLAUDE.md「テストファイルが長くなった場合はファイル内で mod を使って分割すること。テストが長くなるのはモジュール自体が大きすぎるサイン」に該当。0077 で `connection/mod.rs` 分割が予定されているため、それと並行して webtransport 側も整理する。

## 現状

`src/webtransport/session.rs` (1774 行) に同居:

- `MAX_BUFFERED_STREAMS` / `MAX_BUFFERED_DATAGRAMS` 定数
- `DirectionalStreamFlowControl` 構造体 (131-193)
- `DataFlowControl` 構造体 (202-259)
- `SendBlockedState` enum (268-275)
- `CapsuleProcessError` enum (279-289)
- `Session` 本体
- `SessionState` enum
- 単体テスト群

`src/webtransport/connect.rs` (1270 行) に同居:

- `DraftVersion` (35-226) と build_*_settings 群 (73 行)
- `ConnectError` / `CapabilityError` / `TransportCapabilities` (228-353)
- `ConnectRequest` (372-589)
- `ConnectResponse` (596-680)
- Structured Fields パーサ (682-756)
- テスト (758-1269)

## 設計方針

- `src/webtransport/session/` ディレクトリモジュール化:
  - `session/flow_control.rs`: `DirectionalStreamFlowControl`, `DataFlowControl`, `FlowControlLimits`, `FlowControlState`, `SendBlockedState`
  - `session/buffering.rs`: `MAX_BUFFERED_*` 定数, バッファ関連
  - `session/mod.rs`: `Session`, `SessionState`
- `src/webtransport/connect/` ディレクトリモジュール化:
  - `connect/draft.rs`: `DraftVersion`, `ServerSettingsParams`, build_*_settings
  - `connect/request.rs`: `ConnectRequest`
  - `connect/response.rs`: `ConnectResponse`
  - `connect/sf_parser.rs`: Structured Fields パーサ
  - `connect/error.rs`: `ConnectError`, `CapabilityError`, `TransportCapabilities`
  - `connect/mod.rs`: 再エクスポート
- PBT 側 (`pbt/tests/prop_webtransport/`) のサブモジュール対応も検討 (issue 0108 と連動)
- 既存テストはそのまま機能するように再エクスポートを維持

## 完了条件

- session.rs と connect.rs がそれぞれ 300 行以下のサブモジュール群に分割される
- 既存 PBT / tests / examples がそのままパスする
- 公開 API (`webtransport::*`) の表面に破壊的変更が無い (もしくは `CHANGES.md` に明記)
- `make fmt && make clippy && make check` が通る

## 解決方法

リファクタリング段階:

1. `flow_control.rs` を分離 (内部利用のみ)
2. `buffering.rs` を分離
3. `Session` を `session/mod.rs` に整理
4. `connect/` 配下も同様に分離
5. 公開境界の `pub` / `pub(crate)` を見直し

### 関連ファイル

- 修正対象: `src/webtransport/session.rs`, `src/webtransport/connect.rs`, `src/webtransport/mod.rs`
- 関連 issue: 0077 (connection/mod.rs 分割), 0108 (PBT 配置)
