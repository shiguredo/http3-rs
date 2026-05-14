# 0067: send_goaway のサーバー用単調減少チェックが不十分

Created: 2026-05-14
Model: deepseek-v4-pro

## 概要

`src/connection/mod.rs:3727-3732` の `send_goaway` において、
RFC 9114 Section 5.2 の単調減少チェックがロールを考慮していない。

- サーバー: "strictly lower than the identifier in the previous GOAWAY frame" (厳密減少)
- クライアント: "less than or equal to" (非増加)

現在の実装は `id > last_id` のみを拒否しており、サーバー側で `id == last_id` が通過してしまう。

## 修正方針

ロール別の分岐を追加する。

```rust
// 修正前
if let Some(last_id) = self.last_sent_goaway_id
    && id > last_id
{
    return Err(Error::ConnectionError(ErrorCode::IdError));
}

// 修正後 (サーバーは id >= last_id で拒否、クライアントは id > last_id で拒否)
match self.role {
    Role::Server => {
        if let Some(last_id) = self.last_sent_goaway_id && id >= last_id {
            return Err(Error::ConnectionError(ErrorCode::IdError));
        }
    }
    Role::Client => {
        if let Some(last_id) = self.last_sent_goaway_id && id > last_id {
            return Err(Error::ConnectionError(ErrorCode::IdError));
        }
    }
}
```

## 影響範囲

- `src/connection/mod.rs:3727-3732`
- RFC 9114 Section 5.2
