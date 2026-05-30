# 0092: HTTP/3 の STOP_SENDING critical stream 判定漏れと GOAWAY デコードエラー分類を修正する

- Priority: High
- Created: 2026-05-30
- Model: Codex 5.3
- Branch: feature/fix-http3-rfc-compliance-critical-stream-handling
- Polished: 2026-05-30

## 目的

`src/connection/mod.rs` の `stop_sending` と `src/frame/decoder.rs` の `decode_goaway_frame` に、RFC 9114 / RFC 9204 の MUST に対する取りこぼしがある。前者は送信側 critical stream への `STOP_SENDING` を `H3_CLOSED_CRITICAL_STREAM` にできず、後者は GOAWAY の payload 欠落を `H3_FRAME_ERROR` に集約できない。接続エラー種別の誤判定を解消し、peer 実装との相互運用性を担保するため、仕様どおりに修正する。

## 優先度根拠

High。項目 1 (`STOP_SENDING` の critical stream 判定漏れ) は接続管理の根幹に直結する RFC 9114 Section 6.2.1 / RFC 9204 Section 4.2 の MUST 違反であり、送信側 critical stream への `STOP_SENDING` を見逃すと接続を維持し続けてしまう。項目 2 (GOAWAY デコード) は RFC 9114 Section 7.1 の MUST 違反だが、影響はエラーコード分類のみで軽微。両者を併せて High とするが、優先して解消すべきは項目 1。

## 現状

### 項目 1: 送信側 critical stream への `STOP_SENDING` 判定漏れ

`stop_sending` (`src/connection/mod.rs:3891-3898`) の critical 判定が受信側ストリーム (`control_recv` / `peer_encoder_stream_id` / `peer_decoder_stream_id`) を見ている。これは `stream_reset` (`src/connection/mod.rs:3830-3832`) からそのままコピーされたもの。

- `STOP_SENDING` は「こちらが送信するストリームの送信停止を peer が要求する」フレームであり、対象は送信側 critical stream (`control_send` / ローカル QPACK encoder・decoder stream) でなければならない。受信側ストリームはこちらが送信していないため、`STOP_SENDING` が正当に届くことはない。
- 対になる `stream_reset` (RESET_STREAM 受信) は、peer が送信するストリームの中断であるため受信側 critical stream を見るのが正しい。`STOP_SENDING` と `RESET_STREAM` で判定すべき方向 (送信側 / 受信側) が逆になる点が要諦。
- RFC 9114 Section 6.2.1 (`refs/h3/rfc9114.txt`): "the receiver MUST NOT request that the sender close the control stream. If either control stream is closed at any point, this MUST be treated as a connection error of type H3_CLOSED_CRITICAL_STREAM."
- RFC 9204 Section 4.2 (`refs/h3/rfc9204.txt`): "the receiver MUST NOT request that the sender close either of these streams. Closure ... MUST be treated as a connection error of type H3_CLOSED_CRITICAL_STREAM."

`STOP_SENDING` はこの「receiver requesting the sender to close」に該当するため、送信側 critical stream を判定対象にしなければならない。

### 項目 2: GOAWAY payload 欠落が `H3_FRAME_ERROR` に集約されない

`decode_goaway_frame` (`src/frame/decoder.rs:176-183`) は payload が空 (`payload_len == 0`) のとき `varint::decode` が失敗し `FrameDecodeError::BufferTooShort` を返す。

