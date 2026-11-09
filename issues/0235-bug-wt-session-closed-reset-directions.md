# SessionClosed.reset_streams の対象と reliable_size を送信方向に合わせる

- Created: 2026-11-10
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-wt-session-closed-reset-directions
- Polished: {YYYY-MM-DD}
- Category: bug

## 目的

セッション終了時の `WebTransportEvent::SessionClosed.reset_streams` が、ローカルに送信方向がないストリームを含み、
送信方向に WT ヘッダーを送っていないストリームへヘッダー長の reliable_size を付けている。
RESET_STREAM_AT と STOP_SENDING の対象を送信方向・受信方向に合わせ、
doc どおり実装しても接続エラーにならない API にする。

## 現状

- `Connection::terminate_wt_session_with` は `WtSession::associated_streams` の全ストリームとバッファ済みストリームを
  `reset_streams` に入れ、`Connection::wt_stream_header_len` の戻り値を reliable_size にしている
- `associated_streams` には `Connection::associate_or_buffer_stream` 経由でピア開始 uni / bidi も登録される
- `Connection::wt_stream_header_len` はストリームが `wt_uni_streams` / `wt_bidi_streams` に登録されていれば、
  開始側がどちらかに関係なく提示ヘッダー長を返す
- draft-17 Section 4.3 は WT ヘッダーを双方向ストリームの開始側だけが送ると定める。
  ローカル開始双方向ストリームの応答方向と、ピア開始 uni のローカル方向には WT ヘッダーが存在しない
- `WebTransportEvent::SessionClosed` の doc は `reset_streams` の全ストリームへ
  `RESET_STREAM_AT` と STOP_SENDING を送るよう指示している
- この組み合わせにより、doc どおり実装すると次の接続エラーが起こり得る
  - ピア開始双方向ストリーム: 応答方向の reliable_size がヘッダー長 (3 バイト) になる。
    応答を 3 バイト未満しか送っていない場合、Reliable Size > Final Size となり、
    ピアが FRAME_ENCODING_ERROR で接続を閉じる (draft-ietf-quic-reliable-stream-reset Section 4)
  - ピア開始 uni ストリーム: ローカルに送信方向がない。RESET_STREAM_AT を送ると、
    ピアは send-only ストリームへの RESET_STREAM として STREAM_STATE_ERROR で接続を閉じる (RFC 9000 Section 19.4)
  - ローカル開始 uni ストリーム: ローカルに受信方向がない。STOP_SENDING を送ると、
    ピアは receive-only ストリームへの STOP_SENDING として STREAM_STATE_ERROR で接続を閉じる (RFC 9000 Section 19.5)
- 既存テスト `test_wt_session_closed_event_carries_reliable_sizes` は、
  ピア開始双方向ストリーム (stream 4) とピア開始 uni ストリーム (stream 2) に reliable_size = 3 を期待しており、
  現状の挙動を固定している
- `crates/tokio-ngtcp2` と `crates/tokio-s2n-quic` は `reset_streams` をまだ消費していない
  (`tokio-ngtcp2` は `close_error_code` / `close_message` のみ参照し、`tokio-s2n-quic` は空の Vec を返す)。
  現時点では顕在化していないが、Sans I/O API の利用者が doc どおり実装すると発生する

## 再現手順

1. サーバーでセッション 0 を確立する
2. クライアント開始双方向ストリーム 4 に `0x41 0x00` (WT ヘッダーのみ) を流し、サーバーが応答を送らない状態にする
3. CONNECT ストリームを reset してセッションを終了する
4. `SessionClosed.reset_streams` に stream 4 が reliable_size = 3 で含まれる。
   これを `RESET_STREAM_AT` で送ると Final Size 0 < Reliable Size 3 となり、仕様上ピアは FRAME_ENCODING_ERROR で閉じる
5. クライアント開始 uni ストリーム 2 も同様に `reset_streams` に含まれ、
   `RESET_STREAM_AT` の送信がピアで STREAM_STATE_ERROR になる

