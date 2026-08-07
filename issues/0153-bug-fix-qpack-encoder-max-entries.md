# QPACK エンコーダーが max_table_capacity = 0 のまま非空テーブルで encode すると panic / 不正フィールドセクションを生成する

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-qpack-encoder-max-entries
- Polished: {YYYY-MM-DD}

## 目的

公開 API 経由で到達可能な panic 経路と、リモート SETTINGS 受信前の不正なフィールドセクション生成を修正する。

## 現状

- `src/qpack/encoder.rs` の `DynamicEncoder::new` は `max_table_capacity = 0` で始まる。`insert` は `max_table_capacity` を一切見ず、テーブル自身の容量のみで受理する
- `DynamicEncoder::set_table_capacity(1024)` → `insert()` を実行した状態で、`set_max_table_capacity` を設定せずに公開 API `encode()` を呼ぶと:
  - debug ビルド: `debug_assert!(max_entries > 0)` で panic
  - release ビルド: RIC = 0 なのに動的テーブル参照を含む不正なフィールドセクションを生成 (デコーダーは必ずデコードエラーにする)
- `src/qpack/encoder.rs` の `set_table_capacity` はピアの SETTINGS (SETTINGS_QPACK_MAX_TABLE_CAPACITY) 上限を検証しない。RFC 9204 Section 3.2.3「The encoder MUST NOT allow the dynamic table capacity to exceed the maximum capacity set by the decoder」に違反する経路がある
- `EncoderStream::encode_set_capacity` は検証しているのに API が非対称

## 設計方針

- `encode()` 冒頭で `max_entries == 0` を検査し、動的参照を使わないエンコードにフォールバックするか `None` を返す
- `set_table_capacity` にピア上限の検査を追加する

## 完了条件

- `max_table_capacity = 0` の状態で `encode()` を呼んでも panic せず、不正なフィールドセクションを生成しない
- ピア上限を超える `set_table_capacity` がエラーになる
- テストが追加される
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/qpack/encoder.rs` (`DynamicEncoder::encode` / `set_table_capacity` / `set_max_table_capacity` / `insert`)
- 一次資料: `refs/h3/rfc9204.txt` Section 3.2.3
