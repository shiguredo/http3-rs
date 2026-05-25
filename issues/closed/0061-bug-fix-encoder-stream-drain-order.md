# 0061: エンコーダーストリームでバッファ消費後にテーブル操作 — 処理順序の誤り

- Priority: Medium
- Created: 2026-05-14
- Completed: 2026-05-26
- Model: deepseek-v4-pro
- Branch: feature/fix-encoder-stream-drain-order

## 目的

`src/qpack/encoder_stream.rs` の 3 メソッドにおいて、`self.recv_buffer.drain(..consumed)` を実行した後にテーブル操作およびテーブル参照を行っている。テーブル操作・参照が失敗した場合、既にバッファからデータが削除されているため、エラー発生時点の受信データが失われる。

RFC 9204 に基づき、エンコーダーストリーム上のエラーは接続エラー (`QPACK_ENCODER_STREAM_ERROR`, 0x0201) となり接続全体が破棄されるため、破損したバッファ状態が後続処理に波及することはない。しかし処理順序として誤っており、将来のコード変更（エラー時のログ出力やデバッグ機能追加等）で問題が顕在化するリスクがある。

## 優先度根拠

Medium: 現状は接続エラーで即座に閉じられるため実害は発生しないが、防御的プログラミングの観点から修正すべき。修正自体は 3 箇所の行入れ替えのみで低リスク。

## 現状

以下の 3 メソッドで `drain` がテーブル操作・テーブル参照の**前**に実行されている:

| メソッド | 行 | drain 後の失敗しうる操作 | RFC 9204 |
|----------|-----|--------------------------|----------|
| `decode_insert_with_name_ref` | 276 | `STATIC_TABLE.get()` / `table.get_by_relative_index_encoder()` / `table.insert()` | Section 4.3.2 |
| `decode_insert_with_literal_name` | 316 | `table.insert()` | Section 4.3.3 |
| `decode_duplicate` | 336 | `table.duplicate()` | Section 4.3.4 |

`decode_set_capacity` (256行) にも同様の drain → `table.set_capacity` パターンがあるが、`set_capacity` は infallible であり、かつ `max_table_capacity` 超過チェック (252行) が drain より前に走るため修正対象外。

## 設計方針

テーブル操作・テーブル参照を成功させてから `drain` を実行するように順序を入れ替える。

### decode_insert_with_name_ref の修正

```rust
// 修正前 (276行):
self.recv_buffer.drain(..consumed);

let name = if is_static {
    STATIC_TABLE
        .get(name_index as usize)
        .ok_or(QpackError::InvalidIndex(name_index))?
        .name()
        .to_vec()
} else {
    table
        .get_by_relative_index_encoder(name_index)
        .ok_or(QpackError::InvalidIndex(name_index))?
        .name
        .clone()
};

table
    .insert(name, value.clone())
    .ok_or(QpackError::DecodeFailed)?;

// 修正後: テーブル参照・テーブル操作を先に行い、成功後に drain
let name = if is_static {
    STATIC_TABLE
        .get(name_index as usize)
        .ok_or(QpackError::InvalidIndex(name_index))?
        .name()
        .to_vec()
} else {
    table
        .get_by_relative_index_encoder(name_index)
        .ok_or(QpackError::InvalidIndex(name_index))?
        .name
        .clone()
};

table
    .insert(name, value.clone())
    .ok_or(QpackError::DecodeFailed)?;

self.recv_buffer.drain(..consumed);
```

### decode_insert_with_literal_name の修正

```rust
// 修正前 (316行):
self.recv_buffer.drain(..consumed);
table
    .insert(name.clone(), value.clone())
    .ok_or(QpackError::DecodeFailed)?;

// 修正後:
table
    .insert(name.clone(), value.clone())
    .ok_or(QpackError::DecodeFailed)?;
self.recv_buffer.drain(..consumed);
```

### decode_duplicate の修正

```rust
// 修正前 (336行):
self.recv_buffer.drain(..consumed);
table
    .duplicate(relative_index)
    .ok_or(QpackError::InvalidIndex(relative_index))?;

// 修正後:
table
    .duplicate(relative_index)
    .ok_or(QpackError::InvalidIndex(relative_index))?;
self.recv_buffer.drain(..consumed);
```

## 再現手順

不正な `relative_index` で Duplicate を実行し、エラー発生時にバッファが空になることを確認する:

```rust
let mut receiver = EncoderStreamReceiver::new();
receiver.set_max_table_capacity(4096);
let mut table = DynamicTable::with_capacity(4096);
table.insert(b"name".to_vec(), b"value".to_vec()); // abs=0

// Duplicate with relative_index=5 (存在しないインデックス)
// 命令: 00000101 (5-bit prefix, value=5)
receiver.receive(&[0x05]);

// process は Err(QpackError::InvalidIndex(5)) を返す
// 現在の実装では drain 後に duplicate が呼ばれるため、
// recv_buffer は既に空になっている
let result = receiver.process(&mut table);
assert_eq!(result, Err(QpackError::InvalidIndex(5)));

// 現状（修正前）: バッファが空になっている（drain 済み）
assert!(receiver.buffer().is_empty());
```

