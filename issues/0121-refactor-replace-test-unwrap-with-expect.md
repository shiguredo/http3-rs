# `tests/` と `pbt/` 配下の `.unwrap()` を `.expect("MESSAGE")` に一括置換する

- Priority: Medium
- Created: 2026-06-15
- Completed:
- Model: Opus 4.7
- Branch: feature/refactor-replace-test-unwrap-with-expect
- Polished:

## 目的

`tests/`, `pbt/tests/`, `pbt/src/lib.rs`, `interop/` で `.unwrap()` が 500 件以上残存している。AGENTS.md「`.unwrap()` ではなく `.expect("MESSAGE")` を使用する」規約に基づき、すべて意図メッセージ付きの `.expect("...")` に置換する。

## 優先度根拠

Medium。テストパニック時のデバッグ性に直結する。テストが失敗したとき「どの unwrap が失敗したのか」をパス + 行番号だけで追うのは負担が大きい。本体コードの `.unwrap()` (issue 0105) と区別して別 issue にする。

## 現状

代表的な数:

- `tests/test_webtransport_draft_connect.rs` 88 件
- `pbt/tests/prop_qpack/main.rs` 49 件
- `pbt/src/lib.rs:65` 1 件 (`VarInt::new(v).unwrap()`)
- `interop/h3` / `interop/wt` 約 215 件

AGENTS.md:

> `.unwrap()` ではなく `.expect("MESSAGE")` を使用する

(テスト本体の方も対象)

## 設計方針

- `.unwrap()` を機械的に `.expect("...")` に置換するのは危険 (適切なメッセージが付かない)
- セマンティック単位で置換: 「絶対に成功する想定」「Strategy が保証するため成功」等を明示
- まず `pbt/src/lib.rs` の strategy ヘルパー (`VarInt::new(v).unwrap()` 等) を最小限の expect 化
- 次に PBT の `prop_assert!` 系で実行されるパスを expect 化 (PBT は失敗時のメッセージが重要)
- テスト単体 (`tests/`) は最後に対応
- `clippy::unwrap_used` lint を `tests/` 配下で有効化することも検討 (CI レベル)
- 大量変更のためレビュー単位を細かく分けてコミット

## 完了条件

- `tests/`, `pbt/`, `interop/` 配下の `.unwrap()` が `.expect("MESSAGE")` に置換される
- メッセージは日本語で、なぜ成功するかを明示する
- 既存テスト・PBT がすべてパスする
- `make fmt && make clippy && make check` が通る

## 解決方法

例:

```rust
// 変更前
let var = VarInt::new(v).unwrap();
// 変更後
let var = VarInt::new(v).expect("Strategy が 0..=MAX 範囲を保証するため必ず Ok");
```

セマンティック単位で grep して置換する。

### 関連ファイル

- 修正対象: `tests/**/*.rs`, `pbt/tests/**/*.rs`, `pbt/src/lib.rs`, `interop/**/*.rs`
- 関連 issue: 0105 (本体コードの `.unwrap()`)
- 規約: `AGENTS.md`
