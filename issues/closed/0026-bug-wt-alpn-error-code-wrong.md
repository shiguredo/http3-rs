# WT_ALPN_ERROR が WT_SESSION_GONE に潰れている

Created: 2026-04-05
Completed: 2026-04-05
Model: Opus 4.6

## 優先度

P2

## 概要

WT-Protocol の不一致時に `terminate_wt_session()` を使用しているため、エラーコードが `WT_ALPN_ERROR` ではなく `WT_SESSION_GONE` になる。

## 根拠

draft-ietf-webtrans-http3-15 Section 3.3 は以下を要求している:

> If the client receives a WT-Protocol value that was not included in its WT-Available-Protocols list, the client MUST close the WebTransport session with a WT_ALPN_ERROR error code.

`src/connection/mod.rs` L1845-1847 で `terminate_wt_session(stream_id)` を呼んでいるが、この関数は常に `WT_SESSION_GONE` (0x170d7b68) でイベントを生成する。`WT_ALPN_ERROR` (0x0817b3dd) が使われない。

## 影響

エラーの意味論が破壊される。クライアント側のアプリケーションがクローズ理由を正しく判別できない。機能的には動作するが、デバッグやエラーハンドリングに支障が出る。

## 対応方針

`terminate_wt_session` にエラーコードを引数で渡せるようにし、ALPN 不一致時は `WT_ALPN_ERROR` を指定する。この変更は issue 0024 (WT_CLOSE_SESSION の error_code 対応) と同じ `terminate_wt_session` のリファクタリングに含められる。

## 解決方法

issue 0024 と同時に対応。`terminate_wt_session_with` を使い、ALPN 不一致時は `WtErrorCode::AlpnError` をエラーコードとして渡すようにした。

## 参照

- draft-ietf-webtrans-http3-15 Section 3.3
- `src/connection/mod.rs` L1845-1847
- `src/webtransport/error.rs` L26-31
