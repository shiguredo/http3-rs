# ConnectRequest::to_headers() / ConnectResponse::to_headers() を追加する

Created: 2026-03-29
Completed: 2026-03-29
Model: Opus 4.6

## 概要

`webtransport::connect::ConnectRequest` と `ConnectResponse` にヘッダー配列への変換メソッドを追加する。

## 根拠

moqt-rust-private の publisher / subscriber / relay の全てで、WebTransport CONNECT リクエストのヘッダーを手動構築している:

```rust
let headers = vec![
    Header::new(b":method", b"CONNECT"),
    Header::new(b":protocol", b"webtransport"),
    Header::new(b":scheme", b"https"),
    Header::new(b":path", path.as_bytes()),
    Header::new(b":authority", server_name.as_bytes()),
];
```

`ConnectRequest` は既に `scheme`, `authority`, `path` フィールドを持っているが、`Header` 配列への変換メソッドがない。検証専用の型になっていてコーデックとして不完全。

同様に `ConnectResponse` も `:status` やオプショナルヘッダー (`WT-Protocol`, `sec-webtransport-http3-draft`) を含むヘッダー配列を生成できるべき。

## 対応方針

- `ConnectRequest::to_headers(&self) -> Vec<Header>` を追加する
  - `:method`, `:protocol`, `:scheme`, `:authority`, `:path` を生成
  - `origin` が Some なら `Origin` ヘッダーも追加
  - `available_protocols` が非空なら `WT-Available-Protocols` ヘッダーも追加
- `ConnectResponse::to_headers(&self) -> Vec<Header>` を追加する
  - `:status` を生成
  - `selected_protocol` が Some なら `WT-Protocol` ヘッダーも追加
- `:protocol` 値は #0002 の解決後に決定する
- Sans I/O の範疇内 (純粋なデータ変換、I/O なし)

## 解決方法

`ConnectRequest::to_headers()` と `ConnectResponse::to_headers()` を `src/webtransport/connect.rs` に追加した。

- `ConnectRequest::to_headers()`: `:method`, `:protocol`, `:scheme`, `:authority`, `:path` ヘッダーを生成。`origin` と `available_protocols` がある場合はそれも含める
- `ConnectResponse::to_headers()`: `:status` ヘッダーを生成。`selected_protocol` がある場合は `wt-protocol` ヘッダーも含める
- `:protocol` 値は `webtransport-h3` (draft-15) を使用 (#0002 参照)
