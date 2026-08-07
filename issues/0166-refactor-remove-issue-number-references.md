# ソースコードに残る issue 番号参照を除去する

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
- Branch: feature/refactor-remove-issue-number-references
- Polished: {YYYY-MM-DD}

## 目的

shiguredo-issues 規約違反 (ソースコードへの issue 番号参照) を一斉に除去する。

## 現状

- `src/` 配下のコメント・docstring に issue 番号参照が 37 箇所以上残っている。代表例:
  - `src/connection/wt_session.rs` のモジュール doc「WebTransport セッション管理 (0077: connection/mod.rs から分離)」ほか 8 箇所
  - `src/connection/wt_capsule.rs` のモジュール doc「(0077: connection/mod.rs から分離)」
  - `src/connection/wt_stream.rs` のモジュール doc と関数 doc「(0077 Phase 5: 混在関数抽出)」
  - `src/connection/mod.rs` の「// 0077 Phase 5: ...」10 箇所、「// 0023: ...」「// 0048/0049: ...」
  - `src/qpack/encoder.rs`「(0117: DynamicEncoder に統合)」、`src/qpack/wire.rs`「(0117: Encoder/Decoder 重複解消)」
  - `src/webtransport/connect/mod.rs` ほか connect/ 配下 5 ファイルと session/ 配下 2 ファイル「0125: ...」
  - `src/stream/control.rs`「// 本 issue 時点では ...」、`src/webtransport/capsule.rs`「後続 issue で ...」
- 規約では issue 番号はコードに残さず、理由そのもの (仕様の節番号等) を書く

## 設計方針

- issue 番号参照を削除し、残すべき理由があれば理由そのもの (仕様節番号・設計意図) に書き換える

## 完了条件

- `src/` 配下に issue 番号参照が残らない
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/connection/mod.rs` / `wt_session.rs` / `wt_capsule.rs` / `wt_stream.rs` / `wt_types.rs`
- `src/qpack/encoder.rs` / `wire.rs`
- `src/webtransport/connect/` 配下と `session/` 配下
- `src/stream/control.rs` / `src/webtransport/capsule.rs`
