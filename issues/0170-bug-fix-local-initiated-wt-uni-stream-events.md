# ローカル開始の WT uni ストリームでピアの STOP_SENDING が通知されない

- Created: 2026-08-08
- Completed: 2026-08-27
- Branch: feature/fix-local-initiated-wt-uni-stream-events
- Polished: 2026-08-26

## 目的

アプリが自分で開いた WT uni ストリームに対するピアの STOP_SENDING が WebTransport イベントとして通知されない問題を修正する。

## 現状

- `src/connection/wt_session.rs` の `Connection::handle_wt_stop_sending` は `wt_uni_streams` / `wt_bidi_streams` に登録されたストリームのみを処理し、未登録のストリーム (ローカル開始ストリーム) は `false` を返して汎用 `Event::StopSending` にフォールスルーする (セッション ID が付かない)
- ローカル開始の WT uni ストリームを登録する API は 0144 (ローカル開始の WT ストリーム受信データがリクエストストリームとして誤処理される) で bidi のみを対象としており、uni はスコープ外とされた
- ローカル開始 uni ストリームはローカルにとって送信専用であり受信データは存在しない (RFC 9000 Section 2.1)。ピア (受信側) から送信できるのは STOP_SENDING のみ (RFC 9000 Section 3.5) である。RESET_STREAM / STREAM (FIN) は送信側 (initiator) の操作であり、send-only ストリームへの RESET_STREAM / STREAM は STREAM_STATE_ERROR の接続エラーになる (RFC 9000 Section 19.4 / 19.8) ため、WT イベントとして通知する対象にはならない
- draft-16 Section 4.4 は WT データストリームへの STOP_SENDING をアプリに伝播することを求める

## 設計方針

- ローカル開始 WT uni ストリームの登録 API を追加する (0144 の bidi 登録 API (`register_local_wt_stream`) と同様の形)。現状の `register_local_wt_stream` は uni ストリーム ID (下位 2 ビット 0x02 / 0x03) を `IdError` で拒否しているため、ローカル開始 uni (クライアント側は `ClientUni` / サーバー側は `ServerUni`。RFC 9000 Section 2.1 Table 1) を受け入れるよう拡張する
- 登録 API はセッションの `associated_streams` にも追加する (セッション終了時の後始末に含める。0144 の bidi と同様)
- `handle_wt_stop_sending` が登録済みのローカル開始 uni ストリームを処理し、`WebTransportEvent::StreamStopSending` (セッション ID 付き) を通知する
- ピアからの RESET_STREAM / FIN はローカル開始 uni ストリームでは到着しない (到着時は QUIC 層が STREAM_STATE_ERROR で接続エラーにする) ため、本 issue の対象外とする
- STOP_SENDING はストリーム数クレジット (WT_MAX_STREAMS) に関与しない (`handle_wt_stop_sending` は `on_remote_stream_closed` を呼ばない) ため、クレジット返却の考慮は不要

## 完了条件

- ローカル開始 WT uni ストリームに対するピアの STOP_SENDING が `WebTransportEvent::StreamStopSending` (セッション ID 付き) として通知される
- 登録 API の検証が機能する: 非ローカル開始 ID・二重登録・セッション不存在・非 Established セッションがエラーになる
- テストが追加される (`src/connection/mod.rs` の `#[cfg(test)]` モジュールで、クライアント側 / サーバー側それぞれの「登録 → STOP_SENDING → イベント検証」と、登録 API の検証エラー)
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 変更内容

- `src/connection/wt_stream.rs`:
  - `is_local_initiated_uni(kind)` メソッドを追加した (`is_local_initiated_bidi` と対称的、`Role::Client` → `ClientUni` / `Role::Server` → `ServerUni` を判定)
  - `register_local_wt_stream` を bidi / uni の両方を受理するように拡張した。二重登録チェックは `wt_bidi_streams` / `wt_uni_streams` の両方を対象化した。uni の場合は `wt_uni_streams` に登録する
  - doc コメントを bidi / uni 両対応に書き換え、SessionClosed.reset_streams への波及と、ローカル uni への STREAM / FIN / RESET_STREAM 受信は QUIC 層で STREAM_STATE_ERROR となり sans-I/O に到達しない前提を明記した (RFC 9000 Section 19.4 / 19.8)
- `src/connection/{client,server}.rs`:
  - 公開 API `register_local_wt_stream` の doc コメントを bidi / uni 両対応に更新した
- `src/connection/mod.rs`:
  - 既存 `test_register_local_wt_stream_rejects_uni_stream_id` を `test_register_local_wt_stream_accepts_local_uni_and_rejects_peer_uni` に置き換えた (server / client × local / peer uni の 4 パターンを検証)
  - `test_stop_sending_propagates_to_local_wt_uni_data_stream_server` / `_client` を追加した (登録済み uni への STOP_SENDING が `WebTransportEvent::StreamStopSending` (session_id 付き) として通知されることを Role::Server / Role::Client 両方で検証)
  - `test_stop_sending_falls_through_for_unregistered_uni_stream` を追加した (未登録 uni は汎用 `Event::StopSending` にフォールスルーする)
  - `test_wt_session_closed_event_carries_reliable_size_for_local_uni` を追加した (登録された uni が `SessionClosed.reset_streams` に reliable_size 付きで含まれる)
- `CHANGES.md` の `## develop` セクションに `[FIX]` エントリを追加した

### 対象外

- ローカル開始 uni ストリームへの RESET_STREAM / FIN 到着の sans-I/O 側防御コード追加は不要 (QUIC 層で STREAM_STATE_ERROR の接続エラーとなり sans-I/O へは到達しないため。RFC 9000 Section 19.4 / 19.8)
- `handle_wt_stream_reset` がローカル開始 uni について常に `local_initiated=false` と判定する既存の別バグ (仮に QUIC 層をバイパスして RESET が sans-I/O へ渡ると `on_remote_stream_closed` がピアの uni クレジットを不正回復する) は本 issue のスコープ外とし、別 issue で対応する
- `register_local_wt_stream` の critical stream (`control_send` / QPACK encoder / decoder) との衝突検出、event.rs 側 `SessionClosed` doc コメントの更新も別 issue で扱う

### 一次資料

- `refs/quic/rfc9000.txt` Section 2.1 (ストリーム種別) / Section 3.5 (STOP_SENDING) / Section 19.4 (RESET_STREAM) / Section 19.8 (STREAM)
- `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 4.2 (WT ストリーム) / Section 4.4 (STOP_SENDING 伝播)
