# クライアント / サーバー両側の WT セッション確立処理の重複を共通化する

- Priority: Medium
- Created: 2026-06-15
- Completed:
- Model: Opus 4.7
- Branch: feature/refactor-wt-session-establishment-deduplication
- Polished:

## 目的

`src/connection/mod.rs:3030-3155` (クライアント側 CONNECT レスポンス受信) と `:3625-3742` (サーバー側 CONNECT レスポンス送信) の間で、WT セッションを `Established` 状態に遷移させバッファ済みストリーム / データグラムを順序保って配送する処理が約 100 行ずつ完全コピペで存在する。共通メソッドに集約し、片側の修正がもう片側に反映されない broken windows を解消する。

## 優先度根拠

Medium。重複の片方を直し忘れるとプロトコル挙動の非対称性が生じる。実際 fc_violation 判定やイベント生成スタイルが微妙に異なっており既に乖離の兆候がある。CLAUDE.md「Don't live with broken windows」に該当する。

## 現状

クライアント側 (`src/connection/mod.rs:3030-3155`) とサーバー側 (`:3625-3742`) は以下を行う:

- バッファ済みストリームを `take_buffered_streams` で取り出す
- 各ストリームを `check_received_stream` → `add_received_stream` → `associate_stream` で処理
- `Event::WebTransport(Open/Data/End)` を発火
- バッファ済みデータグラムを `take_buffered_datagrams` で取り出して配送
- セッションを Established に遷移
- `SessionEstablished` イベント発火

差分は「クライアント側は `wt_protocol_invalid` 事前チェック」「サーバー側は `is_wt_flow_control_enabled` / `peer_requires_initial_wt_capsules` をローカル変数で先に解決」だけ。本質的なバッファ配送ロジックは同一。

## 設計方針

- 共通メソッド `Connection::establish_wt_session_and_deliver_buffered(session_id) -> Result<(), Error>` を導入
- 共通メソッドはバッファ配送・イベント発火・state 遷移を担当
- 各経路は事前チェック (`wt_protocol_invalid` 判定 / fc 制約計算) のみ行い、共通メソッドに委譲
- イベント生成順序の不変条件 (Open → Data → End の順) は共通メソッドのテストで担保
- 既存テスト (`test_wt_uni_stream_open_and_data` 等) がそのままパスすることを確認

## 完了条件

- 約 200 行の重複が約 100 行 + 共通メソッドに集約される
- クライアント / サーバー両経路で共通メソッドを呼ぶ
- 既存テストがすべてパスする
- WT セッション確立に関する PBT (もしあれば) もパスする
- `make fmt && make clippy && make check` が通る

## 解決方法

`Connection` に private メソッドを追加:

```rust
fn establish_wt_session_and_deliver_buffered(
    &mut self,
    session_id: u64,
) -> Result<(), Error> {
    // バッファ済みストリーム配送
    // バッファ済みデータグラム配送
    // state 遷移
    // SessionEstablished イベント発火
}
```

クライアント / サーバー双方の呼び出し箇所を共通メソッドに置き換える。

### 関連ファイル

- 修正対象: `src/connection/mod.rs:3030-3155, 3625-3742`
- 関連 issue: 0077 (connection/mod.rs 分割)
