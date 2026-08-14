# WT_STREAM (0x41) をリクエストストリームの先頭以外で受信しても H3_FRAME_ERROR にならない

- Created: 2026-08-08
- Completed: 2026-08-14
- Branch: feature/fix-wt-stream-frame-h3-frame-error
- Polished: 2026-08-08

## 目的

draft-ietf-webtrans-http3-16 Section 4.3 の MUST 違反 (WT_STREAM を誤った場所で受信しても接続エラーにならない) を修正する。

## 現状

- 0x41 の判定は `src/connection/mod.rs` の `Connection::dispatch_client_bidi_stream` (サーバー側のみ) と `src/connection/wt_stream.rs` の `resolve_wt_bidi_stream_header` の「ストリーム先頭バイト位置」でのみ行われる (先頭の判定自体は `Role::Server` かつ `is_wt_fully_negotiated()` 時のみ)
- `src/frame/mod.rs` の `FrameType::from_type` に 0x41 がなく、`src/frame/decoder.rs` のフレームデコードは 0x41 を `Frame::Unknown` に落とす
- `src/stream/request.rs` の `RequestStream::process_raw` は `Frame::Unknown` を無視するため、リクエストストリームの 2 フレーム目以降に 0x41 が現れても検出されず静かにスキップされる。制御ストリームも同様に無視される (`ControlStreamRecv::process` の Ready 状態の catch-all `_ => {}` と `process_control_stream` の `_ => {}` で黙殺)
- クライアント側はレスポンス受信方向のストリームで先頭に 0x41 を受信しても、サーバー側の `dispatch_client_bidi_stream` が機能しないため (`Role::Server` 限定)、`RequestStream::process_raw` で無視される
- 根拠: draft-16 Section 4.3「Endpoints MUST NOT send WT_STREAM as a frame type on HTTP/3 streams other than the very first bytes of a request stream. Receiving this frame type in any other circumstances MUST be treated as a connection error of type H3_FRAME_ERROR」

## 設計方針

- **0x41 の検出はフレームデコード層ではなく、ストリーム処理層で行う**。`RequestStream::process_raw` と `ControlStreamRecv::process` の `Frame::Unknown` 分岐 (または catch-all の置き換え) で `frame_type == 0x41` を検査し、`Error::ConnectionError(ErrorCode::FrameError)` に変換する。フレームデコード層 (`decode_frame_header` / `decode_frame` / `UnknownFrame::new`) は変更しない (0x41 は draft 上「not a proper HTTP/3 frame」であり、`Frame` enum に variant を追加するのは誤解釈を招く。`UnknownFrame::new` を変更すると `decode_frame` の None arm の `expect` が panic する)
- **リクエストストリームの先頭位置の 0x41 はサーバー側のみエラーにしない**。サーバー側は WT ネゴシエーション済みなら `dispatch_client_bidi_stream` が先頭 varint を捕捉して WT 経路に回し、未ネゴシエーション時は「very first bytes of a request stream」に該当し MUST NOT の対象外として RFC 9114 Section 9 (未知フレームは無視) に従い無視する (0143 の 0x54 側の方針と整合。接続を殺さない)。**クライアント側はレスポンス受信方向のストリームで先頭に 0x41 を受信した場合も H3_FRAME_ERROR にする** (draft-16 の「very first bytes of a request stream」例外はストリーム開始側 (リクエスト = クライアント送信方向) にのみ該当し、クライアントが受信するレスポンスの先頭は「any other circumstances」に該当するため)
  - 「先頭位置」と「2 フレーム目以降」の区別には、`RequestStream` に先頭フレーム処理済みフラグを追加する (フラグは `Frame::Unknown` のスキップを含む最初のフレーム消費時に立てる。`recv_state == WaitingHeaders` では 1xx 後の復帰や予約フレーム (0x21) のスキップと区別できないため)。サーバー側のみ先頭位置を無視し、クライアント側は先頭フレーム処理済みフラグに関わらず 0x41 をエラーにする。ロールの区別は `RequestStream` にロールを保持させる (コンストラクタ引数追加) ことで実現する
