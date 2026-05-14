# 0082: Event enum の WebTransport バリアントをネスト化する

Created: 2026-05-14
Model: deepseek-v4-pro

## 概要

`src/event.rs` の `Event` enum が 25 バリアントで肥大化している。
`WebTransport*` プレフィックスで区別されているが、フラットな enum 構造では
将来的な追加でさらに肥大化する。

## 修正方針

`WebTransportBidiStreamOpen`, `WebTransportSessionEstablished`, `WebTransportDatagram` 等の
WT イベントを `Event::WebTransport(WebTransportEvent)` にネスト化する。

**依存**: 0059 (`bytes` クレート導入) 完了後に着手すること。0059 で `Vec<u8>` → `Bytes` が
変更されるため。

```rust
pub enum Event {
    SettingsReceived { ... },
    HeadersBegin { stream_id: u64 },
    // ...
    WebTransport(WebTransportEvent),
    ConnectionError { ... },
}

pub enum WebTransportEvent {
    BidiStreamOpen { stream_id: u64, session_id: u64 },
    SessionEstablished { session_id: u64, ... },
    Datagram { session_id: u64, payload: Bytes },
    // ...
}
```

## 影響範囲

- `src/event.rs`
- 全呼び出し元の `match` 分岐 (主に `connection/mod.rs`、tokio crates)
