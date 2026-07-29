# `DynamicTable::insert_with_name_ref` の relative / absolute インデックス取り違えを修正する

- Priority: High
- Created: 2026-06-15
- Completed: 2026-07-30
- Model: Opus 4.7
- Branch: feature/fix-dynamic-table-insert-with-name-ref-index
- Polished: 2026-07-21

## 目的

`src/qpack/dynamic_table.rs:194-209` の `DynamicTable::insert_with_name_ref` が RFC 9204 Section 4.3.2 「When T=0, the number is the **relative index** of the entry in the dynamic table.」を absolute index として解釈する実装ミスを抱えている。本 API は現状エンコーダーストリーム受信経路 (`encoder_stream.rs`) から呼ばれず死にコードに近いが、`pub` 公開されているため外部 / 将来の経路で誤った wire 解釈につながる。relative index を正しく扱うか、または死にコードとして削除するかを判断する。

## 優先度根拠

High。RFC 9204 Section 4.3.2 の仕様違反になり得る公開 API。コードレビュー結果でも「致命的 (相互運用性に直結する公開 API のバグ)」と判定された。`encoder_stream.rs:266-303` 側の `decode_insert_with_name_ref` は内部で正しい解釈を行っているため、本 API は本質的に未使用。死にコードを残すか正しく動かすかの判断が必要。

## 現状

`src/qpack/dynamic_table.rs:194-209`:

```rust
pub fn insert_with_name_ref(
    &mut self,
    name_index: u64,
    is_static: bool,
    value: Vec<u8>,
    static_table: &[crate::qpack::Header],
) -> Option<u64> {
    let entry = if is_static {
        static_table.get(name_index as usize)?
    } else {
        self.get_by_absolute_index(name_index)?
    };
    // ...
}
```

RFC 9204 Section 4.3.2 / `refs/h3/rfc9204.txt`:

> When T=0, the number is the relative index of the entry in the dynamic table.

`encoder_stream.rs:266-303` の `decode_insert_with_name_ref` は `get_by_relative_index_encoder(name_index)` を呼んでおり、こちらは正しい。`dynamic_table.rs:194-209` の本 API は内部呼び出しが無く (`grep` で 0 件)、テスト経由でも実質呼ばれていない。

## 設計方針

2 通りの選択肢から実装側で判断する:

1. **削除**: 本 API は内部・外部とも未使用かつ encoder_stream.rs 側で同等処理が完結している。`pub fn insert_with_name_ref` を削除し、公開境界から外す。`Header::insert_with_name_ref` を期待する利用者がいない以上、これが最小コストで安全
2. **修正**: `get_by_absolute_index` を `get_by_relative_index_encoder` に変更し、関数名や引数名から「relative index を受け取る」ことを明示する (`relative_name_index` 等)。利用者には RFC 9204 Section 4.3.2 の引用コメントを付ける

判断材料:
- 利用箇所が無い (`grep` 結果ゼロ)
- 同等の機能が `encoder_stream.rs` で実装済み
- 公開 API 削除は破壊的変更だが `#[non_exhaustive]` 系で守られていない構造体メソッド

推奨は **削除**。`CHANGES.md` に `[CHANGE] qpack::DynamicTable::insert_with_name_ref を削除する` を追加する。

ただし、外部利用者が依存している可能性が完全に否定できないため、最終判断はメンテナが行う。

## 完了条件

- 選択肢 1 (削除) の場合:
  - `pub fn insert_with_name_ref` が削除される
  - `CHANGES.md` に `[CHANGE]` エントリが追加される
  - `cargo test --tests -p shiguredo_http3 -p pbt` が全てパスする
- 選択肢 2 (修正) の場合:
  - `name_index` が relative として解釈され、`get_by_relative_index_encoder` を呼ぶ
  - RFC 9204 Section 4.3.2 の引用コメントが付く
  - relative index 解釈を PBT で検証
- `make fmt && make clippy && make check` が通る

## 解決方法

選択肢 1 (削除) の場合:

```rust
// 削除:
// pub fn insert_with_name_ref(&mut self, name_index: u64, is_static: bool, value: Vec<u8>, static_table: &[Header]) -> Option<u64>;
```

`CHANGES.md ## develop` に追加:

```markdown
- [CHANGE] `qpack::DynamicTable::insert_with_name_ref` を削除する (未使用の公開 API)
  - @voluntas
```

選択肢 2 (修正) の場合:

```rust
pub fn insert_with_name_ref(
    &mut self,
    relative_name_index: u64,
    is_static: bool,
    value: Vec<u8>,
    static_table: &[crate::qpack::Header],
) -> Option<u64> {
    let entry = if is_static {
        static_table.get(relative_name_index as usize)?
    } else {
        // RFC 9204 Section 4.3.2: T=0 のとき relative index
        self.get_by_relative_index_encoder(relative_name_index)?
    };
    // ...
}
```

### 関連ファイル

- 修正対象: `src/qpack/dynamic_table.rs:194-209`
- 参照: `src/qpack/encoder_stream.rs:266-303` (正しい実装の例)
- 一次資料: `refs/h3/rfc9204.txt` Section 4.3.2

## 解決方法

コミット f5b5260 で実装した。DynamicTable::insert_with_name_ref の relative / absolute インデックス取り違えを修正し、RFC 9204 Section 4.3.2 に準拠する形にした。
