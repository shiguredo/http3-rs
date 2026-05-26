# 0077: connection/mod.rs の WebTransport ロジックをサブモジュールに分割する

- Priority: Low
- Created: 2026-05-14
- Polished: 2026-05-26
- Model: deepseek-v4-pro
- Branch: feature/refactor-split-connection-module

## 目的

`src/connection/mod.rs` が 5852 行 (本体 4005 行 + インラインテスト 1847 行) に肥大化しており、HTTP/3 接続管理、WebTransport セッション管理、QPACK ストリーム処理が単一ファイルに混在している。関数数 181 (うち非公開 140) であり、変更の影響範囲把握が困難。

CLAUDE.md の「テストが長くなるのはモジュール自体が大きすぎるサインなので `src/<module>.rs` 側の分割を検討すること」に従い、WebTransport ロジックをサブモジュールに分離する。

## 優先度根拠

Low: 機能的な問題はないが保守性が低下している。 5852 行のファイルに 181 関数が混在し、 WebTransport 関連だけで本体約 2200 行 + テスト約 1550 行を占める。ただし分割は大規模リファクタリングであり、段階的かつ慎重に進める必要がある。

## 依存関係

- issue 0082 (Event enum の WebTransport バリアントのネスト化) が先に実施された場合、分割対象のイベント発行コードが変わる。 0082 を先に実施するか、同時に行う場合はイベント発行箇所の変更を見越して分割すること
- issue 0080 (未使用コード削除) で `connection/mod.rs` の重複定数 (`WT_MAX_BUFFERED_STREAMS` 等) を `src/webtransport/session.rs` に集約する方針が含まれる。 0080 が先に実施された場合、`wt_types.rs` への定数移動はスキップし `webtransport::session` 側の定数を参照する。 0080 未実施なら `wt_types.rs` に定数を移動する

## 現状

- `src/connection/mod.rs`: 5852 行 (本体 4005 行 + `#[cfg(test)]` テスト 1847 行)
- 関数数: 181 (pub 41 / 非公開 140)
- WebTransport 純粋関数 (名前に `wt_`/`webtransport`/`datagram`/`capsule` を含む): 約 1100 行
- HTTP/3 関数内の WebTransport 分岐 (`send_request`, `send_response`, `emit_header_events`, `handle_unidirectional_stream`, `dispatch_client_bidi_stream` 等): 約 700 行
- WebTransport 型定義・定数 (`WtSession`, `WtSessionState`, `BufferedStreamEntry`, `AssocOutcome`, 定数群): 約 370 行
- WebTransport テストコード (`#[cfg(test)]` 内): 約 1550 行

## 設計方針

### サブモジュール構成

`src/connection/` 配下に以下を新設する:

1. **`wt_types.rs`** — WebTransport 型定義・定数・`impl WtSession` メソッド群
   - `WtSession`, `WtSessionState`, `BufferedStreamEntry`, `AssocOutcome` の型定義
   - `WT_MAX_BUFFERED_STREAMS`, `WT_MAX_BUFFERED_DATAGRAMS`, `WT_MAX_PENDING_SESSIONS`, `WT_MAX_BUFFERED_STREAM_BYTES` (issue 0080 未実施の場合)
   - `impl WtSession` の全メソッド (18 個、約 240 行): `new`, `initialize_flow_control`, `check_received_stream`, `add_received_stream`, `on_remote_stream_closed`, `on_data_consumed`, `take_pending_capsules`, `associate_stream`, `disassociate_stream`, `buffer_stream`, `append_buffered_stream_data`, `mark_buffered_stream_fin`, `take_buffered_streams`, `take_buffered_stream_entry`, `buffer_datagram`, `take_buffered_datagrams`, `check_received_data`, `add_received_data`

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
   - `is_webtransport_connect` (フリー関数、`send_request` / `send_response` / `emit_header_events` から呼ばれる WT 専用判定)

### フィールドアクセス方式

サブモジュール内で `impl Connection` ブロックを定義する方式を採用する。`connection/` 配下のサブモジュールからは `Connection` の private フィールドに直接アクセスできるため、フィールドの可視性変更は不要。

