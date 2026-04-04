# WT_CLOSE_SESSION の意味論が欠けている

Created: 2026-04-05
Completed: 2026-04-05
Model: Opus 4.6

## 優先度

P1

## 概要

`handle_wt_capsule` での `WT_CLOSE_SESSION` 処理が仕様どおりではない。3 つの問題がある。

## 根拠

draft-ietf-webtrans-http3-15 Section 6 の要求に対して、以下が未実装。

### 1. error_code / message の破棄

`src/connection/mod.rs` L1154-1162 で `error_code` と `message` を `let _ = (error_code, message);` で捨てている。`terminate_wt_session` は常に `WT_SESSION_GONE` 固定でイベントを生成するため、アプリケーション層はクローズ理由を知ることができない。

### 2. WT_CLOSE_SESSION 後の追加データ未拒否

仕様は以下を要求している:

> If any additional stream data is received on the CONNECT stream after receiving a WT_CLOSE_SESSION capsule, the stream MUST be reset with code H3_MESSAGE_ERROR.

現実装にはこの制御がない。WT_CLOSE_SESSION を受信した後も CONNECT ストリーム上の後続データがそのまま処理される。

### 3. FIN = code 0 の等価性が未表現

仕様は以下を要求している:

> Cleanly terminating a CONNECT stream without a WT_CLOSE_SESSION capsule SHALL be semantically equivalent to terminating it with a WT_CLOSE_SESSION capsule that has an error code of 0 and an empty error string.

現実装では FIN 受信時に `terminate_wt_session` が呼ばれるが、error_code=0 としてアプリケーション層に通知する等価な処理がない。

## 対応方針

- `WebTransportSessionClosed` イベントに受信した `error_code` と `message` を含める
- `terminate_wt_session` にエラーコードを引数で渡せるようにする
- WT_CLOSE_SESSION 受信後の CONNECT ストリーム状態を追跡し、追加データ受信時に `H3_MESSAGE_ERROR` でリセットする
- FIN 受信パスでは error_code=0, message="" として同イベントを生成する

## 解決方法

- `terminate_wt_session_with` メソッドを新設し、エラーコード (WT_SESSION_GONE / WT_ALPN_ERROR 等)、close_error_code、close_message を引数で受け取るようにした
- `WebTransportSessionClosed` イベントに `close_error_code: u32` と `close_message: String` フィールドを追加した
- `handle_wt_capsule` で WT_CLOSE_SESSION 受信時に error_code / message をイベントに含めるようにした
- `WtSession` に `close_session_received` フラグを追加し、WT_CLOSE_SESSION 受信後の追加データを `H3_MESSAGE_ERROR` で拒否するようにした
- `terminate_wt_session` (引数なし版) は error_code=0, message="" の既定値で `terminate_wt_session_with` を呼ぶラッパーとして残し、FIN / RESET_STREAM での終了は従来どおり動作する

## 参照

- draft-ietf-webtrans-http3-15 Section 6
- `src/connection/mod.rs` L1154-1162, L1219-1234
- `src/event.rs` L110-117
