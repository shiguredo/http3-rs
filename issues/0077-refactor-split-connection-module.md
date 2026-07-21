# 0077: connection/mod.rs の WebTransport ロジックをサブモジュールに分割する

- Priority: Low
- Created: 2026-05-14
- Model: deepseek-v4-pro
- Branch: feature/refactor-split-connection-module
- Polished: 2026-07-21

## 目的

`src/connection/mod.rs` が 6149 行 (本体約 4043 行 + インラインテスト約 2106 行) に肥大化しており、HTTP/3 接続管理、WebTransport セッション管理、QPACK ストリーム処理が単一ファイルに混在している。本体の大半を WebTransport 関連が占め、変更の影響範囲の把握が困難。

AGENTS.md の「テストが長くなるのはモジュール自体が大きすぎるサインなので `src/<module>.rs` 側の分割を検討すること」に従い、WebTransport ロジックをサブモジュールに分離する。

## 優先度根拠

Low: 機能的な問題はなく、保守性の改善が目的。本体約 4043 行の大半を WebTransport 関連 (純粋関数・混在分岐・型定義) が占める。ただし大規模リファクタリングであり、段階的かつ慎重に進める必要がある。

## 前提となる完了済み issue

以下はいずれも完了済み (`issues/closed/`)。その結果を前提に分割する:

- **0080** (未使用コード削除): `MAX_BUFFERED_STREAMS` / `MAX_BUFFERED_DATAGRAMS` は `src/webtransport/session.rs` に集約済みで、mod.rs はそこを path 参照している。mod.rs に残る WebTransport 定数は `WT_MAX_PENDING_SESSIONS` と `WT_MAX_BUFFERED_STREAM_BYTES` の 2 個のみ。
- **0081** (`send_request` / `send_response` の pub(crate) 化): 両関数は既に `pub(crate)`。
- **0082** (Event enum のネスト化): WebTransport イベント発行は `Event::WebTransport(WebTransportEvent::...)` 形式。分割時はこの形を前提とする。

## スコープ外

- テストの外部ファイル分離は本 issue のスコープ外（別 issue として作成予定）
- QPACK / GOAWAY 処理の分割は本 issue のスコープ外
- バッファリング配送処理の共通化は issue 0110 が担当する（本 issue では扱わない）

## 現状

mod.rs に現存する移動対象の WebTransport 要素:

- 型定義: `AssocOutcome`, `BufferedStreamEntry` (`impl BufferedStreamEntry` を含む), `WtSession`, `WtSessionState`
- 定数: `WT_MAX_PENDING_SESSIONS`, `WT_MAX_BUFFERED_STREAM_BYTES`
- `impl WtSession` の 18 メソッド: `new`, `initialize_flow_control`, `check_received_stream`, `add_received_stream`, `on_remote_stream_closed`, `on_data_consumed`, `take_pending_capsules`, `associate_stream`, `disassociate_stream`, `buffer_stream`, `append_buffered_stream_data`, `mark_buffered_stream_fin`, `take_buffered_streams`, `take_buffered_stream_entry`, `buffer_datagram`, `take_buffered_datagrams`, `check_received_data`, `add_received_data`
- WebTransport 純粋関数 (名前に `wt_` / `webtransport` / `datagram` / `capsule` を含む)
- HTTP/3 本体関数内の WebTransport 分岐:
  - `send_request` — WT CONNECT 検証
  - `send_response` — WT セッション確立処理
  - `emit_header_events` — WT セッション検証・確立
  - `handle_unidirectional_stream` — WT ストリーム処理
  - `handle_new_unidirectional_stream` — ストリームタイプ `0x54` (WT 単方向) の case 分岐
  - `feed_stream` — WT bidi ストリームのディスパッチ分岐
  - `stream_reset` — WT セッション / データストリームへの RESET_STREAM 伝播分岐
  - `stop_sending` — WT セッション / データストリームへの STOP_SENDING 伝播分岐
  - `process_control_stream` — GOAWAY 受信時の WT draining 伝播分岐
  - `process_stream_frames` — DATA フレーム受信時の WT セッション状態分岐・Capsule バッファ検証

注意: mod.rs の `WtSession` / `WtSessionState` / `BufferedStreamEntry` は、`webtransport::session` の `Session` / `SessionState` / `BufferedStream` (別実装) とは別物。混同しないこと。

## 設計方針

### サブモジュール構成

`src/connection/` 配下に、既存の `mod client; mod server;` と同様の **private `mod` 宣言** (`pub mod` にはしない。private 型の漏洩を防ぐ) で以下を新設する:

