# StreamHeader デコード経路で session_id の 2^62 上限検査が欠落している

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-wt-stream-header-session-id-limit
- Polished: {YYYY-MM-DD}

## 目的

draft-ietf-webtrans-http3-16 Section 4 の MUST (存在しないストリーム ID に対応する session ID の受信 → H3_ID_ERROR) をデコード経路で満たす。

## 現状

- `src/webtransport/stream.rs` の `StreamHeader::decode_unidirectional_checked` / `decode_bidirectional_checked` は `session_id.is_multiple_of(4)` のみを検査し、2^62-1 上限 (RFC 9000 Section 2.1 のストリーム ID 空間) を検査しない
- `StreamHeaderDecodeError::SessionIdOutOfRange` は `StreamHeader::new` でのみ発生し、デコード経路では未使用
- `varint::decode` は 8 バイト形式で 2^62-1 超の値 (例: 2^62) をそのまま返すため、session_id = 2^62 が受理される
- 実際の受信経路 (`src/connection/wt_stream.rs` の `resolve_wt_uni_stream_session_id` / `resolve_wt_bidi_stream_header`) も `session_id % 4` のみ
- 影響: サーバーが `wt_sessions[2^62]` に Pending セッションを作成し、対応する CONNECT ストリームが存在しないため永久に Pending として残留する (上限 16 を消費)
- 根拠: draft-16 Section 4「If an endpoint receives a session ID ... that does not correspond to a client-initiated bidirectional stream ID, the endpoint MUST close the connection with an H3_ID_ERROR error code」

## 設計方針

- デコード経路で session_id の 2^62-1 上限検査を追加する
- 受信経路 (connection 層) で違反を `ErrorCode::IdError` にマップする

## 完了条件

- 2^62 以上の session_id を含む WT ストリームヘッダー / データグラムを受信すると H3_ID_ERROR になる
- テストが追加される
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/webtransport/stream.rs` (`StreamHeader::decode_unidirectional_checked` / `decode_bidirectional_checked` / `classify_uni_stream_checked`)
- `src/connection/wt_stream.rs` (`resolve_wt_uni_stream_session_id` / `resolve_wt_bidi_stream_header`)
- 一次資料: `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 4、`refs/quic/rfc9000.txt` Section 2.1
