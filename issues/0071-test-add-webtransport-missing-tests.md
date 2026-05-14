# 0071: WebTransport モジュールの不在テストを追加する

Created: 2026-05-14
Model: deepseek-v4-pro

## 概要

以下の WebTransport モジュールに対応する PBT/単体テストが不在である。

### 不在のテスト

1. **`webtransport::connect.rs`**: `ConnectRequest`, `ConnectResponse`, `TransportCapabilities`,
   `DraftVersion` のラウンドトリップ/バリデーション PBT
   - 期待ファイル: `pbt/tests/prop_connect.rs` または `tests/test_connect.rs`

2. **`webtransport::error.rs`**: `ApplicationErrorCode::to_http3_code` / `from_http3_code` の
   ラウンドトリップ PBT
   - 期待ファイル: `pbt/tests/prop_webtransport_error.rs` または
     `pbt/tests/prop_webtransport/main.rs` のサブモジュール

3. **`webtransport::session.rs`**: `Session` の状態遷移、`process_capsule`、フロー制御
   の PBT
   - 期待ファイル: `pbt/tests/prop_session.rs` または
     `pbt/tests/prop_webtransport/main.rs` のサブモジュール
   - `Session` は 1745 行の大規模モジュールだが PBT が不在

4. **`webtransport::stream.rs`**: `Stream`, `classify_uni_stream` の PBT
   - 期待ファイル: `pbt/tests/prop_webtransport_stream.rs` または
     `pbt/tests/prop_webtransport/main.rs` のサブモジュール

## 修正方針

- CLAUDE.md L82-83, L88 に従い、`src/webtransport/` がディレクトリモジュールであるため
  `pbt/tests/prop_webtransport/main.rs` にサブモジュールとして追加する

## 影響範囲

- 新規ファイル: `pbt/tests/prop_webtransport/` (main.rs + サブモジュール)
