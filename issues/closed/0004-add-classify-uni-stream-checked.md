# classify_uni_stream_checked() を追加する

Created: 2026-03-29
Completed: 2026-03-29
Model: Opus 4.6

## 概要

`classify_uni_stream()` の session_id 検証付きバージョンを追加する。

## 根拠

現状の `classify_uni_stream()` は session_id の正当性 (`session_id % 4 == 0`) を検証しない。ドキュメントに「呼び出し側で検証する必要がある」と記載されているが、moqt-rust-private の publisher / subscriber / relay の **全て** でこの検証を忘れている。

一方、同モジュールの `StreamHeader` は checked/unchecked の 2 系統を提供している:

- `StreamHeader::decode_unidirectional()` → `Option` (unchecked)
- `StreamHeader::decode_unidirectional_checked()` → `Result<_, StreamHeaderDecodeError>` (checked)

`classify_uni_stream()` が unchecked のみなのは API として一貫性がない。

draft-ietf-webtrans-http3-15 Section 4 では、不正な session_id を持つストリームは `H3_ID_ERROR` で接続を閉じる MUST。検証忘れはプロトコル違反に直結する。

## 対応方針

- `classify_uni_stream_checked()` を追加する
  - 戻り値: `Result<ClassifiedUniStream, StreamHeaderDecodeError>`
  - WebTransport ストリームの場合、`session_id % 4 != 0` なら `StreamHeaderDecodeError::InvalidSessionId` を返す
  - HTTP/3 ストリームの場合は既存と同じ動作
- 既存の `classify_uni_stream()` は互換性のために維持する
- `StreamHeader` の checked/unchecked 命名規則と統一する

## 解決方法

`classify_uni_stream_checked()` を `src/webtransport/stream.rs` に追加した。

- WebTransport ストリームの場合、`session_id % 4 != 0` なら `StreamHeaderDecodeError::InvalidSessionId` を返す
- バッファ不足は `StreamHeaderDecodeError::BufferTooShort` を返す
- HTTP/3 ストリームの場合は既存の `classify_uni_stream()` と同じ動作
- `StreamHeader` の checked/unchecked 命名規則と統一
