# QPACK Encoder の Indexed Field Line / Literal with Name Reference の prefix 境界判定を修正する

- Priority: High
- Created: 2026-06-15
- Model: Opus 4.7
- Branch: feature/fix-qpack-encoder-prefix-boundary
- Polished: 2026-07-21

## 目的

`shiguredo_http3::qpack::Encoder` (静的テーブル専用) が、RFC 7541 Section 5.1 / RFC 9204 Section 4.5.2 で規定された prefix integer エンコードの境界条件を誤って実装しており、特定の静的テーブルインデックスに対して相互運用性を壊す wire 表現を生成する。デコーダー側は continuation byte を期待して次のヘッダーバイトを誤読するため、HTTP/3 レスポンスやリクエストのフレームが破壊される。本 issue でこの境界判定バグを修正する。

## 優先度根拠

High。相互運用性を直接破壊する致命的バグで、以下の現実的なケースで発生する:

- `Encoder::encode_indexed_field` の境界バグ: 静的テーブル index = 63 (`:status 100`) を含むレスポンスをエンコードすると wire が破壊される。`:status 100` は Continue 応答や Early Hints 等の運用で発生し得る
- `Encoder::encode_literal_with_name_ref` の境界バグ: 静的テーブル index = 15 (`:method CONNECT` の name 部分) を name reference として使う `Header::new(b":method", b"PATCH")` 等で発生する

いずれもピアの QPACK デコーダーは continuation byte を読みに行き、後続のヘッダーバイトを誤読するため、当該ストリームが詰まるか接続クローズに至る。`DynamicEncoder::encode_indexed_field_static` / `DynamicEncoder::encode_literal_with_name_ref_static` は本バグを含まないため、本バグの影響範囲は「`Encoder` (静的専用) 経由でエンコードした場合」に限定されるが、それ自体が公開 API として存在する以上修正は必須。

## 現状

`src/qpack/encoder.rs:88-100` の `Encoder::encode_indexed_field` は次の通り実装されている。

```rust
fn encode_indexed_field(&self, buf: &mut [u8], index: usize) -> Option<usize> {
    if index < 64 {
        // 6-bit prefix で収まる
        if buf.is_empty() {
            return None;
        }
        buf[0] = 0xc0 | (index as u8);
        Some(1)
    } else {
        // 6-bit prefix を超える場合
        integer::encode_integer(buf, index as u64, 6, 0xc0)
    }
}
```

RFC 7541 Section 5.1 / RFC 9204 Section 4.5.2 に基づくと、6-bit prefix integer の `max_prefix = 2^6 - 1 = 63`。`integer::encode_integer` (`src/qpack/integer.rs:22`) は `if value < max_prefix` (= `value < 63`) のときに 1 バイトで完結させる。一方この実装は `index < 64` で 1 バイトに収めているため、index = 63 のときに `buf[0] = 0xff` を 1 バイトだけ書き出して終わる。

ピア側のデコーダー (`src/qpack/integer.rs:94`) は `prefix_value < mask` の判定で `0x3f < 0x3f = false` となり、continuation byte を読みに行く。結果として後続のヘッダーバイト (次の field line representation の先頭) を誤って integer の続きとして消費し、ストリーム全体のデコードが破綻する。

同じ性質のバグが `src/qpack/encoder.rs:111-127` の `Encoder::encode_literal_with_name_ref` にも存在する。

```rust
fn encode_literal_with_name_ref(
    &self,
    buf: &mut [u8],
    index: usize,
    value: &[u8],
) -> Option<usize> {
    // Name Reference: 0101NNNN (T=1 for static, N=0)
    let mut offset = if index < 16 {
        if buf.is_empty() {
            return None;
        }
        buf[0] = 0x50 | (index as u8);
        1
    } else {
        integer::encode_integer(buf, index as u64, 4, 0x50)?
    };
    // ...
};
```

4-bit prefix の `max_prefix = 15`。index = 15 (静的テーブル `:method CONNECT` の name 部分) のときに `buf[0] = 0x5f` を 1 バイトだけ書き、デコーダーは continuation byte を期待する。

参考として、`DynamicEncoder` 側の `encode_indexed_field_static` (`src/qpack/encoder.rs:597-603`) と `encode_literal_with_name_ref_static` は `integer::encode_integer` への一段委譲のみで実装されており、境界条件を `integer::encode_integer` 側に任せている。こちらは正しい実装になっている。

