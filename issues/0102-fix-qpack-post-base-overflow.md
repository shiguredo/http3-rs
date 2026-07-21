# QPACK Post-Base 参照デコードの算術オーバーフローを修正する

- Priority: High
- Created: 2026-06-15
- Model: Opus 4.7
- Branch: feature/fix-qpack-post-base-overflow
- Polished: 2026-07-21

## 目的

`src/qpack/decoder.rs` の Post-Base Indexed Field Line / Literal with Post-Base Name Reference デコードで `base + post_base_index` の加算がオーバーフロー保護を欠いており、`DynamicTable::get_by_post_base_index` の `base + post_base_index` も同様の問題を持つ。debug ビルドで panic、release ビルドで wrap-around が発生し、攻撃者が大きな post-base index を仕込んだ field section でクラッシュやテーブル誤参照を引き起こせる。`checked_add` で安全化する。

## 優先度根拠

High。攻撃者が任意の wire を送り込める前提では、ピアからの入力で debug ビルドのテストランナーが panic する経路は信頼性低下に直結する。release ビルドでも wrap で `absolute_index` が小さい値になり、本来到達してはいけない動的テーブルエントリを参照する可能性がある。

## 現状

`src/qpack/decoder.rs:493-518` `decode_indexed_field_post_base`:

```rust
let absolute_index = base + post_base_index;
```

`src/qpack/decoder.rs:570-601` `decode_literal_with_post_base_name_ref`:

```rust
let absolute_index = base + post_base_index;
```

`src/qpack/dynamic_table.rs:262` `get_by_post_base_index`:

```rust
let absolute_index = base + post_base_index;
```

`base` は `required_insert_count + delta_base` の結果で最大約 `2^63`、`post_base_index` は RFC 9204 Section 4.1.1 で 62 bit までデコード可能。両者の加算は容易に u64 範囲を超える。

`src/qpack/decoder.rs:358` の `decode_required_insert_count` 内 `base = required_insert_count + delta_base` も同様の懸念がある。

## 設計方針

- 各加算を `checked_add` に置き換え、`None` の場合は `QpackError::DecodeFailed` を返す
- 加算結果が `dynamic_table.insert_count()` を超えるかも併せて検査する (現状の `get_by_absolute_index` 内でチェック済みだが、明示的に弾く方が読みやすい)
- PBT で「Post-Base 参照デコードが任意入力で panic / wrap しない」プロパティを追加 (fuzz と棲み分け)
- fuzz_target も `decode_indexed_field_post_base` 経路を通る入力で検証

## 完了条件

- 上記 3 箇所のすべての `base + post_base_index` が `checked_add` 化される
- `src/qpack/decoder.rs:358` の `required_insert_count + delta_base` も `checked_add` 化される
- 任意入力で panic しないことを fuzz と PBT で検証
- 既存テスト・PBT・fuzz が全てパスする
- `make fmt && make clippy && make check` が全て通る

## 解決方法

```rust
let absolute_index = base
    .checked_add(post_base_index)
    .ok_or(QpackError::DecodeFailed)?;
```

`DynamicTable::get_by_post_base_index` も同様にシグネチャを `Result<Option<&Entry>, ...>` または内部チェックで対処する。呼び出し側の影響範囲を狭めるため、内部チェックで `None` を返す形が最小変更。

### 関連ファイル

- 修正対象:
  - `src/qpack/decoder.rs:358, 493-518, 570-601`
  - `src/qpack/dynamic_table.rs:262`
- PBT 追加: `pbt/tests/prop_qpack/main.rs`
- 一次資料: `refs/h3/rfc9204.txt` Section 3.2.6, 4.5.3, 4.5.5
