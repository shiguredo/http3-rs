# content-length 値に + 符号付き数値が受理される

- Created: 2026-08-08
- Completed: 2026-08-23
- Branch: feature/fix-content-length-plus-sign
- Polished: {YYYY-MM-DD}

## 目的

RFC 9110 Section 8.6 の `content-length = 1*DIGIT` 文法に違反する値 (例: `+5`) の受理を防ぐ。

## 現状

- `src/validation.rs` の content-length 検査は `value_str.parse::<u64>()` で値を検証している
- Rust の `str::parse::<u64>` は先頭の `+` を許容するため、`content-length: +5` が正当値として受理される
- RFC 9110 Section 8.6 の `1*DIGIT` に違反する値が RFC 9114 Section 4.1.2 の malformed 判定 (H3_MESSAGE_ERROR) をすり抜ける

## 設計方針

- 全バイトが `is_ascii_digit` であることを検査してから `parse::<u64>` する (または `1*DIGIT` の手動検査に置き換える)

## 完了条件

- `content-length: +5` 等の不正値が拒否される
- テストが追加される
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/validation.rs` (content-length 検査関数)
- 一次資料: `refs/rfc9110.txt` Section 8.6

### 修正内容

- `src/validation.rs` の `validate_content_length` で、`parse::<u64>` の前に値の全バイトが `is_ascii_digit` であることを検査し、`1*DIGIT` (RFC 9110 Section 8.6) に違反する値 (例: `+5`, `-1`) を malformed として拒否するように修正した
- `tests/test_validation.rs` に `test_content_length_with_plus_sign_is_malformed` / `test_content_length_with_minus_sign_is_malformed` を追加した