- `decode_frame` (`src/frame/decoder.rs:112-119`) は呼び出し前に `total_len` 分のバイトが揃っていることを保証する。したがって `decode_goaway_frame` 内での `BufferTooShort` は「データ不足」ではなく「宣言長どおりだが payload が欠落している」ことを意味し、本来 `H3_FRAME_ERROR` にすべきケース。
- `src/stream/control.rs:236-254` の `decode_frame` エラーマッピングには `BufferTooShort` のアームがなく、`other => Error::FrameDecode(other)` (`src/stream/control.rs:253`) に落ちる。`Error::FrameDecode` は `Error::ConnectionError(ErrorCode::FrameError)` とは別 variant であり、空 payload GOAWAY が `H3_FRAME_ERROR` に集約されない。GOAWAY は control stream 専用フレームのため、本問題は control stream 経路でのみ発生する (request stream で GOAWAY を受信した場合はフレーム配置エラー `H3_FRAME_UNEXPECTED` の対象であり、別扱い)。
- `decode_settings_frame` (`src/frame/decoder.rs:161,165`) と `decode_max_push_id_frame` (`src/frame/decoder.rs:187`) は同じ状況で `InvalidLength` を返しており、GOAWAY だけが `BufferTooShort` を返している。この非対称が原因。
- RFC 9114 Section 7.1 (`refs/h3/rfc9114.txt`): "A frame payload that contains additional bytes after the identified fields or a frame payload that terminates before the end of the identified fields MUST be treated as a connection error of type H3_FRAME_ERROR."

## 設計方針

- 仕様の MUST を最優先し、互換性より堅牢性を優先する。
- エラー分類を RFC 9114 / RFC 9204 の規定 (`H3_CLOSED_CRITICAL_STREAM`, `H3_FRAME_ERROR`) に厳密に合わせる。
- 両項目とも API シグネチャは不変で、動作変更のみ。

## 完了条件

- 送信側 control stream / ローカル QPACK encoder・decoder stream への `STOP_SENDING` 受信で `H3_CLOSED_CRITICAL_STREAM` を返す。
- control stream 上の GOAWAY の payload 欠落・余剰バイトを `H3_FRAME_ERROR` にマップできる。
- 追加したテストが修正前は失敗し、修正後に `cargo test --workspace --tests` で通過する。

## 解決方法

1. `stop_sending` (`src/connection/mod.rs:3891`) の critical 判定対象を、送信側ストリーム `control_send.stream_id()` / `encoder_stream_id` / `decoder_stream_id` に置き換える。受信側ストリームの判定は `STOP_SENDING` では誤りなので除去する。
2. `decode_goaway_frame` (`src/frame/decoder.rs:177`) の `.map_err(|_| FrameDecodeError::BufferTooShort)` を `InvalidLength` に変更し、`decode_max_push_id_frame` と揃える。`src/stream/control.rs:250` の既存 `InvalidLength => FrameError` 経路で `H3_FRAME_ERROR` に集約されるため、`control.rs` 側のマッピング変更は不要。
3. 関連するコードコメントの誤った RFC 引用を修正する。`src/connection/mod.rs` の `1383` / `3817` / `3890` / `4081` 行が QPACK critical stream の根拠を "RFC 9204 Section 4.3" と誤記しているが、正しくは "RFC 9204 Section 4.2" (Encoder and Decoder Streams)。Section 4.3 は Encoder Instructions で無関係。
4. テストを追加する (いずれも意図的なエラーパスのため単体テスト):
   - `tests/test_connection.rs`: 送信側 control / encoder / decoder stream への `STOP_SENDING` 受信で `H3_CLOSED_CRITICAL_STREAM` になること。
   - `tests/test_connection.rs`: 空 payload GOAWAY を control stream に投入し、`H3_FRAME_ERROR` の接続エラーになること (end-to-end)。
   - `src/frame/decoder.rs` の既存 `#[cfg(test)]` テスト: 空 payload GOAWAY のデコードが `FrameDecodeError::InvalidLength` を返すこと。

## 関連

- 項目 1 が触れる `stop_sending` は `src/connection/mod.rs` にあり、connection モジュール分割を扱う 0077 (Priority: Low, 未着手) と同じファイルを変更する。0092 が High かつ先行するため実害はないが、0077 着手時は 0092 の変更を取り込んだ上で進めること。
- Host ヘッダーの構文検証 (元の項目 3) は RFC の MUST ではなく堅牢性向上のため、別 issue (0093) に分離した。非 http/https スキームの authority 制約 (元の項目 4) は受信側で原理的に判定できず、既存実装 (`src/validation.rs:484-488`, `562-566`) が呼び出し側責務として意図的に設計済みのため、本 issue の対象から除外した。
