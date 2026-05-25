# 0080: 未使用の公開 API と死にコードを削除する

- Priority: Low
- Created: 2026-05-14
- Model: deepseek-v4-pro
- Branch: feature/refactor-remove-unused-and-dead-code

## 目的

未使用コード・死にコードを削除または非公開化し、API サーフェスを最小化する。

## 優先度根拠

Low: コンパイルやテストに問題はないが、不要な公開 API は将来の semver 互換性維持コストを増大させる。

## 現状

### 死にコード

1. `src/stream/request.rs:471-478` — `pub enum ReceivedData`: コードベースのどこからもインポート・使用されていない
2. `src/connection/mod.rs:354` — `#[allow(dead_code)]` on `disassociate_stream`: 実際に `mod.rs:3809` で呼び出されており dead code ではない。`#[allow(dead_code)]` を `#[expect(dead_code)]` に変更するか、不要なら削除

### 未使用の公開 API (テスト専用なら `pub(crate)` + `#[cfg(test)]` に変更)

3. `src/qpack/encoder_stream.rs:100-113` — `encode_insert_with_name_ref`
4. `src/qpack/encoder_stream.rs:122-135` — `encode_insert_with_literal_name`
5. `src/qpack/encoder_stream.rs:144-150` — `encode_duplicate`
6. `src/qpack/encoder_stream.rs:347-349` — `EncoderStreamReceiver::buffer()`
7. `src/qpack/decoder_stream.rs:78-80` — `encode_insert_count_increment`
8. `src/qpack/decoder_stream.rs:222-224` — `DecoderStreamReceiver::buffer()`

### 重複定数

9. `src/connection/mod.rs:74,77` / `src/webtransport/session.rs:12,15` — `MAX_BUFFERED_STREAMS` / `MAX_BUFFERED_DATAGRAMS` (両方 100): 一方に集約

## 設計方針

1. `ReceivedData` enum を削除
2. `#[allow(dead_code)]` を削除（dead code でないため不要）、または `#[expect(dead_code)]` に変更
3. テスト専用メソッドは `pub(crate)` に変更
4. `buffer()` メソッドが本当に不要か確認し、不要なら削除
5. 重複定数を `src/webtransport/session.rs` に集約し、`connection/mod.rs` 側は参照する

## 完了条件

- 未使用コードが削除または非公開化されていること
- 重複定数が一箇所に集約されていること
- `cargo test` が全て pass すること
- `cargo clippy` で未使用警告がないこと

## 影響範囲

- `src/stream/request.rs`
- `src/connection/mod.rs`
- `src/qpack/encoder_stream.rs`
- `src/qpack/decoder_stream.rs`
- `src/webtransport/session.rs`

## CHANGES.md エントリ案

```
### misc

- [UPDATE] 未使用の公開 API を非公開化し死にコードを削除する
  - @担当者
```
