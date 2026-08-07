# WT_STREAM (0x41) をリクエストストリームの先頭以外で受信しても H3_FRAME_ERROR にならない

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-wt-stream-frame-h3-frame-error
- Polished: {YYYY-MM-DD}

## 目的

draft-ietf-webtrans-http3-16 Section 4.3 の MUST 違反 (WT_STREAM を誤った場所で受信しても接続エラーにならない) を修正する。

## 現状

- 0x41 の判定は `src/connection/mod.rs` の `Connection::dispatch_client_bidi_stream` / `resolve_wt_bidi_stream_header` の「ストリーム先頭バイト位置」でのみ行われる
- `src/frame/mod.rs` の `FrameType::from_type` に 0x41 がなく、`src/frame/decoder.rs` のフレームデコードは 0x41 を `Frame::Unknown` に落とす
- `src/stream/request.rs` の `RequestStream::process_raw` は `Frame::Unknown` を無視するため、リクエストストリームの 2 フレーム目以降に 0x41 が現れても検出されず静かにスキップされる
- 根拠: draft-16 Section 4.3「Endpoints MUST NOT send WT_STREAM as a frame type on HTTP/3 streams other than the very first bytes of a request stream. Receiving this frame type in any other circumstances MUST be treated as a connection error of type H3_FRAME_ERROR」

## 設計方針

- フレームタイプ 0x41 を `FrameDecodeError` に追加し、リクエストストリーム内で受信したら `H3_FRAME_ERROR` 接続エラーに変換する
- WT がネゴシエーションされていない接続の先頭で 0x41 を受信した場合の扱いも仕様に合わせて確認する

## 完了条件

- リクエストストリームの 2 フレーム目以降に 0x41 を受信したとき `Error::ConnectionError(ErrorCode::FrameError)` が返る
- テストが追加される
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/frame/mod.rs` (`FrameType::from_type`)
- `src/frame/decoder.rs` (フレームタイプのデコード)
- `src/stream/request.rs` (`RequestStream::process_raw`)
- 一次資料: `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 4.3
