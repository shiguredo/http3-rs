# 死にコードと未使用の公開 API を削除する

- Created: 2026-08-08
- Completed: 2026-08-23
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

### 修正内容 (削除したもの)

- `src/connection/mod.rs`: 公開ゲッター 8 本 (`encoder_stream` / `encoder_stream_mut` / `decoder_stream` / `decoder_stream_mut` / `qpack_encoder` / `qpack_encoder_mut` / `qpack_decoder` / `qpack_decoder_mut`) を削除 (ワークスペース全体で呼び出し 0 件)
- `src/connection/mod.rs`: `writable_streams` フィールド (`VecDeque<u64>`) を削除。push 箇所 3 箇所も撤去。公開メソッド `writable_streams()` は全ストリーム再計算方式のため維持
- `src/connection/mod.rs`: `peer_goaway_received` フィールドを削除 (write-only。`peer_goaway_last_id` は GOAWAY 受信記録として引き続き使用)
- `src/stream/control.rs`: `ControlStreamRecv::peer_settings` フィールドを削除 (SETTINGS の内容は接続層の `Connection.peer_settings` で管理する二重管理だったため)
- `src/event.rs`: `Event::ConnectionError` バリアントを削除 (生成箇所 0 件)
- `src/qpack/encoder.rs`: `unacked_section_count` / `insert_with_static_name_ref` / `insert_with_dynamic_name_ref` を削除 (本番呼び出し 0 件。テストも削除)
- `src/qpack/encoder.rs`: Post-Base エンコード分岐 (Indexed Field Line / Literal with Name Reference の `absolute_index >= base` パス) を到達不能として `debug_assert!` に置き換え (base = required_insert_count のため `absolute_index < base` が必ず成立。RFC 9204 Section 3.2.6)。Post-Base テスト 3 件も削除
- `src/webtransport/session/mod.rs`: `local_limits_mut` / `is_flow_control_enabled` / `clear_pending_capsules` を削除 (呼び出し 0 件)
- `src/error.rs`: `Error::BufferTooShort` / `FrameDecodeError::UnknownFrameType` / `QpackError::StringTooLong` を削除 (定義のみ)
- `interop/wt/src/lib.rs`: `encode_varint` / `decode_varint` / `encode_wt_bidi_header` / `decode_wt_bidi_header` / `encode_wt_uni_header` / `decode_wt_uni_header` を削除 (呼び出し 0 件。ライブラリ API と重複)

### 維持判断したもの (issue の記述と現状が異なっていたため)

- `ErrorCode::NoError` / `ExcessiveLoad` / `RequestCancelled` / `RequestIncomplete`: `from_code` の受信コード変換網羅性のため維持。各 variant に doc を追記した (from_code テストは error.rs に存在)
- `webtransport::ErrorCode::RequirementsNotMet`: `from_code` / `Display` の網羅性のため維持し、doc を追記
- `src/webtransport/capsule.rs` の `encode_as_data_frame` / `validate_max_streams` / `validate_max_data`: pbt (`pbt/tests/prop_capsule`) で検証されている公開 API であり、`encode_as_data_frame` は 0158 (tokio-s2n-quic の WtSession::close のカプセル DAŅA 包み) で利用予定のため維持
- `src/qpack/header.rs` の `Header::with_never_indexed` / `src/qpack/dynamic_table.rs` の `DynamicTable::clear` / `src/varint.rs` の `peek_len`: 公開 API であり、pbt (prop_varint) が存在。issue 記述の「テスト・pbt のみ」は pbt が主検証手段である前提での推奨表現であり、公開 API として残した方が一貫性があると判断
- `src/webtransport/stream.rs` の `classify_uni_stream` (unchecked 版): `crates/tokio-s2n-quic/src/webtransport/{client,server}.rs` が使用しており、呼び出し 0 の記述は現状と異なるため維持
- `src/stream/mod.rs` の `UniStreamType`: `lib.rs` から再エクスポートしている公開 API のため維持
- `try_add_stream` / `queue_initial_flow_control_capsules` / `add_received_datagram`: 本番経路 (queue_initial_flow_control_capsules) またはテストで使用されており維持
- `src/qpack/encoder_stream.rs` の文字列 encode/decode: `encode_string` 等は encoder_stream.rs 内の本番処理で使用中であり、wire.rs と二重でも独立した可読な実装として維持 (3 重実装は事実ではなく 2 層)