### 混在関数の処理方針

HTTP/3 本体関数内に埋め込まれた WebTransport 分岐は、ヘルパーメソッドとして抽出した上でサブモジュールに配置する。具体的には:

- `send_request` 内の WT CONNECT 検証 → `wt_session.rs` にヘルパーメソッドを抽出し、`send_request` から呼び出す
- `send_response` 内の WT セッション確立処理 → `wt_session.rs` にヘルパーメソッドを抽出
- `emit_header_events` 内の WT セッション検証・確立 → `wt_session.rs` にヘルパーメソッドを抽出
- `handle_unidirectional_stream` 内の WT ストリーム処理 → `wt_stream.rs` にヘルパーメソッドを抽出
- `dispatch_client_bidi_stream` 内の WT bidi 分岐 → `wt_stream.rs` にヘルパーメソッドを抽出 (HTTP/3 リクエストストリーム分岐は `mod.rs` に残す)

混在関数の本体 (`send_request`, `emit_header_events` 等) は `mod.rs` に残し、WT ヘルパーへの呼び出しのみを行う形にする。

`send_response` (3627-3706 行) と `emit_header_events` (3035-3119 行) にはバッファリング配送処理 (`take_buffered_streams` → フロー制御チェック → イベント生成) がほぼ同一のロジックとして重複している (各約 80 行)。この重複を 1 つのヘルパーメソッドに共通化する。借用チェッカー対策として、ヘルパーは `session_id` を引数に取る `Connection` のメソッドとして実装し、 `wt_sessions.get_mut()` のスコープを分離する現行パターンを維持する。

### テスト移動方針

`#[cfg(test)] mod tests` 内の WebTransport テスト (約 1550 行) は、このリファクタリングのスコープでは `mod.rs` 内に残す。テストの外部ファイル分離は別 issue として対応する。WT テストはサブモジュール内の `impl Connection` メソッドを呼ぶため、`mod.rs` 内にあっても問題ない。

QPACK / GOAWAY 処理はこの issue のスコープ外であり、`mod.rs` に残す。

## 段階的な実施計画

1. **Phase 1**: `wt_types.rs` を新設し、WT 型定義・定数・`impl WtSession` を移動 (約 360 行)
2. **Phase 2**: `wt_session.rs` を新設し、セッション管理系メソッドを移動
3. **Phase 3**: `wt_stream.rs` を新設し、ストリーム処理系メソッドを移動
4. **Phase 4**: `wt_capsule.rs` を新設し、Capsule 処理メソッドを移動
5. **Phase 5**: 混在関数からヘルパーメソッドを抽出し、サブモジュールに配置

各 Phase ごとに `cargo test` と `cargo clippy` が pass することを確認する。

## 完了条件

- WebTransport 純粋関数とヘルパーメソッドがサブモジュールに分離されていること
- `src/connection/mod.rs` の本体コード (テスト除外) が 2500 行以下になっていること
- 公開 API に変更がないこと (`ClientConnection` / `ServerConnection` 経由のアクセスが維持されること)
- `cargo test` が全て pass すること
- `cargo clippy` で新たな警告がないこと
- 相互運用テストが pass すること

## 影響範囲

- `src/connection/mod.rs`: 大幅縮小
- 新規: `src/connection/wt_types.rs`, `src/connection/wt_session.rs`, `src/connection/wt_stream.rs`, `src/connection/wt_capsule.rs`
- 影響なし (確認済み): `src/connection/client.rs`, `src/connection/server.rs` (委譲ラッパーのため)
- 影響なし (確認済み): `tests/test_connection.rs`, `tests/test_webtransport_draft_connect.rs`, `tests/test_webtransport_flow_control.rs`, `pbt/tests/prop_connection.rs` (pub API は変更しないため)

## CHANGES.md エントリ案

```markdown
### misc

- [UPDATE] connection/mod.rs から WebTransport ロジックをサブモジュールに分割する
  - @voluntas
```
