# 0082: Event enum の WebTransport バリアントをネスト化する

- Priority: Low
- Created: 2026-05-14
- Model: deepseek-v4-pro
- Branch: feature/change-event-enum-nesting

## 目的

`src/event.rs` の `Event` enum が 25 バリアントでフラットに肥大化している。`WebTransport*` プレフィックスで区別されているが、フラットな構造では将来の追加でさらに肥大化する。WebTransport 関連イベントを `Event::WebTransport(WebTransportEvent)` にネスト化し、関心の分離を明確にする。

## 優先度根拠

Low: 機能に影響しない API 構造の改善。ただし後方互換のない変更であるため、他の CHANGE と合わせて実施する方が効率的。

## 現状

- `Event` enum: 25 バリアント
- WebTransport 関連バリアント: `WebTransportBidiStreamOpen`, `WebTransportSessionEstablished`, `WebTransportDatagram`, `WebTransportUniStreamOpen`, `WebTransportSessionClosed`, `WebTransportSessionDraining` 等

## 設計方針

```rust
pub enum Event {
    SettingsReceived { ... },
    HeadersBegin { stream_id: u64 },
    // ... (HTTP/3 イベント)
    WebTransport(WebTransportEvent),
    ConnectionError { ... },
}

pub enum WebTransportEvent {
    BidiStreamOpen { stream_id: u64, session_id: u64 },
    SessionEstablished { session_id: u64, ... },
    Datagram { session_id: u64, payload: Vec<u8> },
    // ...
}
```

注: issue 0059 (bytes クレート導入) は見送られたため (`2192c91`)、`Vec<u8>` を維持する。

## 完了条件

- WebTransport イベントが `WebTransportEvent` enum に集約されていること
- `Event::WebTransport(WebTransportEvent)` バリアントが追加されていること
- 全 match 分岐が更新されていること
- `cargo test` と相互運用テストが pass すること

## 後方互換性

`Event` enum のバリアント構造が変わるため後方互換のない変更。`[CHANGE]` として記録する。

## 影響範囲

- `src/event.rs`: `Event` enum 再構成、`WebTransportEvent` enum 新設
- `src/connection/mod.rs`: イベント発行箇所の全 WebTransport イベント
- `examples/wt_server`: match 分岐の更新

## CHANGES.md エントリ案

```
- [CHANGE] Event enum の WebTransport バリアントを Event::WebTransport(WebTransportEvent) にネスト化する
  - @担当者
```
