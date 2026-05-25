# 0083: lib.rs の pub use 一覧を整理し内部実装詳細の露出を減らす

- Priority: Low
- Created: 2026-05-14
- Model: deepseek-v4-pro
- Branch: feature/refactor-lib-pub-use-cleanup

## 目的

`src/lib.rs` の `pub use` リストで内部実装詳細が公開 API として再公開されている。これらを公開し続けると、内部型の変更が semantic versioning のメジャーバンプを強制する。

## 優先度根拠

Low: 機能に影響しないが、API サーフェスの最小化と将来の semver 互換性維持コスト削減のために対応すべき。

## 現状

`src/lib.rs` の `pub use` で以下の内部実装詳細が公開されている可能性:

- `DecodeOutput` — connection 内部で使用する型
- `EncoderInstruction` — encoder stream 内部の型
- `DynamicEntry` — 動的テーブルエントリの内部表現

## 設計方針

各再公開について以下の判断を行う:

1. **外部クレートから使用されている** → 公開を維持
2. **tests / examples のみで使用** → `internal-test` フィーチャー限定公開に変更
3. **内部のみで使用** → `pub use` から削除

判断にあたっては `examples/wt_server` と `interop/` クレートでの使用状況を確認する。

## 完了条件

- 内部実装詳細の `pub use` が削除または `internal-test` 限定に変更されていること
- `examples/wt_server` が正常にコンパイルできること
- `cargo test` が全て pass すること

## 後方互換性

公開 API から型を削除するため後方互換のない変更。`[CHANGE]` として記録する。ただし、実際に外部ユーザーが使用している可能性は低い内部型のみを対象とする。

## 影響範囲

- `src/lib.rs`: `pub use` リストの整理

## CHANGES.md エントリ案

```
- [CHANGE] lib.rs から内部実装詳細の pub use を削除し公開 API サーフェスを最小化する
  - @担当者
```