PBT 側でこのバグが検出できなかった理由は、`pbt/src/lib.rs` の `valid_header_name` strategy が小文字英字 (`a-z`) のみを生成しており、静的テーブルにある疑似ヘッダー (`:status`, `:method` 等) との exact match が発生しないため。

## 設計方針

`Encoder::encode_indexed_field` と `Encoder::encode_literal_with_name_ref` の手書きの境界分岐を撤廃し、prefix 整数のエンコードを `integer::encode_integer` に一本化する。`DynamicEncoder::encode_indexed_field_static` / `encode_literal_with_name_ref_static` と同じ実装形式に揃える。

リグレッション再発防止のため、PBT に「静的テーブルの exact name match / exact (name, value) match を引き起こす入力」を生成する strategy を追加し、境界 index (15, 63 等) のラウンドトリップを検証する。

## 完了条件

- `Encoder::encode_indexed_field` が index = 63 を含む任意の静的テーブルインデックスについて、`integer::encode_integer` と一致する wire を生成する
- `Encoder::encode_literal_with_name_ref` が index = 15 を含む任意の静的テーブルインデックスについて、`integer::encode_integer` と一致する wire を生成する
- 上記を検証する PBT が `pbt/tests/prop_qpack/` 配下に追加され、`cargo test` で成功する
- 既存の PBT / 単体テスト / fuzz が全てパスする
- nghttp3 との相互運用テスト (`interop/h3`) で `:status 100` / `:method PATCH` のラウンドトリップが成功する
- `make fmt && make clippy && make check` が全て通る

## 解決方法

### コード修正

`src/qpack/encoder.rs:88-100` の `Encoder::encode_indexed_field` を次の通り簡略化する。

```rust
fn encode_indexed_field(&self, buf: &mut [u8], index: usize) -> Option<usize> {
    integer::encode_integer(buf, index as u64, 6, 0xc0)
}
```

`src/qpack/encoder.rs:111-127` の `Encoder::encode_literal_with_name_ref` も同様に `integer::encode_integer` 一段委譲に揃える。

```rust
fn encode_literal_with_name_ref(
    &self,
    buf: &mut [u8],
    index: usize,
    value: &[u8],
) -> Option<usize> {
    let mut offset = integer::encode_integer(buf, index as u64, 4, 0x50)?;
    let value_len = self.encode_string(&mut buf[offset..], value)?;
    offset += value_len;
    Some(offset)
}
```

### PBT 追加

`pbt/src/lib.rs` の strategy に、静的テーブルの exact name match / exact (name, value) match を引き起こすケースを追加する (例: `:status 100`, `:method PATCH` 相当のヘッダーを混ぜる strategy)。

`pbt/tests/prop_qpack/main.rs` に以下を追加する:

- `Encoder` で静的テーブル exact match を含むヘッダーをエンコードし、`Decoder` でデコードしてラウンドトリップ等価性を検証
- 特に index = 63, 15 を含むエッジケースを通る入力を生成
- `Encoder` と `DynamicEncoder::encode_static_only` の wire 出力が一致することを検証 (cross-validation)

### 仕様引用コメント

`integer::encode_integer` への委譲箇所に、RFC 7541 Section 5.1 / RFC 9204 Section 4.5.2 (Indexed Field Line) / Section 4.5.4 (Literal Field Line with Name Reference) の節番号を引用するコメントを追加する。AGENTS.md「資料を由来の機能を実装する場合は、根拠資料名、節番号、将来変更される可能性があることをコードコメントで明記すること」に従う。

### 関連ファイル

- 修正対象: `src/qpack/encoder.rs:88-100`, `src/qpack/encoder.rs:111-127`
- 参照実装: `src/qpack/encoder.rs:597-603` (`DynamicEncoder::encode_indexed_field_static`)
- 参照実装: `src/qpack/integer.rs:15-51` (`integer::encode_integer`)
- PBT 追加: `pbt/src/lib.rs`, `pbt/tests/prop_qpack/main.rs`
- 一次資料: `refs/rfc7541.txt` Section 5.1, `refs/h3/rfc9204.txt` Section 4.5.2 / 4.5.4
