# ConnectRequest::from_headers() を追加する

Created: 2026-03-29
Completed: 2026-03-29
Model: Opus 4.6

## 概要

ヘッダーのペア列から `ConnectRequest` を構築する `from_headers()` メソッドを追加する。

## 根拠

moqt-rust-private の relay で、サーバー側が `Event::Header` から `:path`, `:authority` 等を手動抽出して WebTransport CONNECT リクエストを解釈している:

```rust
for event in events {
    match event {
        Event::Header { name, value, .. } => {
            if name == b":path" {
                path = value;
            } else if name == b":authority" {
                authority = value;
            }
        }
        Event::HeadersEnd { .. } => {
            headers_complete = true;
        }
        _ => {}
    }
}
```

このパターンは WebTransport サーバーを書く全ての利用者が繰り返す定形コード。`ConnectRequest` 型はフィールドを持っているのにパース機能がなく、検証 (`validate()`) しかできない。

## 対応方針

- `ConnectRequest::from_headers(headers: &[(&[u8], &[u8])]) -> Result<Self, ConnectError>` を追加する
  - `:method` が `CONNECT` であることを確認
  - `:protocol` が `webtransport-h3` または `webtransport` (#0002) であることを確認
  - `:scheme`, `:authority`, `:path` を抽出
  - `Origin`, `WT-Available-Protocols` があれば抽出
  - 不正な UTF-8 の場合は `ConnectError` に新バリアント (`InvalidEncoding`) を追加して返す
- `from_headers()` はパースのみ行い、`validate()` で RFC 準拠の検証を行う既存設計を維持する
- Sans I/O の範疇内 (純粋なデータ変換、I/O なし)

## 解決方法

`ConnectRequest::from_headers(headers: &[(&[u8], &[u8])]) -> Result<Self, ConnectError>` を `src/webtransport/connect.rs` に追加した。

- `:method` が `CONNECT` であることを検証 (`InvalidMethod`)
- `:protocol` が `webtransport-h3` または `webtransport` であることを検証 (`InvalidProtocol`)
- 不正な UTF-8 は `InvalidEncoding` エラーを返す
- `ConnectError` に `InvalidMethod`, `InvalidProtocol`, `InvalidEncoding` バリアントを追加
- ラウンドトリップテスト (`to_headers` → `from_headers`) で正しさを検証済み
