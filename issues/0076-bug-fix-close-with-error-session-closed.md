# 0076: close_with_error がクローズ済みセッションでカプセル未送信状態を作る

Created: 2026-05-14
Model: deepseek-v4-pro

## 概要

`src/webtransport/session.rs:763-783` の `close_with_error` は:

1. `queue_capsule(CloseSession)` → クローズ済みチェックにより早期リターン
2. `close_session_sent = true` → フラグは設定される

という順序で実行される。`close()` が先行して呼ばれセッションが既に `Closed` の場合、
`queue_capsule` は `is_closed()` で早期リターンし `CLOSE_SESSION` カプセルが
キューされないが、`close_session_sent = true` は設定される。
結果として「カプセル未送信なのに送信済みフラグが立つ」状態になる。

## 再現手順

1. ピアが先に `WT_CLOSE_SESSION` を送信
2. `process_capsule(CloseSession)` が `close()` を呼び出しセッションが `Closed` になる
3. 応答としてアプリが `close_with_error` を呼び出す
4. `queue_capsule` は `is_closed()` でスキップされるが `close_session_sent = true` になる

## 修正方針

`close_session_sent` をカプセルが実際にキューされた場合のみ true にする。

```rust
// 修正前
session.queue_capsule(Capsule::CloseSession { ... })?;
session.close_session_sent = true;
session.close();

// 修正後 (カプセルキュー成功時のみフラグを立てる)
if session.queue_capsule(Capsule::CloseSession { ... }).is_ok() {
    session.close_session_sent = true;
}
session.close();
```

## 影響範囲

- `src/webtransport/session.rs:763-783`
