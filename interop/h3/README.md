# interop_h3

HTTP/3 相互運用性テスト

## 概要

異なる QUIC 実装間で HTTP/3 (RFC 9114) の相互運用性を検証するテストスイート。
6 つの QUIC 実装間で全方向のクロステストを行う。

## テスト対象の QUIC 実装

| 実装 | 種別 | TLS バックエンド | 備考 |
|---|---|---|---|
| s2n-quic (AWS) | tokio 統合 | s2n-tls | shiguredo_http3 による HTTP/3 処理 |
| quiche (Cloudflare) | Sans I/O | BoringSSL | HTTP/3 内蔵 |
| quinn (hyperium) | tokio 統合 | rustls (aws-lc-rs) | h3 + h3-quinn による HTTP/3 処理 |
| ngtcp2 / nghttp3 | tokio 統合 | wolfSSL | IETF リファレンス実装 |
| tquic (Tencent) | Sans I/O | BoringSSL | HTTP/3 内蔵 |

## テスト構成

```
interop/h3/
  src/lib.rs          -- 共通ヘルパー関数
  tests/
    advanced.rs                     -- shiguredo_http3 の高度なテスト
    {client}_client_{server}_server.rs  -- クロステスト (30 ファイル)
```

## 相互運用性マトリクス (HTTP/3)

全組み合わせの GET リクエスト/レスポンスをテストする。

| クライアント \ サーバー | s2n-quic | quiche | quinn | ngtcp2 | tquic |
|---|---|---|---|---|---|
| **s2n-quic** | -- | OK | OK | OK | OK |
| **quiche** | OK | -- | OK | -- | OK |
| **quinn** | OK | OK | -- | OK | OK |
| **ngtcp2** | OK | -- | OK | -- | OK |
| **tquic** | OK | OK | OK | OK | -- |

### 証明書検証の扱い

- 各テストは `rcgen` で自己署名証明書を生成し、全実装で共有する
- クライアント側は証明書検証を無効化して接続する

### Sans I/O 実装の扱い

tquic は `Rc` を使用しており `!Send` のため、tokio タスクに直接乗せられない。
`std::thread` + blocking UDP ソケットで駆動し、`std::sync::mpsc::channel` で結果を返す。

## テスト実行

```bash
# 全テスト実行
cargo test -p interop_h3

# 個別テスト実行
cargo test -p interop_h3 --test quinn_client_s2n_server
cargo test -p interop_h3 --test tquic_client_ngtcp2_server
```

## 依存

- s2n-quic: AWS の QUIC 実装 (Rust)
- quiche: Cloudflare の QUIC 実装 (Rust, BoringSSL)
- quinn: 純 Rust QUIC 実装 (rustls)
- h3 / h3-quinn: HTTP/3 プロトコル実装
- ngtcp2 / nghttp3: IETF リファレンス実装 (C)
- tquic: Tencent の QUIC 実装 (Rust, BoringSSL)
- shiguredo_http3: Sans I/O HTTP/3 ライブラリ
