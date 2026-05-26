# 0073: PBT ファイルの過大・重複問題

- Priority: Low
- Created: 2026-05-14
- Completed: 2026-05-26
- Model: deepseek-v4-pro
- Branch: feature/refactor-split-oversized-pbt

## 目的

`pbt/tests/prop_webtransport.rs` が 1802 行に肥大化しており、AGENTS.md の分割基準を満たしている。`src/webtransport/` はディレクトリモジュールであるため `pbt/tests/prop_webtransport/main.rs` にサブモジュール分割する必要がある（AGENTS.md: ディレクトリモジュールの場合は `pbt/tests/prop_<module>/main.rs` にサブモジュール対応で分割すること）。

また、専用 PBT ファイル (`prop_capsule.rs`, `prop_datagram.rs`) と `prop_webtransport.rs` 間でラウンドトリップテストが重複している。

## 優先度根拠

Low: テストの動作自体に問題はない。ファイル構成の改善であり、保守性向上が目的。

## 現状

1. `pbt/tests/prop_webtransport.rs`: 1802 行（AGENTS.md の分割基準超過）
2. Capsule ラウンドトリップが `prop_capsule.rs` と `prop_webtransport.rs` で重複
3. Datagram ラウンドトリップが `prop_datagram.rs` と `prop_webtransport.rs` で重複

## 設計方針

1. `pbt/tests/prop_webtransport.rs` → `pbt/tests/prop_webtransport/main.rs` に変換
2. サブモジュール分割: `capsule.rs`, `datagram.rs`, `connect.rs`, `session.rs`, `stream.rs`, `settings.rs`, `error.rs` 等
3. `prop_capsule.rs` / `prop_datagram.rs` と重複するテストを `prop_webtransport/` 側から削除し、専用ファイルに一本化

## 完了条件

- `prop_webtransport/main.rs` + サブモジュールへの分割が完了していること
- 重複テストが削除されていること
- `cargo test -p pbt` が全て pass すること

## 影響範囲

- `pbt/tests/prop_webtransport.rs` → `pbt/tests/prop_webtransport/main.rs` + サブモジュール
- `pbt/tests/prop_capsule.rs`: 一本化先（変更なし）
- `pbt/tests/prop_datagram.rs`: 一本化先（変更なし）

## 解決方法

`pbt/tests/prop_webtransport.rs`（1802 行）を `pbt/tests/prop_webtransport/` ディレクトリモジュールに分割した。

### 変更内容

- `pbt/tests/prop_webtransport.rs` を削除
- `pbt/tests/prop_webtransport/main.rs` を新規作成（モジュール宣言のみ）
- 以下のサブモジュールを新規作成:
  - `capsule.rs`: Unknown ラウンドトリップ、capsule_type 検証、不完全バッファの Sans I/O 挙動（5 テスト）
  - `connect.rs`: ConnectRequest/Response バリデーション、プロトコルネゴシエーション（7 テスト）
  - `error.rs`: ApplicationErrorCode、メッセージ長制約、予約コードポイント検証（10 テスト）
  - `session.rs`: 状態遷移、フロー制御、バッファリング、GOAWAY、終了処理、動的ウィンドウ更新（39 テスト）
  - `settings.rs`: フロー制御有効化判定、iter 整合性（4 テスト）
  - `stream.rs`: StreamHeader ラウンドトリップ/フォーマット、Stream ID 分類（11 テスト）
- 計 76 テスト → 68 テスト（重複 11 件削除、全テスト pass）

### 削除した重複テスト

- `prop_capsule.rs` と重複する Capsule ラウンドトリップ 6 件（CloseSession / DrainSession / MaxData / MaxStreams / DataBlocked / StreamsBlocked）
- `prop_datagram.rs` と重複する Datagram テスト 5 件（roundtrip / quarter_stream_id / encoded_quarter_stream_id / empty_buffer / large_payload）

### CLAUDE.md 規約対応

- `.unwrap()` を `.expect("MESSAGE")` に置き換え
- 英語コメントを日本語に修正
- 全角半角間スペースを修正

## CHANGES.md エントリ案

```
### misc

- [UPDATE] prop_webtransport.rs をディレクトリモジュールに分割し PBT 間の重複テストを削除する
  - @voluntas
```
