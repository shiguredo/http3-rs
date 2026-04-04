# interop_wt

WebTransport 相互運用性テスト

## 概要

異なる QUIC 実装間で WebTransport (RFC draft-ietf-webtrans-http3) の相互運用性を検証するテストスイート。

## テスト対象の QUIC 実装

| 実装 | 種別 | WebTransport 対応 | 備考 |
|---|---|---|---|
| s2n-quic + shiguredo_http3 | tokio 統合 | draft-02 / draft-07 / draft-15 | 全ドラフトバージョン対応 |
| ngtcp2 + nghttp3 | tokio 統合 | draft-15 のみ | RFC トラック版のみ |
| neqo (Mozilla) | Sans I/O | draft-02 のみ | NSS 依存、古いドラフト版 |
| quinn + h3-webtransport | tokio 統合 | draft-02 | h3-webtransport (0.1.2) + h3-quinn (0.0.10) |
| tquic (Tencent) | Sans I/O | 未対応 | HTTP/3 のみ |
| quiche (Cloudflare) | Sans I/O | 未対応 | HTTP/3 のみ |

## テスト構成

```
interop_wt/
  src/lib.rs          -- 共通ヘルパー関数
  neqo-db/            -- neqo 用 NSS テスト証明書データベース
  tests/
    ngtcp2_client_s2n_server.rs   -- ngtcp2 クライアント ↔ s2n-quic サーバー
    s2n_client_ngtcp2_server.rs   -- s2n-quic クライアント ↔ ngtcp2 サーバー
    quinn_client_s2n_server.rs    -- quinn クライアント ↔ s2n-quic サーバー
    s2n_client_quinn_server.rs    -- s2n-quic クライアント ↔ quinn サーバー
    ngtcp2_client_quinn_server.rs -- ngtcp2 クライアント ↔ quinn サーバー (draft 不一致で失敗)
    quinn_client_ngtcp2_server.rs -- quinn クライアント ↔ ngtcp2 サーバー (draft 不一致で失敗)
```

## テスト内容

各テストで以下の WebTransport 機能を検証する:

- セッション確立 (CONNECT + :protocol=webtransport)
- 双方向ストリーム (Section 4.3)
- 単方向ストリーム (Section 4.2)
- Datagram (Section 4.5)

## WebTransport SETTINGS のドラフトバージョン互換性

WebTransport の仕様は複数のドラフト版を経て進化しており、実装間で使用する SETTINGS 値が異なる。
これが相互運用性の主な障壁となっている。

### SETTINGS 値の対応表

| ドラフト版 | SETTINGS ID | 意味 | 値 |
|---|---|---|---|
| draft-02 | `0x2b603742` | ENABLE_WEBTRANSPORT | 0 or 1 |
| draft-07 | `0xc671706a` | WEBTRANSPORT_MAX_SESSIONS | セッション数上限 |
| draft-15 (RFC track) | `0x2c7cf000` | WT_ENABLED | 0 or 1 |

全ドラフト版共通で以下も必要:

| SETTINGS ID | 意味 | 参照 |
|---|---|---|
| `0x08` | ENABLE_CONNECT_PROTOCOL | RFC 9220 |
| `0x33` | H3_DATAGRAM | RFC 9297 |

### 各実装の SETTINGS 対応状況

#### shiguredo_http3 (s2n-quic 統合)

3 つのドラフト版全てを同時に送信する:

```
SETTINGS_ENABLE_WEBTRANSPORT (0x2b603742) = 1       // draft-02
SETTINGS_WEBTRANSPORT_MAX_SESSIONS (0xc671706a) = N  // draft-07
SETTINGS_WT_ENABLED (0x2c7cf000) = 1                 // draft-15
SETTINGS_ENABLE_CONNECT_PROTOCOL (0x08) = 1           // RFC 9220
SETTINGS_H3_DATAGRAM (0x33) = 1                       // RFC 9297
```

これにより、どのドラフト版の実装とも接続可能。

#### ngtcp2 / nghttp3

draft-15 (RFC トラック) のみ対応:

```
SETTINGS_WT_ENABLED (0x2c7cf000) = 1                 // draft-15
SETTINGS_ENABLE_CONNECT_PROTOCOL (0x08) = 1           // RFC 9220
SETTINGS_H3_DATAGRAM (0x33) = 1                       // RFC 9297
```

#### neqo (Mozilla)

draft-02 のみ対応:

```
SETTINGS_ENABLE_WEBTRANSPORT (0x2b603742) = 1        // draft-02
SETTINGS_ENABLE_CONNECT_PROTOCOL (0x08) = 1           // RFC 9220
SETTINGS_H3_DATAGRAM (0x33) = 1                       // RFC 9297
```

### 相互運用性マトリクス (WebTransport)

| クライアント \ サーバー | s2n-quic | ngtcp2 | quinn | neqo |
|---|---|---|---|---|
| **s2n-quic** | -- | OK (draft-15) | OK (draft-02) | NG (*1) |
| **ngtcp2** | OK (draft-15) | -- | NG (*4) | NG (*2) |
| **quinn** | OK (draft-02) | NG (*4) | -- | 未テスト |
| **neqo** | NG (*3) | NG (*2) | 未テスト | 未テスト |

- *1: neqo の NSS テスト証明書が有効期限切れ (2019年) のため s2n-quic クライアントが接続不可
- *2: neqo は draft-02 のみ、ngtcp2 は draft-15 のみ対応のため SETTINGS ネゴシエーション失敗 (`WT negotiated: false`)
- *3: neqo クライアントと s2n-quic サーバー間ではネゴシエーションは成功するが (s2n-quic が draft-02 対応のため)、draft-15 で必要なフロー制御パラメータが不足しセッション即時クローズ (`SessionClosed { Error(0) }`)
- *4: h3-webtransport (draft-02) と ngtcp2 (draft-15) の SETTINGS ドラフトバージョン不一致でセッション確立不可

### 今後の展望

- neqo が draft-15 に対応すれば ngtcp2 / s2n-quic との相互運用が可能になる
- quinn + h3-webtransport の統合により、更に多くの実装間テストが可能になる
- tquic / quiche は現時点で WebTransport 未対応

## テスト実行

```bash
# 全テスト実行
cargo test -p interop_wt

# 個別テスト実行
cargo test -p interop_wt --test ngtcp2_client_s2n_server
cargo test -p interop_wt --test s2n_client_ngtcp2_server
```

## 依存

- s2n-quic: AWS の QUIC 実装 (Rust)
- ngtcp2 / nghttp3: IETF リファレンス実装 (C)
- neqo: Mozilla Firefox の QUIC 実装 (Rust, NSS 依存)
- shiguredo_http3: Sans I/O HTTP/3 ライブラリ
