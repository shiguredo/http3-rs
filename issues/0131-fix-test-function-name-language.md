# テスト関数名の言語表記を統一する

- Priority: Low
- Created: 2026-06-15
- Model: Opus 4.7
- Branch: feature/fix-test-function-name-language
- Polished: 2026-07-21

## 目的

`src/qpack/encoder.rs` のテスト関数名に日本語 (`ack_section_は未追跡ストリームに対してエラーを返す` 等) が混入する一方、他のテストファイルは英語名で統一されている。AGENTS.md「テストメッセージは全て日本語にすること」の解釈を統一し、テスト関数名を片方に揃える。

## 優先度根拠

Low。動作には影響しないが、検索性と整合性のために整える。AGENTS.md の規約解釈を明文化する機会でもある。

## 現状

- `src/qpack/encoder.rs:993` `fn ack_section_は未追跡ストリームに対してエラーを返す() { ... }` (日本語名)
- 他のテストファイル: `tests/test_validation.rs`, `tests/test_webtransport_draft_connect.rs`, `pbt/tests/prop_*.rs` などはすべて英語関数名
- panic / assert メッセージは日本語化が AGENTS.md 規約

## 設計方針

選択肢:
- 案 A: テスト関数名を英語に統一 (`fn ack_section_returns_error_for_untracked_stream`) — 既存大多数に合わせる
- 案 B: テスト関数名を日本語に統一 — AGENTS.md「テストメッセージ」を関数名まで拡大解釈

実装と運用の整合性から案 A を推奨。テスト失敗時の assert メッセージは日本語のままにする。

合わせて AGENTS.md または別のドキュメントに「テスト関数名は英語、テスト失敗時メッセージは日本語」というガイドラインを書き加える。

## 完了条件

- `src/qpack/encoder.rs` のテスト関数名が英語化される (もしくは全テストファイルが日本語化される)
- 規約解釈のメモが AGENTS.md に追加される
- `make fmt && make clippy && make check` が通る

## 解決方法

```rust
#[test]
fn ack_section_returns_error_for_untracked_stream() {
    // ...
}
```

### 関連ファイル

- 修正対象: `src/qpack/encoder.rs` の `#[cfg(test)] mod tests` (テスト関数名)
- ドキュメント: `AGENTS.md` (規約追加)
