# 0083: lib.rs の pub use 一覧を整理し内部実装詳細の露出を減らす

Created: 2026-05-14
Model: deepseek-v4-pro

## 概要

`src/lib.rs:59-73` の `pub use` リストで、以下の内部実装詳細が公開 API として
再公開されている:

- `DecodeOutput` — connection 内部で使用する型
- `EncoderInstruction` — encoder stream 内部の型
- `DynamicEntry` — 動的テーブルエントリの内部表現

これらを公開 API として露出させ続けると、内部的な型変更が
semantic versioning のメジャーバンプを強制する。

## 修正方針

各再公開について以下の判断を行う:
1. 外部クレートから使用されている → 公開を維持
2. tests / examples のみで使用 → `pub(crate)` に変更し、テストは内部からアクセス
3. 内部のみで使用 → `pub use` から削除

## 影響範囲

- `src/lib.rs:59-73`
