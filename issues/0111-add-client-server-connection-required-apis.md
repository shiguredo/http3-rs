# `ClientConnection` / `ServerConnection` に QUIC 統合必須 API を公開する

- Priority: Medium
- Created: 2026-06-15
- Model: Opus 4.7
- Branch: feature/add-client-server-connection-required-apis
- Polished: 2026-07-21

## 目的

`ClientConnection` / `ServerConnection` は `Connection` の薄ラッパだが、QUIC 統合に必須の API (`stream_reset` / `stop_sending` / `wt_data_consumed` / `take_wt_pending_capsules` 等) が公開されていない。ライブラリ利用者がこれらを呼ぶには `Connection::client(...)` / `Connection::server(...)` の生 API を使うしかなく、ラッパの存在意義が崩れている。ラッパの API を充実させて利用者が一貫した型を使えるようにする。

## 優先度根拠

Medium。lib.rs の doc は `ClientConnection::new` での利用を案内しているが、`stream_reset` が無いと QUIC 統合が成立しないため、利用者は途中で詰まって `Connection` に書き換える必要がある。API シェイプの一貫性に対する課題。

## 現状

ラッパが露出していない API:

- `Connection::stream_reset(stream_id, error_code, final_size)` — QUIC RESET_STREAM 受信時に必須
- `Connection::stop_sending(stream_id, error_code)` — QUIC STOP_SENDING 受信時に必須
- `Connection::wt_data_consumed(session_id, bytes)` — WT フロー制御
- `Connection::take_wt_pending_capsules(session_id)` — WT カプセル送信
- `Connection::wt_session_flow_control_enabled(session_id)`
- `Connection::wt_stream_header_len(stream_id)`
- `Connection::collect_closed_streams()` (死にコードなら削除候補)
- `Connection::mark_stream_fin_sent(stream_id)` (死にコードなら削除候補)
- `Connection::qpack_encoder` / `qpack_encoder_mut` / `qpack_decoder` / `qpack_decoder_mut`
- `Connection::encoder_stream*` / `decoder_stream*`
- `Connection::error()` (死にコードなら削除候補)

## 設計方針

- ラッパ方針を継続するなら、必須 API を `pub fn` でラッパに追加し、内部で `self.inner.xxx(...)` を呼ぶ
- 「ラッパは要らない」と判断するなら、`ClientConnection` / `ServerConnection` を廃止し `Connection` のみ公開する選択肢もある (破壊的変更)
- 死にコード API (`error()` 等) は別 issue 0113 で削除を扱うため、本 issue では生存している API のみ公開する
- 公開 API の `pub use` を `lib.rs` から再公開する

## 完了条件

- `ClientConnection` / `ServerConnection` から `stream_reset`, `stop_sending`, `wt_data_consumed`, `take_wt_pending_capsules` 等の QUIC 統合必須 API が呼べる
- `examples/wt_server` および `interop/h3` / `interop/wt` の利用箇所が `Connection` 直接呼び出しではなくラッパ経由で書ける
- `lib.rs` の doc 例がラッパで完結する
- `make fmt && make clippy && make check` が通る

## 解決方法

`src/connection/client.rs` / `src/connection/server.rs` にメソッドを追加:

```rust
impl ClientConnection {
    pub fn stream_reset(&mut self, stream_id: u64, error_code: u64, final_size: u64) -> Result<(), Error> {
        self.inner.stream_reset(stream_id, error_code, final_size)
    }
    pub fn stop_sending(&mut self, stream_id: u64, error_code: u64) -> Result<(), Error> {
        self.inner.stop_sending(stream_id, error_code)
    }
    // ...
}
```

ServerConnection も同様。

### 関連ファイル

- 修正対象: `src/connection/client.rs`, `src/connection/server.rs`, `src/lib.rs`
- 関連 issue: 0113 (死に API 削除)