- **制御ストリームでは SETTINGS 受信後 (Ready 状態) の 0x41 を H3_FRAME_ERROR にする**。SETTINGS 受信前 (WaitingSettings 状態) の 0x41 は既存の `MissingSettings` エラーを維持する (RFC 9114 Section 6.2.1 の SETTINGS 先頭必須が優先)。Ready 状態の catch-all 置き換え時は、GOAWAY / MAX_PUSH_ID 等の既存フレームのパススルーを維持する
- **送出側のブロックは 0169 で対応する**。`frame::encode_frame` は公開 API で `Frame::Unknown` をエンコードでき、`UnknownFrame::new(VarInt::from_static(0x41), ...)` も成功するため、WT_STREAM をフレームとしてエンコードできる状態にある。この送出側の MUST NOT 違反は 0169 (WT_STREAM をフレームとしてエンコードできる状態をブロックする) で対応する。本 issue の受信側の検出では `UnknownFrame::new` を変更しない (decode 経路の expect が panic するため)
- 単方向ストリームの先頭 0x41 は「ストリームタイプ」でありフレームタイプではないため MUST NOT の対象外。サーバー開始 bidi ストリームの先頭 0x41 は `resolve_wt_bidi_stream_header` が検証済みで、本 issue の対象外

## 完了条件

- リクエストストリームの 2 フレーム目以降に 0x41 を受信したとき (サーバー側・クライアント側とも) `Error::ConnectionError(ErrorCode::FrameError)` が返る
- クライアント側のレスポンス受信方向のストリームで先頭に 0x41 を受信したとき `Error::ConnectionError(ErrorCode::FrameError)` が返る
- 制御ストリームの SETTINGS 受信後に 0x41 を受信したとき `Error::ConnectionError(ErrorCode::FrameError)` が返る
- サーバー側のリクエストストリーム先頭位置の 0x41 は既存挙動を維持する (WT ネゴシエーション済み: WT ストリームとして処理 / 未ネゴシエーション: 無視)
- テストが追加される: `src/stream/request.rs` の `#[cfg(test)]` モジュールで `process_raw` の 2 フレーム目以降 0x41 拒否とサーバー側先頭位置 0x41 の無視維持を検証し、`src/stream/control.rs` の `#[cfg(test)]` モジュールで制御ストリームの 0x41 拒否を検証する (リクエストストリームの 1 フレーム目は予約フレーム (0x21) を使い、`process_raw` は QPACK デコードを行わないため QPACK エンコードは不要。制御ストリームは SETTINGS を最初に置く)。既存の PBT が壊れないことを確認する
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/stream/request.rs` (`RequestStream::process_raw` の `Frame::Unknown` 分岐 / 先頭フレーム処理済みフラグ / ロール別の先頭位置扱い)
- `src/stream/control.rs` (`ControlStreamRecv::process` の Ready 状態の catch-all 置き換え)
- 一次資料: `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 4.3、`refs/h3/rfc9114.txt` Section 9 (未知フレームの無視)、Section 6.2.1 (制御ストリームの SETTINGS 先頭必須)

### 修正内容

- `RequestStream::new` のシグネチャを `new(stream_id, role)` に変更し、接続ロールを保持するようにした (`Role` は `src/connection/mod.rs` の公開 enum)
- `RequestStream` に先頭フレーム処理済みフラグ `first_frame_processed` を追加し、最初のフレーム消費時に立てるようにした。`Frame::Unknown` のスキップを含む最初のフレーム消費時に立てることで、先頭位置と 2 フレーム目以降を区別する
- `RequestStream::process_raw` の `Frame::Unknown` 分岐で `frame_type == 0x41` を検査し、`Error::ConnectionError(ErrorCode::FrameError)` を返すようにした。サーバー側の先頭位置のみ「very first bytes of a request stream」に該当するため無視を維持し、クライアント側は先頭位置でもエラーにする
- `ControlStreamRecv::process` の Ready 状態の catch-all を置き換え、0x41 を `Error::ConnectionError(ErrorCode::FrameError)` にするようにした。WaitingSettings 状態の 0x41 は既存の `MissingSettings` エラーを維持する (RFC 9114 Section 6.2.1 の SETTINGS 先頭必須が優先)
- `src/connection/mod.rs` の `RequestStream::new` 呼び出し 2 箇所に `self.role` を渡すようにした
- `fuzz/fuzz_targets/fuzz_stream.rs` にサーバーロールの variant を追加し、先頭 0x41 無視パスを fuzz 対象にした

### 追加したテスト

- `src/stream/request.rs` の `#[cfg(test)]` モジュール:
  - サーバー側先頭位置 0x41 の無視維持 (後続 HEADERS の正常処理を含む)
  - サーバー側 2 フレーム目以降 0x41 の `FrameError` 拒否
  - クライアント側先頭位置 0x41 の `FrameError` 拒否
  - クライアント側 2 フレーム目以降 0x41 の `FrameError` 拒否
  - ペイロード長非ゼロの 0x41 無視
  - チャンク分割で到着した先頭 0x41 の無視
- `src/stream/control.rs` の `#[cfg(test)]` モジュール:
  - SETTINGS 受信後 (Ready 状態) の 0x41 の `FrameError` 拒否
  - SETTINGS 受信前 (WaitingSettings 状態) の 0x41 の `MissingSettings` 維持
