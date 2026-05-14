# 0078: src/validation.rs が約 1845 行で過大 — モジュール分割が必要

Created: 2026-05-14
Model: deepseek-v4-pro

## 概要

`src/validation.rs` が約 1845 行に肥大化している。

- `validate_request_headers`: 約 550 行の単一関数
- レスポンス検証、トレーラー検証、content-length 整合性検証、authority 検証が同ファイルに混在
- テストも 1100 行以上ありファイルが過大

## 修正方針

以下のように分割を検討する:
1. `src/validation/mod.rs` — 共通部分、`HeaderField` trait、`is_valid_*` ヘルパー
2. `src/validation/request.rs` — `validate_request_headers`
3. `src/validation/response.rs` — `validate_response_headers`
4. `src/validation/trailer.rs` — `validate_trailer_headers`
5. `src/validation/content_length.rs` — `validate_content_length`
6. `src/validation/authority.rs` — `is_valid_authority` 等

## 影響範囲

- `src/validation.rs` (1845 行 → 削除)
- 新規: `src/validation/` ディレクトリ以下
