# ローカル開始の WT uni ストリームでピアの STOP_SENDING が通知されない

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
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

### 関連ファイル

- `src/connection/wt_session.rs` (`Connection::handle_wt_stop_sending`)
- `src/connection/wt_stream.rs` (`Connection::register_local_wt_stream` の uni 対応)
- `src/connection/mod.rs` (`wt_uni_streams` / 登録 API / テスト)
- 関連 issue: 0144 (ローカル開始 WT bidi ストリームの登録 API。本 issue は uni 側の拡張。0144 の「uni ストリームの RESET / FIN 伝播は 0170 で対応」という言及は、RESET_STREAM / FIN が send-only ストリームでは STREAM_STATE_ERROR の接続エラーになるため、本 issue の対象外に変更される)
- 一次資料: `refs/quic/rfc9000.txt` Section 2.1 / 3.5 / 19.4 / 19.8、`refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 4.2 / 4.4

(実装時に追記)
