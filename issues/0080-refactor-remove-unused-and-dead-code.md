# 0080: 未使用の公開 API と死にコードを削除する

Created: 2026-05-14
Model: deepseek-v4-pro

## 概要

以下の未使用コード・死にコードを削除または非公開化する。

### 死にコード (削除)

1. `src/stream/request.rs:471-478` `pub enum ReceivedData`
   - コードベースのどこからもインポート・使用されていない

2. `src/connection/mod.rs:354` `#[allow(dead_code)]` on `disassociate_stream`
   - 実際に `mod.rs:3809` で呼び出されており dead code ではない。アノテーションを削除

### 未使用の公開 API (pub を外す)

3. `src/qpack/encoder_stream.rs:100-113` `encode_insert_with_name_ref` — テストのみ使用
4. `src/qpack/encoder_stream.rs:122-135` `encode_insert_with_literal_name` — テストのみ使用
5. `src/qpack/encoder_stream.rs:144-150` `encode_duplicate` — テストのみ使用
6. `src/qpack/encoder_stream.rs:347-349` `EncoderStreamReceiver::buffer()` — 呼び出しなし
7. `src/qpack/decoder_stream.rs:78-80` `encode_insert_count_increment` — テストのみ使用
8. `src/qpack/decoder_stream.rs:222-224` `DecoderStreamReceiver::buffer()` — 呼び出しなし
9. `src/webtransport/settings.rs:46-51` `SettingsId::is_webtransport()` — 本番未使用
10. `src/webtransport/capsule.rs:426-447` `validate_max_streams` / `validate_max_data` — 本番未使用

### 重複定数 (片方に集約)

11. `src/connection/mod.rs:74,77` / `src/webtransport/session.rs:12,15`
    `MAX_BUFFERED_STREAMS` / `MAX_BUFFERED_DATAGRAMS` (両方 100)

## 影響範囲

- 複数ファイル
