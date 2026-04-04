# セッション終端時の WT_SESSION_GONE 処理が Connection に統合されていない

Created: 2026-04-05
Completed: 2026-04-05
Model: Opus 4.6

## 優先度

P1

## 概要

`Session` 側にはセッション終端後に関連ストリームを `WT_SESSION_GONE` でリセットするための API が実装済みだが、`Connection` 側にはセッション表がなく、終端処理が live path に存在しなかった。

## 根拠

- draft-ietf-webtrans-http3-15 Section 6 (L1436-1450)

## 解決方法

1. `Connection` に `WtSession` 構造体と `wt_sessions: HashMap<u64, WtSession>` を導入した
2. `WtSession` はセッション状態 (Pending/Established/Closed)、関連ストリーム ID、バッファリングを管理する
3. セッションの登録タイミング:
   - クライアント: `send_request` で WT CONNECT 送信時 (Pending)
   - サーバー: `emit_header_events` で WT CONNECT 受信時 (Pending)
   - ストリーム先行到着: `associate_or_buffer_stream` でセッション未登録時に自動作成 (Pending)
4. セッション確立: クライアントが 200 OK を受信時に Established に遷移し、`WebTransportSessionEstablished` イベントを発火
5. セッション終了: `terminate_wt_session` メソッドを追加
   - CONNECT stream の FIN (StreamEnd) または RESET_STREAM で呼ばれる
   - 関連する全ストリーム ID を収集し、`WebTransportSessionClosed` イベントで通知
   - 呼び出し側は通知された stream_id に対して `WT_SESSION_GONE` で RESET_STREAM / STOP_SENDING を送信する
