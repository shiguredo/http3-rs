# StreamHeader を公開フィールドで直接構築すると encode がパニックする

- Created: 2026-08-15
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-stream-header-encode-panic
- Polished: {YYYY-MM-DD}

## 目的

公開 API 経由で値域違反の `StreamHeader` を構築して encode を呼ぶとパニックする問題を修正する。

## 現状

- `src/webtransport/stream.rs` の `StreamHeader` は `pub session_id: u64` の公開フィールドを持ち、`webtransport/mod.rs` で公開 re-export されている
- `StreamHeader::new` は値域 (2^62-1 以下・4 の倍数) を検証して `Result` を返す (パニックしない) が、公開フィールドがあるため構造体リテラル `StreamHeader { session_id: ... }` で `new` を迂回できる
- `encode_unidirectional` / `encode_bidirectional` は `session_id_to_varint(self.session_id).expect("session_id fits in VarInt")` で変換するため、2^62 以上の session_id で直接構築された `StreamHeader` を encode するとパニックする

## 設計方針

- `session_id` フィールドを private 化し、構築を検証済みの `StreamHeader::new` 経由のみに制限する。encode のシグネチャは変更しない (パニック経路を構造的に排除する)
- デコード関数 (`decode_unidirectional_checked` 等) はモジュール内から構造体リテラルで構築するため、private 化の影響は受けない

## 完了条件

- 値域外の session_id を持つ `StreamHeader` を構築できない (構造体リテラルで直接構築するとコンパイルエラーになる)
- テストが追加される
- CHANGES.md の `## develop` セクションに変更履歴が記載される
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/webtransport/stream.rs` (`StreamHeader.session_id` フィールド / `encode_unidirectional` / `encode_bidirectional`)
