# 死にコードと未使用の公開 API を削除する

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
- Branch: feature/refactor-remove-dead-code
- Polished: {YYYY-MM-DD}

## 目的

コードベースに残る死にコード・未使用の公開 API・write-only フィールドを整理する。

## 現状

以下はワークスペース全体で呼び出し 0 件または write-only であることを確認済み:

- `src/connection/mod.rs` の公開ゲッター 8 本 (`encoder_stream` / `encoder_stream_mut` / `decoder_stream` / `decoder_stream_mut` / `qpack_encoder` / `qpack_encoder_mut` / `qpack_decoder` / `qpack_decoder_mut`)。内部可変状態の露出でもある
- `src/connection/mod.rs` の `writable_streams` フィールド (`VecDeque<u64>`)。push のみで読み出し 0。メソッド `writable_streams()` は全ストリーム再計算
- `src/connection/mod.rs` の `peer_goaway_received` フィールド (write-only)
- `src/stream/control.rs` の `ControlStreamRecv::peer_settings` フィールド (write-only の二重管理)
- `src/event.rs` の `Event::ConnectionError` バリアント (生成箇所 0)
- `src/qpack/encoder.rs` の `DynamicEncoder::insert_with_dynamic_name_ref` / `unacked_section_count` / `peer_max_blocked_streams` (呼び出し 0)、`insert_with_static_name_ref` (テストのみ)
- `src/error.rs` の未使用バリアント: `ErrorCode::NoError` / `ExcessiveLoad` / `RequestCancelled` / `RequestIncomplete` (受信コード変換用の `from_code` 網羅のため残す判断も可、その場合は from_code テストを追加)、`Error::BufferTooShort` / `FrameDecodeError::UnknownFrameType` / `QpackError::StringTooLong` (定義のみ)
- `src/webtransport/error.rs` の `ErrorCode::RequirementsNotMet` (from_code / Display 内のみ)
- `src/qpack/encoder.rs` の Post-Base エンコード分岐 (到達不能な推測コード。テストまで存在)
- `src/webtransport/session/mod.rs` の `local_limits_mut` / `is_flow_control_enabled` / `clear_pending_capsules` (呼び出し 0)、`try_add_stream` / `queue_initial_flow_control_capsules` / `add_received_datagram` (テスト専用)
- `src/webtransport/stream.rs` の `classify_uni_stream` (unchecked 版) と stream_type ヘルパー (`StreamKind` と重複、pbt のみ)
- `src/webtransport/capsule.rs` の `encode_as_data_frame` / `validate_max_streams` / `validate_max_data` (セッション側と二重実装のまま未使用)
- `src/qpack/header.rs` の `Header::with_never_indexed` / `dynamic_table.rs` の `DynamicTable::clear` / `varint.rs` の `peek_len` (テスト・pbt のみ)
- `src/qpack/encoder_stream.rs` の文字列 encode/decode (`wire.rs` と 3 重実装のうちの 1 つ)
- `interop/wt/src/lib.rs` の varint / header encode ヘルパー (呼び出し 0、ライブラリと重複)
- `src/stream/mod.rs` の `UniStreamType::from_type` 系 (本番は生リテラル match で enum 未使用)

## 設計方針

- 呼び出し 0 件を grep で再確認してから削除する
- 公開 API として維持する意図があるものは残し、doc で明記する (例: `from_code` の網羅性のための ErrorCode バリアント)

## 完了条件

- 上記の死にコード・未使用 API が削除される (維持判断したものは doc 明記)
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/connection/mod.rs` / `src/stream/control.rs` / `src/event.rs`
- `src/qpack/` 配下 (encoder.rs / encoder_stream.rs / dynamic_table.rs / header.rs)
- `src/webtransport/` 配下 (session/mod.rs / stream.rs / capsule.rs / error.rs)
- `src/error.rs` / `src/varint.rs` / `src/stream/mod.rs`
- `interop/wt/src/lib.rs`
