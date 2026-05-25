# 0069: QPACK エンコーダーの複数の問題 (ダブルアック API/Post-Base/RIC)

- Priority: Medium
- Created: 2026-05-14
- Model: deepseek-v4-pro
- Branch: feature/fix-qpack-encoder-issues

## 目的

`src/qpack/encoder.rs` に 3 つの問題がある。

## 優先度根拠

Medium: 問題 1 は API 設計の改善であり現状は呼び出し元で正しく処理されている。問題 2 は符号化効率の欠損（正しいが非効率な出力）。問題 3 は理論上のエンコーダー/デコーダー不整合（`max_table_capacity == 0` 時に `req_insert_count != 0` は実運用で発生しない）。いずれも即座のデータ破損にはつながらないが、堅牢性向上のため修正すべき。

## 現状

### 問題 1: ack_section の戻り値型が bool で意図が不明確

`encoder.rs:397` の `ack_section` は `bool` を返す。`false` は「ack 対象のセクションが存在しない」ことを意味し、RFC 9204 Section 4.4.1 に基づき `QPACK_DECODER_STREAM_ERROR` として扱うべき状態を表す。

現在の呼び出し元 (`connection/mod.rs:2462`) は `!ack_section(stream_id)` で適切にエラーを生成しており、RFC の MUST 要件は**満たされている**。しかし `bool` 返却では API のセマンティクスが不明瞭であり、将来の呼び出し元が戻り値を無視するリスクがある。

### 問題 2: Post-Base 参照がエンコードできず None を返している

`encoder.rs:640-641` と `678-679` で `absolute_index >= base` の場合に `None` を返している。RFC 9204 Section 4.5.3 (Post-Base Indexed Field Line) / Section 4.5.5 (Literal Field Line with Post-Base Name Reference) を使えばエンコード可能だが未実装。

現状はリテラル表現にフォールバックするため正しい出力は生成されるが、動的テーブルの活用効率が下がる。

### 問題 3: encode_required_insert_count が max_entries=0 時に固定値 1 を返す

`encoder.rs:566-567` で `max_entries == 0` かつ `req_insert_count != 0` の場合に `return 1` としている。デコーダー側 (`decoder.rs`) は `max_entries == 0` 時に RIC != 0 を `DecodeFailed` とするため、エンコーダー/デコーダー間で不整合が生じる。

実運用では `max_table_capacity == 0`（つまり動的テーブル未使用）の場合に `req_insert_count != 0` は発生しないため、この不整合が顕在化することは通常ない。ただし防御的に修正すべき。

## 設計方針

### 修正 1: ack_section の戻り値を Result 化

```rust
// 修正前:
pub fn ack_section(&mut self, stream_id: u64) -> bool

// 修正後:
pub fn ack_section(&mut self, stream_id: u64) -> Result<(), QpackError>
```

ダブルアック時（対象セクションなし）は `Err(QpackError::DecodeFailed)` を返す。呼び出し元は `?` 演算子でエラーを伝播できる。

### 修正 2: Post-Base Indexed / Post-Base Name Reference を実装

`absolute_index >= base` の場合:
- `encode_indexed_field_dynamic`: Post-Base Indexed Field Line (`0001` prefix, RFC 9204 Section 4.5.3)
- `encode_literal_with_name_ref_dynamic`: Literal Field Line with Post-Base Name Reference (`0000` prefix, RFC 9204 Section 4.5.5)

Post-Base Index の計算: `absolute_index - base`

### 修正 3: max_entries=0 時は assert またはエラー

`max_entries == 0` かつ `req_insert_count != 0` は不変条件違反。`debug_assert!` で検出し、リリースビルドでは `req_insert_count` を 0 として扱う（安全側にフォールバック）。

## テスト戦略

### 問題 1

単体テスト: track_section していないストリーム ID に対する ack_section がエラーを返すことを確認。

### 問題 2

PBT: 動的テーブルにエントリを挿入後、base を適切に設定して Post-Base 参照が生成されることを既存の `prop_qpack.rs` のラウンドトリップテストで確認（Post-Base を含むエンコード結果がデコードで元に戻ること）。

### 問題 3

単体テスト: `max_table_capacity == 0` の状態で `encode_required_insert_count` に非ゼロ値を渡し、適切にハンドリングされることを確認。

## 完了条件

- `ack_section` が `Result<(), QpackError>` を返すこと
- Post-Base Indexed / Post-Base Name Reference が実装されていること
- `max_entries == 0` 時の RIC エンコードが安全にハンドリングされること
- 既存テスト (`cargo test`) と相互運用テストが全て pass すること
- CHANGES.md にエントリが追記されていること

## 後方互換性

- `ack_section` の戻り値型変更: `pub` メソッドだが内部使用のみ（外部クレートは QPACK encoder を直接操作しない）。`[CHANGE]` は不要。
- Post-Base 実装: 出力フォーマットが変わるがデコーダーは既に Post-Base をサポートしているため互換性に影響なし。
- RIC 修正: `max_table_capacity == 0` 時のエッジケース修正であり通常の使用に影響なし。

## 影響範囲

- `src/qpack/encoder.rs`: `ack_section` (397行)、`encode_required_insert_count` (560行)、`encode_indexed_field_dynamic` (634行)、`encode_literal_with_name_ref_dynamic` (671行)
- `src/connection/mod.rs`: `ack_section` の呼び出し元 (2462行) の `bool` チェックを `?` に変更

## RFC 根拠

- RFC 9204 Section 4.4.1: Section Acknowledgment — 重複 ack は QPACK_DECODER_STREAM_ERROR
- RFC 9204 Section 4.5.1.1: Required Insert Count のエンコード/デコード規定
- RFC 9204 Section 4.5.3: Post-Base Indexed Field Line
- RFC 9204 Section 4.5.5: Literal Field Line with Post-Base Name Reference

## CHANGES.md エントリ案

```
- [FIX] QPACK エンコーダーの ack_section を Result 化し、Post-Base 参照エンコードを実装し、RIC エンコードの max_entries=0 時のエッジケースを修正する
  - @担当者
```
