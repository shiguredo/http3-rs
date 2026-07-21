# テストメッセージの英語混在を日本語に統一する

- Priority: Medium
- Created: 2026-06-15
- Model: Opus 4.7
- Branch: feature/fix-test-messages-japanese
- Polished: 2026-07-21

## 目的

`tests/`, `pbt/tests/`, `src/*/mod.rs` (内部 `#[cfg(test)] mod tests`) の `assert!` / `assert_eq!` / `prop_assert!` / `panic!` の補助メッセージで英語が混在している。AGENTS.md「テストメッセージは全て日本語にすること」規約違反のため、一括日本語化する。

## 優先度根拠

Medium。AGENTS.md の明示的な規約違反。プロジェクト全体でテスト失敗時のメッセージ言語が統一されていない状態は読み手の混乱を招く。

## 現状

代表箇所:

- `tests/test_webtransport_flow_control.rs:49 "should be blocked at limit"`
- `tests/integration.rs:206 "GOAWAY id increase must be rejected"`
- `pbt/tests/prop_frame.rs:161,213,246,265,294,332 "Expected DATA frame", "Expected HEADERS frame" 他`
- `pbt/tests/prop_varint.rs:64 "encode should succeed for any VarInt"`
- `pbt/tests/prop_qpack/main.rs:201,345,351,704,741,813 "Expected InsertWithLiteralName instruction" 他`
- `pbt/tests/prop_webtransport/session.rs:721 "process_capsule failed: {:?}"`
- `pbt/tests/prop_webtransport/stream.rs:129 "Stream ID must be either client or server initiated"`
- `src/settings.rs:622 panic!("expected Unknown")`
- `src/qpack/decoder.rs:793,825,852 panic!("unexpected Blocked")`
- `src/stream/request.rs:663 panic!("expected Headers")`
- `src/connection/mod.rs:5520 panic!("expected BufferedStreamRejected, got {other:?}")`
- `src/frame/decoder.rs:287 panic!("expected DATA, got {other:?}")`
- `src/qpack/encoder_stream.rs:539 panic!("Unexpected instruction")`

AGENTS.md:

> テストメッセージは全て日本語にすること

## 設計方針

- `grep -nE '(assert!\(|assert_eq!\(|prop_assert!\(|panic!\()' tests/ pbt/ src/ | grep '"[A-Z][a-z]'` で英語メッセージを抽出
- 機械翻訳ではなく、テストの意図を読んで日本語化する
- panic! のメッセージも対象 (テストコード内のすべての文字列リテラルメッセージ)
- 文字列引用 (RFC ABNF など) は対象外

## 完了条件

- 英語メッセージが日本語に統一される
- 文字列引用 (仕様文面 / ヘッダー値) は変更しない
- テスト動作は変わらない
- `make fmt && make clippy && make check` が通る

## 解決方法

ファイルごとに以下を実施:

```rust
// 変更前
prop_assert_eq!(frame_type, FrameType::Data, "Expected DATA frame");
// 変更後
prop_assert_eq!(frame_type, FrameType::Data, "DATA フレームが期待される");
```

### 関連ファイル

- 修正対象: `tests/**/*.rs`, `pbt/tests/**/*.rs`, `src/*/mod.rs` の `#[cfg(test)] mod tests`, `src/connection/mod.rs:5520`, `src/frame/decoder.rs:287`, `src/qpack/encoder_stream.rs:539` 等
- 規約: `AGENTS.md`
