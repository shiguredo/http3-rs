# WT_STREAM (0x41) をフレームとしてエンコードできる状態をブロックする

- Created: 2026-08-08
- Completed: 2026-08-23
- Branch: feature/fix-wt-stream-frame-encode-block
- Polished: {YYYY-MM-DD}

## 目的

draft-ietf-webtrans-http3-16 Section 4.3 の MUST「Endpoints MUST NOT send WT_STREAM as a frame type」の送出側違反 (WT_STREAM をフレームとしてエンコードできる状態) を修正する。

## 現状

- `src/frame/encoder.rs` の `encode_frame` / `encode_unknown_frame` は公開 API で、`Frame::Unknown` をワイヤバイト列にエンコードできる
- `src/frame/mod.rs` の `UnknownFrame::new(VarInt::from_static(0x41), ...)` は成功する (0x41 は既知タイプでも HTTP/2 専用タイプでもないため)
- つまり、ライブラリ利用者は `shiguredo_http3::frame::encode_frame` 経由で WT_STREAM (0x41) をフレームとしてエンコード・送信できてしまう
- 根拠: draft-16 Section 4.3「Endpoints MUST NOT send WT_STREAM as a frame type on HTTP/3 streams other than the very first bytes of a request stream」

## 設計方針

- **エンコード側で 0x41 をブロックする**。`encode_unknown_frame` (または `encode_frame` の `Frame::Unknown` 分岐) で `frame_type == 0x41` を検査し、`None` を返す
- **`UnknownFrame::new` ではブロックしない**。`decode_frame` の None arm (decoder.rs) が `UnknownFrame::new(...).expect(...)` で構築しており、`new` で 0x41 を拒否すると受信した 0x41 が panic になるため (受信側の 0x41 検出は別 issue 0142 で対応する)
- WT_STREAM は draft 上「not a proper HTTP/3 frame」であり、フレームとしての送信は常に不正。エンコードをブロックしても正当な送信経路を壊さない

## 完了条件

- `encode_frame` に `Frame::Unknown` (frame_type = 0x41) を渡すと `None` が返る
- 0x41 以外の `Frame::Unknown` のエンコードは従来どおり成功する
- テストが追加される: `src/frame/encoder.rs` の `#[cfg(test)]` モジュールで 0x41 のエンコード拒否と 0x41 以外の Unknown フレームのエンコード成功を検証する
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/frame/encoder.rs` (`encode_frame` / `encode_unknown_frame`)
- 一次資料: `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 4.3

### 修正内容

- `src/frame/encoder.rs` の `encode_unknown_frame` で `frame_type == 0x41` を検査し `None` を返すように修正した (draft-ietf-webtrans-http3-16 Section 4.3 の MUST「Endpoints MUST NOT send WT_STREAM as a frame type」)
- `src/frame/encoder.rs` の `#[cfg(test)]` モジュールに `test_encode_unknown_frame_wt_stream_is_rejected` (0x41 の拒否) と `test_encode_unknown_frame_other_type_is_ok` (0x42 は従来どおり成功) を追加した
- `UnknownFrame::new` はブロックしない (受信側の 0x41 デコード経路を壊さない方針を維持)