1. **`wt_types.rs`** — WebTransport 型定義・定数・`impl WtSession`
   - 現状セクションで列挙した型定義・定数・18 メソッドを移動する

2. **`wt_session.rs`** — WebTransport セッションのライフサイクルと状態管理
   - `is_wt_fully_negotiated`, `is_wt_flow_control_enabled`
   - `peer_wt_draft_version`, `negotiated_wt_draft_version`, `mutually_advertised_wt_drafts`
   - `peer_requires_initial_wt_capsules`
   - `terminate_wt_session`, `terminate_wt_session_with`
   - `count_pending_wt_sessions`, `count_active_wt_sessions`
   - `set_webtransport_transport_verified`, `is_webtransport_transport_verified`

3. **`wt_stream.rs`** — WebTransport ストリーム受信・ディスパッチ・送信
   - `resolve_wt_uni_stream_session_id`, `resolve_wt_bidi_stream_header`
   - `handle_wt_bidi_stream`
   - `associate_or_buffer_stream`
   - `feed_datagram`, `send_datagram`
   - `wt_stream_header_len`
   - `wt_data_consumed`, `wt_session_flow_control_enabled`

4. **`wt_capsule.rs`** — Capsule Protocol 処理
   - `process_wt_capsule_data`, `handle_wt_capsule`
   - `take_wt_pending_capsules`

`is_webtransport_connect` (フリー関数) は Capsule 処理からは呼ばれず、`send_request` / `emit_header_events` (mod.rs に残す混在本体) から呼ばれる WT 判定。よって `wt_capsule.rs` には置かず、mod.rs にフリー関数として残す。

### フィールド・メソッドの可視性

サブモジュール内に `impl Connection` ブロックを定義する方式を採用する。ただし可視性に注意する:

- **フィールド**: 子モジュール (`connection::wt_*`) から `Connection` の private フィールドに直接アクセスできるため、フィールドの可視性変更は不要。
- **メソッド (重要)**: 子モジュールの `impl Connection` に置いた private メソッドは、Rust の private スコープ (定義モジュールとその子孫) の制約により、**親 (mod.rs) や兄弟 (`#[cfg(test)] mod tests`) からは呼べず E0624 になる**。mod.rs 残存コードとテストから呼ばれるメソッドは、移動時に可視性を `pub(crate)` (または `pub(super)`) へ引き上げる。
- **既に `pub` のメソッド**: `feed_datagram`, `send_datagram`, `set_webtransport_transport_verified`, `is_webtransport_transport_verified`, `wt_stream_header_len`, `wt_data_consumed`, `take_wt_pending_capsules`, `wt_session_flow_control_enabled` はそのまま `pub` を維持する。
- **型・enum・定数 (重要)**: `wt_types.rs` に移動する `WtSession`, `WtSessionState`, `BufferedStreamEntry`, `AssocOutcome` の各 struct / enum とそのフィールド・variant、および `WT_MAX_PENDING_SESSIONS`, `WT_MAX_BUFFERED_STREAM_BYTES` は、mod.rs から直接アクセスされるため `pub(crate)` (または `pub(super)`) にする。private のままでは E0603 / E0616 でコンパイル不能になる。

### 混在関数の処理方針

HTTP/3 本体関数内に埋め込まれた WebTransport 分岐は、`pub(crate)` ヘルパーメソッドとして抽出した上でサブモジュールに配置し、本体は呼び出しのみにする:

- `send_request` 内の WT CONNECT 検証 → `wt_session.rs` にヘルパー抽出
- `send_response` 内の WT セッション確立処理 → `wt_session.rs` にヘルパー抽出
- `emit_header_events` 内の WT セッション検証・確立 → `wt_session.rs` にヘルパー抽出
- `handle_unidirectional_stream` 内の WT ストリーム処理 → `wt_stream.rs` にヘルパー抽出
- `handle_new_unidirectional_stream` 内の `0x54` マッチアーム → `wt_stream.rs` にヘルパー抽出
- `feed_stream` 内の WT bidi ディスパッチ分岐 → `wt_stream.rs` にヘルパー抽出
- `stream_reset` 内の WT セッション / データストリーム伝播分岐 → `wt_session.rs` にヘルパー抽出
- `stop_sending` 内の WT セッション / データストリーム伝播分岐 → `wt_session.rs` にヘルパー抽出
- `process_control_stream` 内の GOAWAY → WT draining 伝播分岐 → `wt_session.rs` にヘルパー抽出
- `process_stream_frames` 内の DATA フレーム受信時 WT セッション状態分岐・Capsule バッファ検証 → `wt_capsule.rs` にヘルパー抽出

