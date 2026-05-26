# 0082: Event enum の WebTransport バリアントを Event::WebTransport(WebTransportEvent) にネスト化する

- Priority: Low
- Created: 2026-05-14
- Polished: 2026-05-26
- Model: deepseek-v4-pro
- Branch: feature/change-event-enum-nesting

## 目的

`src/event.rs` の `Event` enum が 24 バリアントでフラットに肥大化している。WebTransport 関連の 14 バリアントを `Event::WebTransport(WebTransportEvent)` にネスト化し、関心の分離を明確にする。ネスト化により `Event` は 11 バリアント (非 WT 10 + `WebTransport` 1) に縮小する。

## 優先度根拠

Low: 機能に影響しない API 構造の改善。ただし後方互換のない変更であるため、他の CHANGE と合わせて実施する方が効率的。

## 依存関係

- issue 0077 (connection モジュール分割) で `connection/mod.rs` のイベント発行コードが約 72 箇所変更対象になる。 0077 が先に実施されると発行箇所が複数ファイルに分散し、0082 の影響範囲が広がる。 0082 を先に実施するか、0077 と同時に行うことを推奨する

## 現状

- `Event` enum: 24 バリアント (非 WebTransport 10 + WebTransport 14)
- `Event::WebTransport*` の参照箇所: `connection/mod.rs` 本体に約 40 箇所、同ファイルのインラインテストに約 30 箇所、`tests/` に 2 箇所
- `impl Event::stream_id()`: 14 個の WT バリアントに対する match 分岐あり

### WebTransport バリアント (14 個)

1. `WebTransportBidiStreamOpen { stream_id, session_id }`
2. `WebTransportBidiStreamData { stream_id, data }`
3. `WebTransportBidiStreamEnd { stream_id }`
4. `WebTransportUniStreamOpen { stream_id, session_id }`
5. `WebTransportUniStreamData { stream_id, data }`
6. `WebTransportUniStreamEnd { stream_id }`
7. `WebTransportSessionClosed { session_id, reset_streams, error_code, close_error_code, close_message }`
8. `WebTransportSessionEstablished { session_id, flow_control_enabled }`
9. `WebTransportSessionDraining { session_id }`
10. `WebTransportCapsule { session_id, capsule }`
11. `WebTransportDatagram { session_id, payload }`
12. `WebTransportStreamReset { session_id, stream_id, error_code, final_size }`
13. `WebTransportStreamStopSending { session_id, stream_id, error_code }`
14. `WebTransportBufferedStreamRejected { stream_id, error_code }`

## 設計方針

### 新しい型定義

```rust
// src/event.rs

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    SettingsReceived { ... },
    HeadersBegin { stream_id: u64 },
    Header { stream_id: u64, name: Vec<u8>, value: Vec<u8> },
    HeadersEnd { stream_id: u64 },
    Data { stream_id: u64, data: Vec<u8> },
    StreamEnd { stream_id: u64 },
    StreamReset { stream_id: u64, error_code: u64 },
    StopSending { stream_id: u64, error_code: u64 },
    GoawayReceived { id: VarInt },
    WebTransport(WebTransportEvent),
    ConnectionError { error_code: u64, reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebTransportEvent {
    BidiStreamOpen { stream_id: u64, session_id: u64 },
    BidiStreamData { stream_id: u64, data: Vec<u8> },
    BidiStreamEnd { stream_id: u64 },
    UniStreamOpen { stream_id: u64, session_id: u64 },
    UniStreamData { stream_id: u64, data: Vec<u8> },
    UniStreamEnd { stream_id: u64 },
    SessionClosed { session_id: u64, reset_streams: Vec<WtStreamReset>, error_code: u64, close_error_code: u32, close_message: String },
    SessionEstablished { session_id: u64, flow_control_enabled: bool },
    SessionDraining { session_id: u64 },
    Capsule { session_id: u64, capsule: crate::webtransport::Capsule },
    Datagram { session_id: u64, payload: Vec<u8> },
    StreamReset { session_id: u64, stream_id: u64, error_code: u64, final_size: u64 },
    StreamStopSending { session_id: u64, stream_id: u64, error_code: u64 },
    BufferedStreamRejected { stream_id: u64, error_code: u64 },
}
```

### `WtStreamReset` 構造体の扱い

`WtStreamReset` 構造体 (event.rs:7-23) は `WebTransportSessionClosed` バリアントのフィールドで使用されている。 `WebTransportEvent` と同じファイル (`event.rs`) に残す。

### `impl Event::stream_id()` の更新

ネスト化後は `Event::WebTransport(wt)` に対してマッチし、`WebTransportEvent` 側に `stream_id()` メソッドを追加して委譲する:

```rust
impl Event {
    pub fn stream_id(&self) -> Option<u64> {
        match self {
            Self::HeadersBegin { stream_id } | ... => Some(*stream_id),
            Self::WebTransport(wt) => wt.stream_id(),
            _ => None,
        }
    }
}

impl WebTransportEvent {
    pub fn stream_id(&self) -> Option<u64> {
        match self {
            Self::BidiStreamOpen { stream_id, .. } | ... => Some(*stream_id),
            Self::SessionClosed { session_id, .. } | ... => Some(*session_id),
        }
    }
}
```

## 完了条件

- WebTransport イベント 14 個が `WebTransportEvent` enum に集約されていること
- `Event::WebTransport(WebTransportEvent)` バリアントが追加されていること
- `impl Event::stream_id()` と新設 `impl WebTransportEvent::stream_id()` が正しく動作すること
- `connection/mod.rs` の全 match 分岐 (本体約 40 箇所 + テスト約 30 箇所) が更新されていること
- `cargo test --workspace` が全て pass すること
- 相互運用テストが pass すること

## 後方互換性

`Event` enum のバリアント構造が変わるため後方互換のない変更。 `[CHANGE]` として記録する。

## 影響範囲

- `src/event.rs`: `Event` enum 再構成、`WebTransportEvent` enum 新設、`stream_id()` メソッド更新
- `src/connection/mod.rs`: イベント発行箇所 約 40 箇所 + インラインテスト約 30 箇所の match パターン変更
- `src/lib.rs`: `pub use event::WebTransportEvent` の追加
- `tests/test_webtransport_draft_connect.rs`: 2 箇所の match パターン変更
- 影響なし (確認済み): `examples/wt_server` (WebTransport Event を直接 match していない)
- 影響なし (確認済み): `interop/` (本クレートの `Event` を使用していない)

## CHANGES.md エントリ案

```markdown
- [CHANGE] `Event` enum の WebTransport バリアント 14 個を `Event::WebTransport(WebTransportEvent)` にネスト化する
  - @voluntas
```