## エラーパス一覧

修正対象 3 メソッドで drain 後に発生しうるエラーパス（drain 前に発生する `decode_integer` / `decode_string` の `BufferTooShort` 等は含まない）:

| メソッド | エラーパス | エラー種別 |
|----------|-----------|-----------|
| `decode_insert_with_name_ref` | 静的テーブル不正インデックス (`STATIC_TABLE.get()` → `None`) | `InvalidIndex` |
| `decode_insert_with_name_ref` | 動的テーブル不正相対インデックス (`get_by_relative_index_encoder()` → `None`) | `InvalidIndex` |
| `decode_insert_with_name_ref` | 容量オーバーで insert 失敗 (`table.insert()` → `None`) | `DecodeFailed` |
| `decode_insert_with_literal_name` | 容量オーバーで insert 失敗 (`table.insert()` → `None`) | `DecodeFailed` |
| `decode_duplicate` | 不正相対インデックス (`table.duplicate()` → `None`) | `InvalidIndex` |

## テスト戦略

意図的なエラーパスの検証であるため、単体テストで対応する（AGENTS.md: PBT はラウンドトリップ等のプロパティ検証に使用し、エラーパスは単体テストの責務）。

`tests/test_qpack_encoder_stream.rs` を**新規作成**し、上記エラーパス一覧の 5 ケースすべてについて:
- エラーが正しく返ること
- エラー発生時に `receiver.buffer()` が空でないこと（drain されていないこと）

を検証する。

Fuzzing: 不要（意図的エラーパスは単体テストでカバー）。

## 完了条件

- 3 メソッドの drain が全テーブル操作・テーブル参照の後に移動していること
- 上記 5 エラーパスの単体テストが全て pass すること
- 既存テスト (`cargo test`) が全て pass すること
- CHANGES.md にエントリが追記されていること

## 後方互換性

外部 API に変更なし。`process()` の戻り値型 (`Result<Option<EncoderInstruction>, QpackError>`) は変更されない。エラー発生時の `recv_buffer` 状態が変わる（空 → 未消費）が、RFC 9204 Section 2.2.3 / Section 3.2.2 に基づきエラー時は接続エラーとして接続が閉じられるため互換性に影響しない。

## 影響範囲

- `src/qpack/encoder_stream.rs`: 276行, 316行, 336行
- 接続エラー処理 (`connection/mod.rs` の `process_encoder_stream` 関数, 2348行) は `map_err(|_| Error::ConnectionError(ErrorCode::QpackEncoderStreamError))` で QpackError を接続エラーに変換しており、drain 順序変更の影響を受けない

## RFC 根拠

- RFC 9204 Section 4.3.2 (Insert with Name Reference): エンコーダー命令の仕様
- RFC 9204 Section 4.3.3 (Insert with Literal Name): エンコーダー命令の仕様
- RFC 9204 Section 4.3.4 (Duplicate): エンコーダー命令の仕様
- RFC 9204 Section 2.2.3 (Invalid References): エンコーダー命令内の無効な動的テーブル参照は QPACK_ENCODER_STREAM_ERROR で接続エラーとする MUST 規定
- RFC 9204 Section 3.1 (Static Table): 静的テーブルの範囲外インデックスをエンコーダーストリームで受信した場合 QPACK_ENCODER_STREAM_ERROR とする MUST 規定
- RFC 9204 Section 3.2.2 (Dynamic Table Capacity and Eviction): 容量超過エントリの追加は QPACK_ENCODER_STREAM_ERROR で接続エラーとする MUST 規定
- RFC 9204 Section 6 (Error Handling): QPACK_ENCODER_STREAM_ERROR (0x0201) の定義

## 解決方法

`src/qpack/encoder_stream.rs` の 3 メソッドで `self.recv_buffer.drain(..consumed)` をテーブル操作・テーブル参照の後に移動した:

- `decode_insert_with_name_ref`: 静的/動的テーブル参照と insert の後に drain
- `decode_insert_with_literal_name`: insert の後に drain
- `decode_duplicate`: duplicate の後に drain

`tests/test_qpack_encoder_stream.rs` を新規作成し、以下の 5 エラーパスについて「正しいエラーが返ること」および「バッファが drain されていないこと」を検証する単体テストを追加した:

1. 静的テーブル不正インデックス (InvalidIndex)
2. 動的テーブル不正相対インデックス (InvalidIndex)
3. 名前参照挿入の容量超過 (DecodeFailed)
4. リテラル名挿入の容量超過 (DecodeFailed)
5. 複製の不正相対インデックス (InvalidIndex)

## CHANGES.md エントリ案

```
- [FIX] QPACK エンコーダーストリームレシーバーでテーブル操作前にバッファを drain していた処理順序を修正する
  - @voluntas
```
