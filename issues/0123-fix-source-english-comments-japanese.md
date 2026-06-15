# `src/qpack` と `src/connection` と `src/webtransport` の英語コメント混在を是正する

- Priority: Medium
- Created: 2026-06-15
- Completed:
- Model: Opus 4.7
- Branch: feature/fix-source-english-comments-japanese
- Polished:

## 目的

`src/qpack/encoder.rs`, `decoder.rs`, `connection/mod.rs`, `webtransport/stream.rs`, `capsule.rs` 等で英語単独コメント (`// Literal Field Line ...`, `// Name index`, `// Value`, `// Stream Type`, `// Control Stream` 等) が多数残存している。CLAUDE.md「コメントは全て日本語にすること」規約違反を是正する。

## 優先度根拠

Medium。CLAUDE.md の明示的な規約違反。RFC のフィールド名引用などは原文を残す必要があるが、その場合も日本語の補足説明を併記すべき。

## 現状

代表箇所:

- `src/qpack/encoder.rs:79,122,229,232,560,592,648,686` `// Literal Field Line ...`, `// Value`, `// Literal with Name Reference`
- `src/qpack/decoder.rs:66,164,175,192,199,472,503,533,546,560,584,596,613,619` `// Required Insert Count`, `// Name index`, `// Value`, `// Decode name`
- `src/connection/mod.rs:1311,1330,1342,3074,3086,3106,4655` `// Control Stream`, `// QPACK Encoder Stream`, `// Open`, `// Data`, `// End`, `// session_id`
- `src/webtransport/stream.rs:122,131,160,169` `// Stream Type`, `// Session ID`, `// Signal Value`
- `src/webtransport/capsule.rs:305,311` `// Capsule Type`, `// Length`
- `src/validation.rs:217,242,323,402,460,466,485,523,618` 多数の RFC ABNF 引用

CLAUDE.md:

> コメントは全て日本語にすること

## 設計方針

- RFC / 仕様の英語原文引用は残す (`// "field-value = ..."` のような ABNF 引用は変更しない)
- 日本語による補足を必須化する
  - 例: `// Stream Type` → `// Stream Type (ストリームタイプ識別子: RFC 9114 Section 6.2)`
- 単独英語コメント (`// Value`, `// Open` 等) は日本語に置き換える
- grep ベースで機械的に検出し、レビューで適切な日本語化を行う

## 完了条件

- src 配下の `.rs` ファイルから単独英語コメントが消える (RFC 引用 + 日本語補足の形に統一)
- `make fmt && make clippy && make check` が通る

## 解決方法

各ファイルを順にレビューし、ガイドラインに沿って書き換える。コミットはファイル単位で分けると読みやすい。

### 関連ファイル

- 修正対象: `src/qpack/encoder.rs`, `src/qpack/decoder.rs`, `src/connection/mod.rs`, `src/webtransport/stream.rs`, `src/webtransport/capsule.rs`, `src/validation.rs` ほか
- 規約: `CLAUDE.md`
