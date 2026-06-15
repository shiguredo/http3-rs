# `EncoderStreamReceiver` の挿入失敗時に無限ループする問題を修正する

- Priority: Medium
- Created: 2026-06-15
- Completed:
- Model: Opus 4.7
- Branch: feature/fix-encoder-stream-receiver-infinite-loop
- Polished:

## 目的

`src/qpack/encoder_stream.rs:292-296, 318-322` で `decode_insert_with_*` が `table.insert(...).ok_or(QpackError::DecodeFailed)?` で early return する際、`recv_buffer.drain(...)` を呼ばないため、同じバイト列で何度呼び出しても同じ失敗を返す無限ループ状態になる。RFC 9204 Section 2.2.3 は「動的テーブル容量を超えるエントリの挿入は接続エラー」と規定するため、専用エラー型を導入して上位層に接続クローズを促す。

## 優先度根拠

Medium。実装上は呼び出し元 (`Connection`) が接続エラーで閉じれば顕在化しないが、エラー型が `QpackError::DecodeFailed` で一般的すぎ、上位で接続クローズすべきか判断しづらい。

## 現状

`src/qpack/encoder_stream.rs:292-296`:

```rust
let inserted = table.insert(name.clone(), value.clone()).ok_or(QpackError::DecodeFailed)?;
self.recv_buffer.drain(..consumed);
```

`?` で early return すると drain が呼ばれず、次回呼び出しでも同じ位置から同じデータを読み直す。

RFC 9204 Section 2.2.3:

> If the decoder encounters a reference in an encoder instruction to a dynamic table entry that has already been evicted, it MUST treat this as a connection error of type QPACK_ENCODER_STREAM_ERROR.

## 設計方針

- `QpackError` に `EncoderStreamError` / `DecoderStreamError` / `DecompressionFailed` の 3 種を導入し RFC 9204 Section 6 のエラーコードと対応させる
- `decode_insert_with_*` が `insert` 失敗時に `EncoderStreamError` を返す
- 上位層 (`Connection`) で `EncoderStreamError` を H3_QPACK_ENCODER_STREAM_ERROR で接続クローズに変換
- `recv_buffer.drain(..consumed)` は失敗時にも呼ばない方針を維持する (どうせ接続クローズするため)。ただしコメントで「これは致命的・接続クローズ必須」と明記
- 専用 fuzz / PBT で「容量超過時に必ず EncoderStreamError を返す」プロパティを検証

## 完了条件

- `QpackError` に新 variant が追加される
- `decode_insert_with_*` 系の failure path が EncoderStreamError を返す
- `Connection` が H3_QPACK_ENCODER_STREAM_ERROR でクローズする
- PBT / fuzz でカバー
- `make fmt && make clippy && make check` が通る

## 解決方法

```rust
#[non_exhaustive]
pub enum QpackError {
    BufferTooShort,
    DecodeFailed,
    EncoderStreamError,
    DecoderStreamError,
    DecompressionFailed,
    // ...
}
```

呼び出し箇所を該当 variant に置き換える。

### 関連ファイル

- 修正対象: `src/qpack/encoder_stream.rs:292-296, 318-322`, `src/error.rs::QpackError`
- 上位対応: `src/connection/mod.rs`
- 一次資料: `refs/h3/rfc9204.txt` Section 2.2.3, Section 6
