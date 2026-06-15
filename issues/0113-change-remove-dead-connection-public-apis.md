# `Connection` の死に public API を削除する

- Priority: Medium
- Created: 2026-06-15
- Completed:
- Model: Opus 4.7
- Branch: feature/change-remove-dead-connection-public-apis
- Polished:

## 目的

`Connection` の以下の public API が本体・テスト以外で利用されておらず、死にコードになっている。削除または `pub(crate)` 化して API 表面積を縮小する。

- `Connection::error()` (mod.rs:3960-3963)
- `Connection::collect_closed_streams()` (mod.rs:3801-3812)
- `Connection::mark_stream_fin_sent(stream_id)` (mod.rs:3369-3373)
- `Connection::client(settings)` / `Connection::server(settings)` (mod.rs:597-604)

## 優先度根拠

Medium。死に API を残すと利用者を混乱させ、リファクタリングのコストが上がる。`ClientConnection` / `ServerConnection` の薄ラッパが存在する以上、`Connection::client/server` は重複する API。削除によってラッパ経由の利用パスが明確になる。

## 現状

各 API の利用状況を `grep` で確認:

- `Connection::error()` → 呼び出し 0 件
- `collect_closed_streams` → 呼び出し 0 件
- `mark_stream_fin_sent` → 外部・内部とも 0 件 (`RequestStream::mark_fin_sent` も同様)
- `Connection::client/server` → src 内のテストと fuzz_target のみ

`ClientConnection::new` / `ServerConnection::new` が正規 API として存在しているため、`Connection::client/server` は API 二重露出になっている。

## 設計方針

- `Connection::error()` を削除 (`self.error` フィールドはコメント上「呼び出し側がエラー後の Connection を破棄する」設計と乖離。テストが直接 `conn.error = Some(...)` する形で使うため `pub(crate)` 維持の選択肢も)
- `collect_closed_streams` / `mark_stream_fin_sent` を削除 (どこからも呼ばれない)
- `Connection::client/server` を削除または `pub(crate)` 化 (fuzz_target は `ClientConnection::new` / `ServerConnection::new` を直接呼べる)
- `CHANGES.md` に `[CHANGE]` エントリを追加

## 完了条件

- 上記 API が削除または `pub(crate)` 化される
- fuzz / tests / 本体が新 API でビルド・テスト通過する
- `CHANGES.md` に変更エントリ追加
- `make fmt && make clippy && make check` が通る

## 解決方法

各 API の `pub` を削除または `pub(crate)` に変更。fuzz_target は `ClientConnection::new` / `ServerConnection::new` を呼ぶように書き換える。

### 関連ファイル

- 修正対象: `src/connection/mod.rs:597-604, 3369-3373, 3801-3812, 3960-3963`
- 影響: `fuzz/fuzz_targets/fuzz_connection.rs`, `fuzz/fuzz_targets/fuzz_stream.rs`
- 関連 issue: 0111 (Client/Server ラッパ API 拡充)
