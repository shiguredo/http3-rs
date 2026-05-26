# 0078: src/validation.rs が過大 — モジュール分割が必要

- Priority: Low
- Created: 2026-05-14
- Completed: 2026-05-26
- Model: deepseek-v4-pro
- Branch: feature/refactor-split-validation-module

## 目的

`src/validation.rs` が約 1845 行に肥大化している。リクエスト検証、レスポンス検証、トレーラー検証、content-length 整合性検証、authority 検証が同一ファイルに混在しており保守性が低い。

## 優先度根拠

Low: 機能的な問題はない。コードの見通し改善と保守性向上が目的。

## 現状

- `src/validation.rs`: 約 1845 行
- `validate_request_headers`: 約 550 行の単一関数
- レスポンス/トレーラー/content-length/authority 検証が混在
- インラインテスト約 1100 行（issue 0074 で分離予定）

## 設計方針

`src/validation/` ディレクトリモジュールに分割する:

1. `src/validation/mod.rs` — 共通ヘルパー (`is_valid_*` 関数群)、公開 API の re-export
2. `src/validation/request.rs` — `validate_request_headers`
3. `src/validation/response.rs` — `validate_response_headers`
4. `src/validation/trailer.rs` — `validate_trailer_headers`
5. `src/validation/content_length.rs` — `validate_content_length`

注: issue 0074（インラインテスト分離）を先に実施すること。テスト分離後にモジュール分割を行う方が安全。

## 完了条件

- `src/validation.rs` が `src/validation/` ディレクトリモジュールに変換されていること
- 公開 API に変更がないこと（re-export で互換性維持）
- `cargo test` が全て pass すること

## 影響範囲

- `src/validation.rs` → `src/validation/` ディレクトリ（mod.rs + サブモジュール）
- `src/lib.rs`: `mod validation` の参照パスは変更なし

## 解決方法

issue 0074 (validation.rs のインラインテスト分離、コミット `c0d540b`) の完了により、`src/validation.rs` は約 1845 行から **792 行** に縮小した。792 行・20 関数のファイルを 5 つのサブモジュールに分割する必要性は解消されたため、本 issue はクローズする。
