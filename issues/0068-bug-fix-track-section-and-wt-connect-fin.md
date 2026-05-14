# 0068: send_request/send_response の track_section 順序と WT CONNECT FIN 拒否

Created: 2026-05-14
Model: deepseek-v4-pro

## 概要

`src/connection/mod.rs` の `send_request` / `send_response` において複数の問題がある。

### 問題 1: track_section が send_encoded_headers より先に呼ばれる

- `mod.rs:3432-3433` (send_request)
- `mod.rs:3553-3555` (send_response)

`qpack_encoder.track_section(stream_id, ric)` が `stream.send_encoded_headers()` より
先に実行される。後者がストリーム状態不正等でエラーを返した場合、
セクションがエンコーダーに登録されたままになり、QPACK エンコーダーの状態不整合と
リソースリークが発生する (RFC 9204 Section 2.1.1, 4.4.1)。

### 問題 2: WebTransport CONNECT で fin=true が拒否されない

- `mod.rs:3449-3456`

plain CONNECT の場合は `fin=true` が `StreamError(MessageError)` で拒否されるが、
WebTransport CONNECT (`has_protocol=true`) の場合はスルーされる。
CONNECT ストリームは長期生存双方向ストリームであり、FIN 送信で後続の
Capsule プロトコル通信が不可能になる (draft-ietf-webtrans-http3-15 Section 3)。

## 修正方針

### 修正 1

`track_section` を `send_encoded_headers` の成功後に移動する。

```rust
// 修正前
let ric = self.qpack_encoder.last_required_insert_count();
self.qpack_encoder.track_section(stream_id, ric);
// ... stream.send_encoded_headers(&qpack_buf, fin, false)?;

// 修正後
let ric = self.qpack_encoder.last_required_insert_count();
// ... stream.send_encoded_headers(&qpack_buf, fin, false)?;
self.qpack_encoder.track_section(stream_id, ric);
```

### 修正 2

WT CONNECT でも FIN を拒否する。

```rust
// 修正前
if is_connect && !has_protocol {
    if fin { return Err(...); }
    stream.set_connect_request();
}

// 修正後
if is_connect {
    if fin { return Err(...); }
    if !has_protocol { stream.set_connect_request(); }
}
```

## 影響範囲

- `src/connection/mod.rs:3432-3433,3449-3456,3553-3555`
