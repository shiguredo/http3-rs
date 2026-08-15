# QPACK エンコーダーが max_table_capacity = 0 のまま非空テーブルで encode すると panic / 不正フィールドセクションを生成する

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-qpack-encoder-max-entries
- Polished: 2026-08-15

## 目的

`DynamicEncoder` がピアの `max_table_capacity` (SETTINGS_QPACK_MAX_TABLE_CAPACITY) を尊重しない 2 経路を修正する:

1. 公開 API 経由で到達可能な panic 経路 (debug ビルド) と不正なフィールドセクション生成 (release ビルド)
2. ピア上限を超える動的テーブル容量設定 (RFC 9204 Section 3.2.3 違反)

## 現状

- `src/qpack/encoder.rs` の `DynamicEncoder::new` は `max_table_capacity = 0` で始まる。`insert` は `max_table_capacity` を一切見ず、テーブル自身の容量のみで受理する
- `DynamicEncoder::set_peer_max_blocked_streams(1)` → `set_table_capacity(1024)` → `insert()` を実行した状態で、`set_max_table_capacity` を設定せずに公開 API `encode(buf, headers, 0)` (`blocked_streams_count = 0`) を呼ぶと (デフォルトの `peer_max_blocked_streams = 0` では `encode` が静的フォールバックに入り再現しないため、1 以上への設定が必須):
  - debug ビルド: `debug_assert!(max_entries > 0)` で panic
  - release ビルド: RIC = 0 なのに動的テーブル参照を含む不正なフィールドセクションを生成 (デコーダーは必ずデコードエラーにする)
- この状態は公開 API (`DynamicEncoder`) を直接利用する場合のみ到達可能 (connection 層では動的テーブルが populate されず static-only 経路を通る)
- `src/qpack/encoder.rs` の `set_table_capacity` はピアの SETTINGS 上限を検証しない。RFC 9204 Section 3.2.3「The encoder MUST NOT set a dynamic table capacity that exceeds this maximum」に違反する経路がある
- `EncoderStream::encode_set_capacity` は検証しているのに API が非対称

## 設計方針

- `encode()` 冒頭で `max_entries == 0` を検査し、この検査ではエラーや `None` を返さず、動的参照を使わない静的テーブルのみのエンコードにフォールバックする (既存の空テーブル・ブロック上限時と同じ経路)
- `set_table_capacity` を `Result<(), QpackError>` 化し、`capacity > max_table_capacity` のとき `QpackError::CapacityExceeded` を返す。`capacity = 0` は常に許可する (`max_table_capacity = 0` でも `set_table_capacity(0)` は成功する。`DynamicTableDisabled` はエンコーダーストリーム命令 `encode_set_capacity` の意味論であり、`set_table_capacity` には適用しない)
- `set_table_capacity` の呼び出し元 (`src/connection/mod.rs` の `process_control_stream`) はクランプ済み (`use_capacity = min(...)`) で capacity=0 も許可されるためエラーにならないが、シグネチャ変更に合わせて更新する (既存テスト・pbt・fuzz の呼び出し元更新も必要)
- `insert` 系は変更しない (呼び出し側がピア上限を尊重して呼ぶ。encode() 側の防御で不正なワイヤ出力を防ぐ)

## 完了条件

- `max_table_capacity = 0` の状態で `encode()` を呼んでも panic せず、不正なフィールドセクションを生成しない
- ピア上限を超える `set_table_capacity` がエラーになる
- テストが追加される
- CHANGES.md の `## develop` セクションに変更履歴が記載される (`set_table_capacity` の戻り値型変更は公開 API の後方互換のない変更のため [CHANGE] 種別)
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/qpack/encoder.rs` (`DynamicEncoder::encode` / `set_table_capacity` / `set_max_table_capacity` / `insert`)
- `src/connection/mod.rs` (`process_control_stream` の `set_table_capacity` 呼び出し)
- `pbt/tests/prop_qpack/main.rs` / `fuzz/fuzz_targets/fuzz_qpack.rs` (`set_table_capacity` 呼び出しのシグネチャ変更に伴う更新)
- 一次資料: `refs/h3/rfc9204.txt` Section 3.2.3
