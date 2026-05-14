# 0062: encode_frame の戻り値が各所で破棄されている — サイレントデータ破損

Created: 2026-05-14
Model: deepseek-v4-pro

## 概要

`frame::encode_frame` の `Option<usize>` 戻り値が以下の 4 箇所で破棄されている。
バッファサイズ計算 (`encoded_frame_len`) と実際のエンコード処理で齟齬が生じた場合、
ゼロ初期化されたバッファが上位層に渡され、サイレントデータ破損が発生する。

## 対象箇所

- `src/stream/control.rs:76`: SETTINGS フレームのエンコード
- `src/stream/control.rs:95`: GOAWAY フレームのエンコード
- `src/stream/request.rs:172`: HEADERS フレームのエンコード
- `src/stream/request.rs:221`: DATA フレームのエンコード

## 修正方針

各箇所で `frame::encode_frame(...)` の戻り値をチェックする。
すべて `encoded_frame_len` で確保済みのバッファに書き込むため、
実用上は `None` を返すことはないが、防御的プログラミングとして
`expect("frame encode buffer correctly sized")` を追加する。

```rust
// 修正前
frame::encode_frame(&mut buf, &frame);

// 修正後
frame::encode_frame(&mut buf, &frame)
    .expect("frame encode buffer correctly sized");
```

## 影響範囲

- `src/stream/control.rs:76,95`
- `src/stream/request.rs:172,221`

## 備考

`encoded_frame_len` で確保したバッファに書き込むため実用上 `None` にはならないが、不変条件が破綻した場合のサイレントデータ破損を防ぐため `.expect()` で早期発見する。リリースビルドでのオーバーヘッドは許容範囲。
