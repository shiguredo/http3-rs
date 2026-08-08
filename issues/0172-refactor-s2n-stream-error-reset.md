# tokio-s2n-quic に STOP_SENDING / セッション終了への RESET 応答を配線する

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
- Branch: feature/refactor-s2n-stream-error-reset
- Polished: {YYYY-MM-DD}

## 目的

sans-I/O 層が発行する `Event::StopSending` と `WebTransportEvent::SessionClosed` を受けて、該当ストリームを RESET_STREAM で閉じる処理を統合層 (tokio-s2n-quic) に実装する。現在は RESET 応答が配線されておらず、QUIC 層のストリームが半開のまま残留する。

## 現状

- `crates/tokio-s2n-quic/src/` に `Event::StopSending` / `WebTransportEvent::SessionClosed` の処理が一切ない (grep 0 件)
- STOP_SENDING 受信時: sans-I/O 層は `Event::StopSending` を発行し送信バッファを破棄するが、統合層が RESET_STREAM で応答しないため QUIC 層の送信ストリームが半開のまま残留する (RFC 9000 Section 3.5: STOP_SENDING 受信時は RESET_STREAM を送る MUST)
- セッション終了時: tombstone 除去により CONNECT ストリームが `streams` から消え、`send_body` による FIN 応答は `StreamNotFound` になる。draft-ietf-webtrans-http3-16 Section 6 の「close or reset the stream in response」への応答は統合層の `SessionClosed` 処理 (RESET) の責務

## 設計方針

- h3/server.rs / h3/client.rs の受信ループで `Event::StopSending` を受けたら `send_stream.reset()` を呼ぶ
- `WtSession` の `connect_send` を RESET できるようにし、`SessionClosed` イベント受信時にリセットする (公開 API の追加が必要な場合はシグネチャを最小限に保つ)
- `reset_stream_on_stream_error` (crates/tokio-s2n-quic/src/internal/mod.rs) と同じエラーコード変換 (`application::Error::new`) を使う

## 完了条件

- STOP_SENDING 受信時に RESET_STREAM が送信される
- セッション終了 (`SessionClosed`) 時に CONNECT ストリームが RESET_STREAM で閉じられる
- tokio-s2n-quic の既存テスト (interop/h3, interop/wt) が通る

## 解決方法

(実装時に追記)
