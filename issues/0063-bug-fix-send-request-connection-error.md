# 0063: send_request が GOAWAY 境界超過 / WT セッション上限超過時に接続全体を閉じる

Created: 2026-05-14
Model: deepseek-v4-pro

## 概要

`src/connection/mod.rs` の `send_request` において、2 箇所で `ConnectionError(RequestRejected)` を返している。
`H3_REQUEST_REJECTED` はストリームレベルのエラーであり、接続全体を閉じるべきではない。

## 対象箇所

1. `mod.rs:3401-3403`: フロー制御なし WT セッション上限超過時
   - draft-ietf-webtrans-http3-15 Section 5.1
   - 同時 1 セッションまでの制限を超えた場合

2. `mod.rs:3411-3414`: GOAWAY 受信後の新規リクエスト作成時
   - RFC 9114 Section 5.2
   - GOAWAY 境界値以上のストリーム ID でのリクエスト作成

## 修正方針

両方とも `ConnectionError` を `StreamError` に変更する。

```rust
// 修正前
return Err(Error::ConnectionError(ErrorCode::RequestRejected));

// 修正後
return Err(Error::StreamError(ErrorCode::RequestRejected));
```

## 影響範囲

- `src/connection/mod.rs:3402,3414`
- `tests/test_webtransport_draft_connect.rs:674` (テストの期待値修正も必要)
