# QPACK の `Encoder`/`DynamicEncoder` と `Decoder`/`DynamicDecoder` の重複を解消する

- Priority: Medium
- Created: 2026-06-15
- Model: Opus 4.7
- Branch: feature/refactor-merge-encoder-decoder-static-and-dynamic
- Polished: 2026-07-21

## 目的

`src/qpack/encoder.rs` と `src/qpack/decoder.rs` の静的専用 (`Encoder`, `Decoder`) と動的対応 (`DynamicEncoder`, `DynamicDecoder`) に約 200 行の実装重複がある。`encode_string` / `encode_string_with_prefix` / `encode_indexed_field` / `encode_literal_with_*` / `decode_string` 等が両側にコピペで存在する。`Encoder` / `Decoder` を廃止して `DynamicEncoder(max_table_capacity=0)` / `DynamicDecoder(max_table_capacity=0)` に統合する。

## 優先度根拠

Medium。Don't live with broken windows。重複の片方を直し忘れるとバグ (例: 0097 の境界判定バグも `Encoder` のみ存在) の温床。テスト数も倍増させる原因。

## 現状

- `Encoder::encode_string` (166-214) と `DynamicEncoder::encode_string` (729-776) が完全同一
- `Encoder::encode_string_with_prefix` も同様
- `Encoder::encode_indexed_field` (88-100) と `DynamicEncoder::encode_indexed_field_static` (597-603) が機能的に同一 (本実装で 0097 のバグも露見)
- `Encoder::encode_literal_with_name_ref` と `encode_literal_with_literal_name` も同様
- `Decoder` (38-251) と `DynamicDecoder` (258-638) も静的経路がほぼ同一

`Encoder` は「静的テーブルだけ使う」`DynamicEncoder` の特殊形であり、機能的サブセット。

## 設計方針

- `Encoder` を廃止し、`pub type Encoder = DynamicEncoder;` のエイリアスにするか、ファクトリ関数 `DynamicEncoder::static_only()` で代替する
- `Decoder` も同様に廃止し `DynamicDecoder` に統合
- `lib.rs` の doc 例を `DynamicEncoder` 利用に書き換える
- 重複ヘルパー (`encode_string` 等) は `qpack/wire.rs` (新規) に集約する選択肢もある
- 0097 / 0099 の修正と並行して進める (重複が解消されればその issue で 1 箇所修正で済む)
- 破壊的変更のため `CHANGES.md` に `[CHANGE]` 追加

## 完了条件

- `Encoder` / `Decoder` が削除またはエイリアスになる
- 重複コード約 200 行が解消される
- 既存テスト / PBT / fuzz が全てパスする
- `lib.rs` doc 例が更新される
- `make fmt && make clippy && make check` が通る

## 解決方法

1. `DynamicEncoder` / `DynamicDecoder` に必要な API を整理 (静的のみで使える初期化方法)
2. `Encoder` 利用箇所を `DynamicEncoder` に置き換える
3. `Encoder` 型を削除またはエイリアスに変更
4. `Decoder` も同様
5. ヘルパー関数を必要に応じて自由関数化

### 関連ファイル

- 修正対象: `src/qpack/encoder.rs`, `src/qpack/decoder.rs`, `src/qpack/mod.rs`, `src/lib.rs`
- 関連 issue: 0097 (境界判定バグ)
- `CHANGES.md` 追記必要