## 修正方針

- `reset_streams` を「ローカルが送信方向を持つストリーム」に限定する。
  対象はローカル開始双方向 / ローカル開始 uni / ピア開始双方向とし、ピア開始 uni は除外する
- reliable_size は、リセットする送信方向に実際に WT ヘッダーを送った場合だけヘッダー長を加える。
  ローカル開始双方向 / uni はヘッダー長、ピア開始双方向の応答方向は 0 とする
  (アプリケーション指定オフセットは未対応のため現状は 0)
- STOP_SENDING の対象は受信方向を持つストリーム (ローカル開始双方向 / ピア開始双方向 / ピア開始 uni) であり、
  RESET の対象と一致しない。`SessionClosed` の doc を方向別に書き換え、
  必要なら送信対象と受信対象を区別するフィールドを追加する。公開 API の変更は破壊的変更として許容する
- バッファ済みストリームは WT ヘッダーを送っていないため reliable_size は 0 のままとし、
  ストリーム種別で RESET 対象と STOP_SENDING 対象を分ける
- テストの期待値を更新し、ピア開始 uni の除外、ピア開始双方向の reliable_size 0、
  ローカル開始ストリームのヘッダー長維持、方向別の送信対象を検証する

## 完了条件

- `reset_streams` にピア開始 uni ストリームが含まれない
- ピア開始双方向ストリームの reliable_size が 0 になる (アプリ指定オフセット導入時はオフセットのみ)
- ローカル開始双方向 / uni ストリームの reliable_size がヘッダー長のまま
- `SessionClosed` の doc が方向別の送信対象を明確にし、doc どおり実装して
  STREAM_STATE_ERROR / FRAME_ENCODING_ERROR にならない
- 既存テストの期待値を更新し、上記を検証するテストがある
- `cargo fmt --all -- --check` と
  `cargo clippy --workspace --exclude interop_h3 --exclude interop_wt --all-targets -- -D warnings` と
  `cargo test --workspace --exclude interop_h3 --exclude interop_wt` が通る

## 関連ファイル

- `src/connection/wt_session.rs`: `Connection::terminate_wt_session_with`,
  `Connection::associate_or_buffer_stream`
- `src/connection/wt_stream.rs`: `Connection::wt_stream_header_len`,
  `Connection::resolve_wt_uni_stream_session_id`, `Connection::resolve_wt_bidi_stream_header`
- `src/event.rs`: `WebTransportEvent::SessionClosed`, `WtStreamReset`
- `src/connection/wt_types.rs`: `WtSession::associated_streams`
- `src/connection/mod.rs`: `test_wt_session_closed_event_carries_reliable_sizes`,
  `test_wt_session_closed_event_carries_reliable_size_for_local_uni`
- `crates/tokio-ngtcp2/src/webtransport.rs` / `crates/tokio-s2n-quic/src/webtransport/session.rs`:
  `reset_streams` の消費状況

## 一次資料

- `refs/webtrans/draft-ietf-webtrans-http3-17.txt` Section 4.3 (L754-807): WT ヘッダーは開始側だけが送る
- 同 Section 6 (L1530-1545): セッション終了時に送信側をリセットし受信側の読み取りを中断する
- `refs/quic/draft-ietf-quic-reliable-stream-reset-11.txt` Section 4 (L229-240):
  Reliable Size が Final Size より大きい場合は FRAME_ENCODING_ERROR
- `refs/quic/rfc9000.txt` Section 19.4 (L6072-6073): send-only ストリームへの RESET_STREAM は STREAM_STATE_ERROR
- `refs/quic/rfc9000.txt` Section 19.5 (L6108-6111): receive-only ストリームへの STOP_SENDING は STREAM_STATE_ERROR
- `refs/quic/rfc9000.txt` Section 2.1: ストリーム ID の下位 2 ビットと方向
