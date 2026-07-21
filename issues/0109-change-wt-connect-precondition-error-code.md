# WebTransport CONNECT 前提条件違反のエラーを `InternalError` から区別可能な形に変更する

- Priority: Medium
- Created: 2026-06-15
- Model: Opus 4.7
- Branch: feature/change-wt-connect-precondition-error-code
- Polished: 2026-07-21

## 目的

`src/connection/mod.rs:3404-3439` で WebTransport CONNECT を送信する前提条件チェックの 8 種類すべて (peer SETTINGS 未受信 / peer が WT 未広告 / draft 不明 / プロトコル未広告 / H3_DATAGRAM 無効 / transport parameter 未注入 / reset_stream_at 未サポート / `:protocol` 不一致) を `Error::ConnectionError(ErrorCode::InternalError)` で返している。呼び出し側がどの前提条件を直すべきか判断できないため、構造化されたエラー variant を導入する。

## 優先度根拠

Medium。仕様違反ではないが API 利用者のデバッグ性に直結する設計負債。`H3_INTERNAL_ERROR` (0x102) は本来「内部ロジックバグ」を意味する RFC 9114 のコードで、呼び出し順序エラー全部を区別なく被せるのはセマンティクス的に誤り。

## 現状

`src/connection/mod.rs:3404-3439` 抜粋:

```rust
if self.peer_settings.is_none() {
    return Err(Error::ConnectionError(ErrorCode::InternalError));
}
let Some(peer_settings) = self.peer_settings else {
    return Err(Error::ConnectionError(ErrorCode::InternalError));
};
if !peer_settings.is_webtransport_enabled() {
    return Err(Error::ConnectionError(ErrorCode::InternalError));
}
// 以下 8 種類のチェックが全て同じ InternalError を返す
```

## 設計方針

- `Error` enum (もしくは新規 `WtSetupError` enum) に「呼び出し順序 / 前提条件不足」を表す variant を追加
- 8 種類のチェックがそれぞれ異なる variant を返すように分離
- 既存利用者の影響を抑えるため、`Error::PreconditionFailed { what: WtSetupError }` のような形を検討
- `CHANGES.md` に `[CHANGE] WT CONNECT 前提条件違反のエラーを <新 variant> に変更する` を追加

## 完了条件

- 8 種類の前提条件違反がそれぞれ区別可能なエラーで返る
- 既存テストおよび `tests/test_webtransport_draft_connect.rs` がパスする (期待エラーコードを更新)
- `make fmt && make clippy && make check` が通る

## 解決方法

`src/error.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WtSetupError {
    PeerSettingsUnreceived,
    PeerDoesNotSupportWebTransport,
    PeerDraftVersionUnknown,
    ConnectProtocolNotAdvertised,
    H3DatagramDisabled,
    TransportParameterMissing,
    ResetStreamAtUnsupported,
    ProtocolValueMismatch,
}
```

`Error` に `WtSetupFailed(WtSetupError)` variant を追加し、`send_request` 内で適切な variant を返す。

### 関連ファイル

- 修正対象: `src/connection/mod.rs:3404-3439`, `src/error.rs`
- 影響: `tests/test_webtransport_draft_connect.rs`, `examples/wt_server`
- `CHANGES.md` 追記必要
