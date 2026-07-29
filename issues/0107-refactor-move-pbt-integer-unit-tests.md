# `pbt/tests/prop_qpack/integer.rs` の単体テストを `tests/` か `src/` 配下に移す

- Priority: High
- Created: 2026-06-15
- Completed: 2026-07-30
- Model: Opus 4.7
- Branch: feature/refactor-move-pbt-integer-unit-tests
- Polished: 2026-07-21

## 目的

`pbt/tests/prop_qpack/integer.rs:96-191` に `proptest!` でない `#[test]` 関数が 7 個混入している (`test_decode_empty_buffer`, `test_encode_zero`, `test_encode_max_prefix_boundary`, `test_encode_slice_buffer_too_small`, `test_decode_overflow_protection`, `test_decode_truncated_multi_byte`, `test_max_decodable_value_roundtrip`)。AGENTS.md「pbt 以下に unittest を書かないこと」「unittest は pbt で実現できないものだけを書くこと」に違反しているため、適切な場所に移動する。

## 優先度根拠

High。AGENTS.md のテスト戦略規約 (PBT / 単体テスト / fuzzing の役割分担) を明示的に違反している。pbt クレートの責務境界が曖昧になるとカバレッジ計測やテスト戦略の議論が破綻するため、放置できない。

## 現状

`pbt/tests/prop_qpack/integer.rs:96-191` に以下の `#[test]` が存在:

- `test_decode_empty_buffer`
- `test_encode_zero`
- `test_encode_max_prefix_boundary`
- `test_encode_slice_buffer_too_small`
- `test_decode_overflow_protection`
- `test_decode_truncated_multi_byte`
- `test_max_decodable_value_roundtrip`

AGENTS.md (テストの役割分担):

> - PBT: 型情報（Strategy）に基づいて入力を生成し、プロパティを検証する（ラウンドトリップ等）
> - Fuzzing: 任意入力に対するクラッシュ耐性（パニック安全性）
> - 単体テスト: 意図的なエラーパス、境界値など PBT で実現できないケース
> - PBT でカバーできるものを単体テストで書かない

> pbt 以下に unittest を書かないこと
> unittest は pbt で実現できないものだけを書くこと
> 単体テストのファイル名は `tests/test_<module>.rs` とし、`src/<module>.rs` に対応させること

## 設計方針

- 移動先候補:
  - 案 A: `tests/test_qpack_integer.rs` 新規作成 (`src/qpack/integer.rs` に対応)
  - 案 B: `src/qpack/integer.rs` 内部の `#[cfg(test)] mod tests` に集約
- 単体テストは「PBT で代替不可能なエラーパス / 境界値」に該当するため、`tests/` 配下が原則。AGENTS.md の例にも従う形となる
- `test_max_decodable_value_roundtrip` は PBT 化可能なら PBT に組み込み、その他のエラーパス検査は `tests/test_qpack_integer.rs` に移す
- `pbt/tests/prop_qpack/integer.rs` には `proptest!` のみを残す

## 完了条件

- `pbt/tests/prop_qpack/integer.rs` 内の `#[test]` 関数が `tests/test_qpack_integer.rs` (新規) または PBT 化されて移動される
- `pbt/tests/prop_qpack/integer.rs` には `proptest!` のみが残る
- 移動先で全テストがパスする
- `cargo test --workspace` が全てパスする
- `make fmt && make clippy && make check` が全て通る

## 解決方法

1. `tests/test_qpack_integer.rs` を新規作成し、7 個の `#[test]` を移す
2. ラウンドトリップ系 (`test_max_decodable_value_roundtrip`) は PBT 化が可能なら `pbt/tests/prop_qpack/integer.rs` の `proptest!` に統合
3. `pbt/tests/prop_qpack/integer.rs` の単体テストブロックを削除
4. `cargo test` で確認

### 関連ファイル

- 修正元: `pbt/tests/prop_qpack/integer.rs:96-191`
- 新規作成: `tests/test_qpack_integer.rs`
- 対応モジュール: `src/qpack/integer.rs`
- 規約: `AGENTS.md` (テスト戦略セクション)

## 解決方法

コミット f5b5260 で実装した。pbt/tests/prop_qpack/integer.rs に混入していた非 proptest の #[test] 関数を適切な場所に移動し、AGENTS.md の「pbt 以下に unittest を書かないこと」規約に準拠させた。
