# WebTransport reset_stream_at の final size を Sans I/O 層で授受できない

Created: 2026-04-07
Completed: 2026-04-07
Model: Opus 4.6

## 概要

draft-ietf-webtrans-http3-15 Section 6 / Section 4.4 / Section 5.4 は、WebTransport ストリームのリセットに `RESET_STREAM_AT` (draft-ietf-quic-reliable-stream-reset) を用い、stream header (Quarter Stream ID prefix) を含む reliable size までは確実に配送することを要求する (stream header 長要件は Section 4.4)。これにより受信側は stream header を必ず読み切り、対応する WebTransport セッションを特定できる。

現状の `Connection` API はこの要件を Sans I/O 層で表現できていない。

1. 受信側: `Connection::stream_reset(stream_id, error_code)` は `error_code` のみを受け取り、QUIC 層から渡されるべき final size / reliable size を引数に持たない。WebTransport セッション側で「stream header の長さと final size の整合確認」「将来 `WT_MAX_DATA` のフロー制御に対する消費量集計」を行うための情報がない。
2. 送信側: 上位層が `RESET_STREAM_AT` を組み立てる際に必要な reliable size (= stream header byte 数) を Connection 層から取得できない。`Event::WebTransportSessionClosed` も `reset_stream_ids: Vec<u64>` と単一 `error_code` を返すだけで、各ストリームの reliable size を渡していない。
3. WebTransport データストリームの reset を通知する `Event::WebTransportStreamReset` も `error_code` のみで final size を持たない。

結果として、`reset_stream_at` transport parameter を双方が広告した draft-15 接続でも、Connection 層は仕様要件を上位層へ正しく中継できない。

## 該当箇所

- `src/connection/mod.rs` `Connection::stream_reset` (現在 L3550 付近)
- `src/connection/mod.rs` `Connection::terminate_wt_session_with` (現在 L1917 付近)
- `src/connection/mod.rs` `Connection::set_webtransport_transport_verified` (現在 L758 付近)
- `src/event.rs` `Event::WebTransportStreamReset` / `Event::WebTransportSessionClosed` (現在 L105 付近)

## 根拠

- draft-ietf-webtrans-http3-15 Section 3.1: `reset_stream_at` transport parameter を双方が送る MUST
- draft-ietf-webtrans-http3-15 Section 4.4: stream header byte は確実に配送されなければならず、reset 時の reliable size は stream header 長以上である必要がある
- draft-ietf-webtrans-http3-15 Section 6: セッション終了時の関連ストリームのリセットも同じ前提
- `refs/webtrans/draft-ietf-webtrans-http3-15.txt`

## 修正方針

破壊的変更を許容する。

1. `Connection::stream_reset` のシグネチャを `stream_reset(stream_id, error_code, final_size: u64)` へ変更する。`final_size` は QUIC 層から渡される final size (RFC 9000 Section 19.4)。
2. WebTransport データストリームの reset 経路で、`final_size` を `Event::WebTransportStreamReset` に追加して上位層へ伝達する。
3. `terminate_wt_session_with` を「セッションが保持している関連ストリームについて、stream header 長を計算し、`reliable_size = max(stream_header_len, 0)` を返す」形に拡張する。`Event::WebTransportSessionClosed` の `reset_stream_ids: Vec<u64>` を `reset_streams: Vec<WtStreamReset { stream_id, reliable_size }>` 等の構造体ベクタへ置き換える。
4. 送信側ヘルパとして、CONNECT stream および WT データストリームの「stream header 長」を計算する関数を Connection 層に置き、上位層が `RESET_STREAM_AT` の reliable size を決定できるようにする。Sans I/O の責務分離を維持するため、QUIC 層への送信自体は上位層が行う。
5. `wt_reset_stream_at_supported = false` の draft-02/07 経路では従来通り通常の `RESET_STREAM` にフォールバックする。`Event` の追加フィールドは draft 共通だが、reliable size が無意味な場合は 0 を入れる。
6. `CHANGES.md` に `[CHANGE]` として記載する。
7. テストで以下を追加する:
   - WT bidi/uni データストリームの reset で `final_size` がイベントに反映される
   - `WebTransportSessionClosed` で各関連ストリームの reliable size が stream header 長以上になる
   - draft-15 で `wt_reset_stream_at_supported = false` のときは CONNECT が拒否される (既存) ことを再確認する

## 補足

`WT_MAX_DATA` のフロー制御反映 (issue 0048 残課題) もこの API 拡張に乗せて実装するのが自然だが、本 issue のスコープは「Sans I/O 層が final size と reliable size を授受できる API にする」までとし、フロー制御カウンタの更新は別 issue で行ってよい。

## 解決方法

- `src/event.rs` に `WtStreamReset { stream_id, reliable_size }` 構造体を追加し、`Event::WebTransportSessionClosed` の `reset_stream_ids: Vec<u64>` を `reset_streams: Vec<WtStreamReset>` に置き換えた。`Event::WebTransportStreamReset` には `final_size: u64` フィールドを追加した。
- `src/connection/mod.rs` `Connection::stream_reset` のシグネチャを `(stream_id, error_code, final_size: u64)` に変更し、QUIC 層が運ぶ Final Size (RFC 9000 Section 19.4) を受け取れるようにした。WT データストリームの reset 経路ではこの値をそのまま `Event::WebTransportStreamReset::final_size` に伝達する。
- 上位層が `RESET_STREAM_AT` の reliable size を決定するためのヘルパとして `Connection::wt_stream_header_len(stream_id) -> u64` を追加した。`wt_bidi_streams` / `wt_uni_streams` から WT データストリームの session_id を引き、`varint(stream_type|signal) + varint(session_id)` を返す。
- `Connection::terminate_wt_session_with` を拡張し、関連 WT データストリームについて `wt_stream_header_len` を呼び出してから `wt_uni_streams` / `wt_bidi_streams` から除去するようにした。バッファリング段階で stream header 長を決定できないストリームは `reliable_size = 0` を返す (`reset_stream_at` 経路ではフォールバックを上位層に委ねる)。
- 単体テストを 3 件追加した:
  - `test_wt_session_closed_event_carries_reliable_sizes`: CONNECT stream RESET によるセッション終了時、関連 WT bidi/uni データストリームの `reliable_size` が stream header 長 (3 バイト) と一致することを検証
  - `test_wt_data_stream_reset_event_carries_final_size`: WT データストリーム単独 reset 時、QUIC から渡された `final_size` が `WebTransportStreamReset` イベントに反映されることを検証
  - `test_wt_stream_header_len_helper`: `wt_stream_header_len` がドメイン内では正しい値、非 WT ストリームでは 0 を返すことを検証
- 既存の `stream_reset` テスト 6 件を新シグネチャに合わせて更新した (final_size = 0 を渡す)。
- `CHANGES.md` の `## develop` に `[CHANGE]` として記載した。

## 残課題

- `RESET_STREAM_AT` (draft-ietf-quic-reliable-stream-reset) 自体の送信トリガと WT_MAX_DATA フロー制御消費量集計はスコープ外。Sans I/O 層は `final_size` / `reliable_size` を授受できる状態になっており、上位層が `RESET_STREAM_AT` フレームを組み立てる経路に `wt_stream_header_len` を渡す形で実装できる。
- バッファリング段階の WT ストリーム (CONNECT 確立前に到着し session に紐付く前のもの) は `reliable_size = 0` を返している。バッファリング段階で stream header 長を保持して伝搬する強化は、別 issue で扱うのが妥当。
