# `validation.rs` の過剰コメントと重複検査を整理する

- Priority: Medium
- Created: 2026-06-15
- Model: Opus 4.7
- Branch: feature/refactor-trim-validation-comments-and-duplication
- Polished: 2026-07-21

## 目的

`src/validation.rs` に以下の負債が蓄積している。整理する。

- L7-28 の冗長な設計意図解説 (22 行)
- `is_valid_authority` (271-311) と `is_valid_connect_authority` (318-356) で 90% の重複
- Extended CONNECT 経路と末尾共通経路で `:authority` / `Host` 整合チェックの二重 (L495-501 と L576-580)
- `validate_request_headers` 関数 (270 行) の責務混在

## 優先度根拠

Medium。コードの理解コストが高くなり、修正時の波及範囲も読みづらい。レビュアーが「同じ目的の処理が複数箇所に書かれている」と指摘済み。

## 現状

L7-28 のコメント:

```
//! `qpack::header` と本モジュールの責務分離は…
//! (22 行の解説)
```

L271-311 の `is_valid_authority` と L318-356 の `is_valid_connect_authority` は authority のパース部分 (IPv6 角括弧 / port 検出 / `is_valid_reg_name`) がほぼ同一で、`require_port: bool` のみが差。

`validate_request_headers` 内に Extended CONNECT 分岐 (494-525) と末尾共通検査 (570-625) で `:authority` 空文字 / `Host` 整合の重複検査が存在。

## 設計方針

- L7-28 の解説を 4〜5 行に圧縮 (要点のみ)
- `validate_authority(value: &[u8], require_port: bool) -> bool` の統一関数を導入し、両 `is_valid_*_authority` を委譲
- `validate_request_headers` を以下に分解:
  - `validate_extended_connect`
  - `validate_plain_connect`
  - `validate_non_connect_request`
  - 共通部 (authority / Host 整合) は最後に 1 回だけ呼ぶ
- 既存テスト (`tests/test_validation.rs`) はそのままパスすることを確認

## 完了条件

- L7-28 のコメントが 5 行以下に圧縮される
- `validate_authority(value, require_port)` が共通実装になる
- `validate_request_headers` が 100 行以下に縮む
- 既存テストがパスする
- `make fmt && make clippy && make check` が通る

## 解決方法

各 sub-validate 関数に分解し、共通検査をモジュール内 private で集約。

### 関連ファイル

- 修正対象: `src/validation.rs:7-28, 271-356, 359-628`
- 関連テスト: `tests/test_validation.rs`