混在関数の本体 (`send_request`, `emit_header_events` 等) は mod.rs に残し、ヘルパーへの呼び出しのみを行う形にする。

注意点:

- `dispatch_client_bidi_stream` の WT bidi 処理は既に `handle_wt_bidi_stream` メソッドに分離済みで、`dispatch_client_bidi_stream` 内は呼び出し 1 行のみ。新たな分岐抽出は不要で、`handle_wt_bidi_stream` 本体を `wt_stream.rs` に移すだけでよい。
- バッファリング配送処理の共通化は issue 0110 が担当する。本 issue では `send_response` / `emit_header_events` 内の重複コードに手を加えない。

### テスト移動方針

`#[cfg(test)] mod tests` 内の WebTransport テストは、本リファクタリングのスコープでは mod.rs 内に残す。移動するメソッドを `pub(crate)` 化するため、`mod tests` から呼べる。テストの外部ファイル分離は別 issue として対応する。

QPACK / GOAWAY 処理は本 issue のスコープ外であり、mod.rs に残す。

## 段階的な実施計画

順序は wt_types → wt_session → wt_stream → wt_capsule → 混在関数抽出。各 Phase では、移動対象メソッドの可視性を `pub(crate)` に引き上げる変更と移動を**同一コミット内**で行い、中間状態がビルド不能にならないようにする。各 Phase ごとに `make fmt` / `make clippy` / `make test` が pass することを確認する。

各 Phase の完了基準 (検証可能な形で):

1. **Phase 1 (`wt_types.rs`)**: WT 型定義・定数・`impl WtSession` (`impl BufferedStreamEntry` を含む) が `wt_types.rs` に移り、mod.rs から該当定義が消えている。
2. **Phase 2 (`wt_session.rs`)**: 上記セッション管理系メソッドが `wt_session.rs` に移っている。
3. **Phase 3 (`wt_stream.rs`)**: 上記ストリーム処理系メソッド (`handle_wt_bidi_stream` を含む) が `wt_stream.rs` に移っている。
4. **Phase 4 (`wt_capsule.rs`)**: Capsule 処理メソッドが `wt_capsule.rs` に移っている。
5. **Phase 5 (混在関数抽出)**: 混在 WT 分岐がヘルパーに抽出され、本体は呼び出しのみになっている。バッファリング配送の共通化は issue 0110 のスコープのため本 Phase では扱わない。

## 完了条件

- WebTransport 純粋関数とヘルパーメソッドがサブモジュールに分離されている。
- mod.rs の本体コード (テスト除外) が大幅に縮小している (目安: 2500 行程度)。混在関数の本体・呼び出しは mod.rs に残るため、縮小の中心は純粋関数・型定義・抽出ヘルパー分。バッファリング配送の共通化 (issue 0110) は本 issue の縮小目標に含めない。
- 公開 API に変更がない (`ClientConnection` / `ServerConnection` 経由のアクセスが維持される。`send_request` / `send_response` は既に `pub(crate)`)。
- `make test` が全て pass する。
- `make clippy` で新たな警告がない。
- `make interop-test` (相互運用テスト) が pass する。

## 影響範囲

- `src/connection/mod.rs`: 大幅縮小
- 新規: `src/connection/wt_types.rs`, `wt_session.rs`, `wt_stream.rs`, `wt_capsule.rs` (いずれも private `mod`)
- 影響なし: `src/connection/client.rs`, `server.rs` (委譲ラッパー)、`tests/test_connection.rs`, `tests/test_webtransport_draft_connect.rs`, `tests/test_webtransport_flow_control.rs`, `pbt/tests/prop_connection.rs` (pub API は変更しないため)

## 関連 issue との順序関係

- **issue 0127** (`connection/mod.rs` の stale コメント修正): 特定行番号を参照しているため、0077 が先に実施されると行番号が無効になる。0127 を先に実施するか、0077 実施後に 0127 の行番号を更新すること。
- **issue 0110** (WT セッション確立処理の重複排除): バッファリング配送の共通化を担当。0077 の Phase 5 後に実施すると、ヘルパー抽出済みのコードに対して共通化できる。

## CHANGES.md エントリ案

```markdown
### misc

- [UPDATE] connection/mod.rs から WebTransport ロジックをサブモジュールに分割する
  - @voluntas
```
