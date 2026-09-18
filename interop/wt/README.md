# interop_wt

WebTransport 相互運用性テスト

## 概要

異なる QUIC 実装間で WebTransport (RFC draft-ietf-webtrans-http3) の相互運用性を検証するテストスイート。

## テスト対象の QUIC 実装

| 実装 | 種別 | WebTransport 対応 | 備考 |
|---|---|---|---|
| s2n-quic + shiguredo_http3 | tokio 統合 | draft-02 / draft-07 / draft-15 | 全ドラフトバージョン対応 |
| ngtcp2 + shiguredo_http3 | tokio 統合 | draft-02 / draft-07 / draft-15 | ngtcp2 (C) で QUIC、shiguredo_http3 で HTTP/3 |
| quinn + h3-webtransport | tokio 統合 | draft-02 | h3-webtransport (0.1.2) + h3-quinn (0.0.10) |
| tquic (Tencent) | Sans I/O | 未対応 | HTTP/3 のみ |
| quiche (Cloudflare) | Sans I/O | 未対応 | HTTP/3 のみ |

## テスト構成

```
interop/wt/
  src/lib.rs          -- 共通ヘルパー関数
  tests/
    ngtcp2_client_s2n_server.rs   -- ngtcp2 クライアント ↔ s2n-quic サーバー
    s2n_client_ngtcp2_server.rs   -- s2n-quic クライアント ↔ ngtcp2 サーバー
    quinn_client_s2n_server.rs    -- quinn クライアント ↔ s2n-quic サーバー
    s2n_client_quinn_server.rs    -- s2n-quic クライアント ↔ quinn サーバー
    ngtcp2_client_quinn_server.rs -- ngtcp2 クライアント ↔ quinn サーバー (draft-02 でネゴシエーション)
    quinn_client_ngtcp2_server.rs -- quinn クライアント ↔ ngtcp2 サーバー (draft 不一致で CONNECT を拒否)
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

#### ngtcp2 + shiguredo_http3

draft-02 / draft-07 / draft-15 を同時に広告し、ピアに合わせてネゴシエーションする:

```
SETTINGS_ENABLE_WEBTRANSPORT (0x2b603742) = 1        // draft-02
SETTINGS_WEBTRANSPORT_MAX_SESSIONS (0xc671706a) = 1  // draft-07
SETTINGS_WT_ENABLED (0x2c7cf000) = 1                 // draft-15
SETTINGS_ENABLE_CONNECT_PROTOCOL (0x08) = 1           // RFC 9220
SETTINGS_H3_DATAGRAM (0x33) = 1                       // RFC 9297
```

WT_INITIAL_MAX_* を送信しないため WebTransport のフロー制御は無効になり、
ストリーム数とデータ量は QUIC のフロー制御だけで制限される。

#### 相互運用性マトリクス (WebTransport)

| クライアント \ サーバー | s2n-quic | ngtcp2 | quinn |
|---|---|---|---|
| **s2n-quic** | -- | OK (draft-15) | OK (draft-02) |
| **ngtcp2** | OK (draft-15) | -- | OK (draft-02) |
| **quinn** | OK (draft-02) | NG (*1) | -- |

- *1: h3-webtransport は SETTINGS_WT_ENABLED を送信しないため、draft-15 を広告するサーバーは CONNECT を拒否する (draft-ietf-webtrans-http3-15 Section 3.1)

### 今後の展望

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
- ngtcp2: IETF リファレンス実装 (C)。QUIC のみを担い、HTTP/3 は shiguredo_http3 を使用する
- shiguredo_http3: Sans I/O HTTP/3 ライブラリ
