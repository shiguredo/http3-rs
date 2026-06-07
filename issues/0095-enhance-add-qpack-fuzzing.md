# QPACK の fuzzing カバレッジを拡充する

- Priority: Medium
- Created: 2026-06-07
- Model: DeepSeek v4-pro
- Branch: feature/add-qpack-fuzzing
- Polished: 2026-06-07

## 目的

QPACK の fuzzing カバレッジが一部不足しているため、以下の対象を追加する:

1. `DynamicEncoder` (`src/qpack/encoder.rs`) の fuzz — encode 側の核。既存の `fuzz_qpack` は Decoder 側だけをカバーしている。
2. QPACK 整数エンコード `encode_integer` / `encode_integer_to_vec` (`src/qpack/integer.rs`, RFC 9204 Section 4.1.1 / RFC 7541 Section 5.1) の fuzz — `prefix_bits >= 64` 時の `1u64 << prefix_bits`（`integer.rs:15`）によるデバッグビルドでのパニックリスクがある。
3. QPACK 整数デコード `decode_integer` (`src/qpack/integer.rs`) の fuzz — 他 fuzz で間接通るが、任意バイト列 + 任意 prefix_bits の直接検証がない。`prefix_bits >= 16` 時の `1u16 << prefix_bits`（`integer.rs:76`）によるデバッグビルドでのパニックリスクもある。

## 優先度根拠

fuzzing は任意バイト列に対するパニック安全性の検証を目的としており、特にエンコード側は整数演算でパニックのリスクがある。`encode_integer`: `prefix_bits >= 64` で `1u64 << prefix_bits`、`decode_integer`: `prefix_bits >= 16` で `1u16 << prefix_bits` がデバッグビルドでパニックする。これらの shift overflow を含め、任意入力でパニックせずエラーを返すことを保証する必要がある。Medium とする。

## 現状

既存の `fuzz_qpack` は Decoder（静的/動的）、EncoderStreamReceiver、DecoderStreamReceiver の 4 経路をカバーしているが、以下の 2 つが不足:

1. **`DynamicEncoder` (`src/qpack/encoder.rs`)**: Required Insert Count（RFC 9204 Section 4.5.1.1）、Base（RFC 9204 Section 4.5.1.2）、Post-Base Indexing（RFC 9204 Section 3.2.6）等の計算を含むエンコード側の核。`encode_required_insert_count`（`encoder.rs:536`）には `max_entries == 0` かつ `req_insert_count != 0` 時の `debug_assert!` も存在する。Decoder 側は fuzz 済みだが Encoder 側は未カバー。ヘッダーの正しさは PBT (`pbt/tests/prop_qpack/main.rs` の `prop_dynamic_encoder_decoder_roundtrip`) で検証済みだが、任意入力に対するパニック安全性は fuzz で別途検証する。
2. **QPACK 整数エンコーディング (`src/qpack/integer.rs`)**: `encode_integer` / `encode_integer_to_vec` / `decode_integer` は、他の fuzz で間接通るが、任意バイト列 + 任意 prefix_bits の直接 fuzz がない。特に shift overflow のパニックリスクがある。

## 設計方針

- fuzz の責務は「任意入力でパニックしないこと」の検証に限定する。RFC 9204 Section 7.4 では、decode できない値はパニックではなくエラーとして扱うことを MUST としている。正しさの検証は PBT で行う。
- 既存の `fuzz_qpack.rs` の `FuzzInput` enum に variant を追加する形で対応する。
- `DynamicEncoder` には mutable な状態を構築し、任意のヘッダーリストをエンコードさせる。
- 整数エンコーディングは任意バイト列 + 任意引数を各関数に通す。
- `prefix_bits` が RFC の範囲外（`>8`）の値も含め、仕様外の入力でパニックしないことを fuzz で検証する。

## 完了条件

- `fuzz_qpack` に `DynamicEncoder` の fuzz 経路が追加されている
- `fuzz_qpack` に `decode_integer` の fuzz 経路が追加されている
- `fuzz_qpack` に `encode_integer` / `encode_integer_to_vec` の fuzz 経路が追加されている
- 整数関数の shift overflow パニックを解消するコード修正も併せて行う（`prefix_bits >= 16` または `>= 64` のガード追加。RFC 7541 Section 5.1 より prefix_bits は 1..=8 が正規の範囲であり、範囲外は即座にエラーを返してよい）
- `cargo fuzz run fuzz_qpack -- -max_total_time=5` がクラッシュなく完了すること
- PBT (`prop_qpack`) と既存の全テストが通ること

## 解決方法

### 変更 1: `DynamicEncoder` variant

`FuzzInput` に以下の variant を追加:

```rust
DynamicEncoder {
    headers: Vec<(Vec<u8>, Vec<u8>)>,
    blocked_streams_count: u8,
    use_huffman: bool,
}
```

fuzz ターゲット内の処理手順:

1. `let mut encoder = DynamicEncoder::new().use_huffman(use_huffman);`
2. `encoder.set_max_table_capacity(4096);`
3. `encoder.set_table_capacity(4096);`
4. `encoder.set_peer_max_blocked_streams(100);`
5. `let _ = encoder.insert(b"a".to_vec(), b"b".to_vec());` — 戻り値が `None` でも動的テーブルが空のまま encode が静的テーブル path を通るだけなので問題ない
6. 各 `(name, value)` を `Header::new(name, value)` で変換し、失敗したらスキップ（不正バイト列は validation で弾かれるが、それは期待動作）
7. 有効な Header を収集し、エンコード用バッファを確保する。バッファサイズは `headers.len() * 32 + 128` の固定計算で十分なマージンを取る。`estimate_encoded_size` は静的テーブルのみを考慮するため使用しない
8. `encoder.encode(&mut buf, &headers, blocked_streams_count as usize)` を呼ぶ
9. 戻り値が `None` の場合はバッファ不足だが、`None` を返すことは期待動作でありパニックではない

### 変更 2: `decode_integer` variant

```rust
IntegerDecode {
    data: Vec<u8>,
    prefix_bits: u8,
}
```

fuzz ターゲット内の処理:

1. `decode_integer(&data, prefix_bits)` を呼ぶ
2. 戻り値（`Ok` / `Err(QpackError::BufferTooShort | QpackError::DecodeFailed)`）を問わず、パニックしなければ成功

### 変更 3: `encode_integer` / `encode_integer_to_vec` variant

```rust
IntegerEncode {
    value: u64,
    prefix_bits: u8,
    prefix: u8,
}
```

fuzz ターゲット内の処理:

1. バッファを `let mut buf = [0u8; 32];` で確保（RFC 7541 Section 5.1 の疑似コードからの導出で `ceil((value + 1).log2() / 7) + 1` が最大長だが、`value = u64::MAX` で最大 10 バイト。32 バイトで十分）
2. `encode_integer(&mut buf, value, prefix_bits, prefix)` を呼ぶ — `prefix_bits >= 64` の shift overflow は実装側のガード追加で解消済みの想定
3. `let mut vec = Vec::new();`
4. `encode_integer_to_vec(&mut vec, value, prefix_bits, prefix)` を呼ぶ
5. 戻り値を問わず、パニックしなければ成功

### 完了後の注意点

- fuzz はテスト基盤の改善であり、機能に直接影響しないため `CHANGES.md` の `### misc` に `[ADD]` として追記する。
- `FuzzInput` に variant が追加されるため、既存の fuzz corpus は新しい enum representation と一致せず破棄される。corpus は fuzz 実行により再構築される。
