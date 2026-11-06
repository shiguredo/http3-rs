# http3-rs

[![crates.io](https://img.shields.io/crates/v/shiguredo_http3.svg)](https://crates.io/crates/shiguredo_http3)
[![docs.rs](https://docs.rs/shiguredo_http3/badge.svg)](https://docs.rs/shiguredo_http3)
[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)
[![GitHub Actions](https://github.com/shiguredo/http3-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/shiguredo/http3-rs/actions/workflows/ci.yml)
[![Discord](https://img.shields.io/badge/Discord-%235865F2.svg?logo=discord&logoColor=white)](https://discord.gg/shiguredo)

> [!WARNING]
> このライブラリは開発中であり、仕様が積極的に変更される場合があります。

## About Shiguredo's open source software

We will not respond to PRs or issues that have not been discussed on Discord. Also, Discord is only available in Japanese.

Please read <https://github.com/shiguredo/oss> before use.

## 時雨堂のオープンソースソフトウェアについて

利用前に <https://github.com/shiguredo/oss> をお読みください。

## 概要

Rust で実装された依存 0 かつ Sans I/O な HTTP/3 と WebTransport over HTTP/3 ライブラリです。

## 特徴

- Sans I/O
  - <https://sans-io.readthedocs.io/index.html>
- 依存ライブラリ 0
- QUIC 非依存
- RFC 9114 (HTTP/3) および RFC 9204 (QPACK) 準拠
- WebTransport over HTTP/3 (draft-ietf-webtrans-http3 の draft-02 / 07 / 14 / 15 / 16 / 17) サポート

## 使い方

Sans I/O ライブラリのため、QUIC の送受信は利用側が行います。

### クライアント (リクエスト送信、レスポンス受信)

```rust
use shiguredo_http3::{ClientConnection, Header, Settings};

// クライアント接続を生成
let mut conn = ClientConnection::new(Settings::default());

// H3 ストリームを初期化 (制御、QPACK エンコーダー、QPACK デコーダーの順)
let init = conn.init_h3_streams(2, 6, 10)?;
// init.control_data / init.encoder_data / init.decoder_data を各ストリームで送信

// リクエストを送信
let stream_id = conn.send_request(&[
    Header::new(b":method", b"GET")?,
    Header::new(b":path", b"/")?,
    Header::new(b":scheme", b"https")?,
    Header::new(b":authority", b"example.com")?,
], true)?;

// 送信データを取得して QUIC で送信
// while let Some((data, fin)) = conn.take_stream_data(stream_id) { ... }

// QUIC から受信したデータを入力してイベントを処理
// conn.feed_stream(stream_id, &response_data, fin)?;
// while let Some(event) = conn.poll_event()? { ... }
```

### サーバー (リクエスト受信、レスポンス送信)

```rust
use shiguredo_http3::{Event, Header, ServerConnection, Settings};

// サーバー接続を生成
let mut conn = ServerConnection::new(Settings::default());

// H3 ストリームを初期化 (制御、QPACK エンコーダー、QPACK デコーダーの順)
let init = conn.init_h3_streams(3, 7, 11)?;
// init.control_data / init.encoder_data / init.decoder_data を各ストリームで送信

// QUIC から受信したデータを入力
// conn.feed_stream(stream_id, &request_data, fin)?;

// イベントを処理してレスポンスを送信
while let Some(event) = conn.poll_event()? {
    match event {
        Event::HeadersEnd { stream_id } => {
            conn.send_response(stream_id, &[
                Header::new(b":status", b"200")?,
            ], false)?;
            conn.send_body(stream_id, b"Hello, HTTP/3!", true)?;
        }
        _ => {}
    }
}
```

### QPACK エンコード/デコード

```rust
use shiguredo_http3::qpack::{Decoder, Encoder, Header};

// エンコード
let mut encoder = Encoder::new();
let headers = vec![
    Header::new(b":method", b"GET")?,
    Header::new(b":path", b"/")?,
];
let mut buf = vec![0u8; 1024];
let encoded_len = encoder.encode(&mut buf, &headers, 0)?;

// デコード
let decoder = Decoder::new();
let decoded = decoder.decode(&buf[..encoded_len])?;
```

### WebTransport over HTTP/3

```rust
use shiguredo_http3::webtransport::{Capsule, Datagram, Session};

// セッションを生成 (session_id は CONNECT ストリーム ID)
let mut session = Session::new(0);
session.set_connecting();
session.set_established();

// データグラムをエンコード
let datagram = Datagram::new(0, b"hello".to_vec())?;
let mut buf = Vec::new();
datagram.encode(&mut buf);

// カプセルを送信キューに追加してグレースフルシャットダウン
session.queue_capsule(Capsule::MaxData { maximum: 2_000_000 });
session.drain();
```

## HTTP/3

このライブラリが対応している HTTP/3 の機能です。

### フレームタイプ (RFC 9114 Section 7.2)

- DATA (0x00) - データ転送
- HEADERS (0x01) - ヘッダー (QPACK 圧縮)
- SETTINGS (0x04) - 接続設定
- GOAWAY (0x07) - 接続終了

### SETTINGS パラメータ (RFC 9114 Section 7.2.4.1)

- SETTINGS_QPACK_MAX_TABLE_CAPACITY (0x01)
- SETTINGS_MAX_FIELD_SECTION_SIZE (0x06)
- SETTINGS_QPACK_BLOCKED_STREAMS (0x07)
- SETTINGS_ENABLE_CONNECT_PROTOCOL (0x08)
- SETTINGS_H3_DATAGRAM (0x33)
- 予約 ID のフィルタリング (0x1f * N + 0x21)
- HTTP/2 専用 ID の検出と拒否 (0x02-0x05)

### ストリーム

- 制御ストリーム (0x00)
- QPACK エンコーダーストリーム (0x02)
- QPACK デコーダーストリーム (0x03)
- リクエスト/レスポンスストリーム

### エラーコード (RFC 9114 Section 8.1 / RFC 9297 Section 5.2)

- H3_NO_ERROR (0x100)
- H3_GENERAL_PROTOCOL_ERROR (0x101)
- H3_INTERNAL_ERROR (0x102)
- H3_STREAM_CREATION_ERROR (0x103)
- H3_CLOSED_CRITICAL_STREAM (0x104)
- H3_FRAME_UNEXPECTED (0x105)
- H3_FRAME_ERROR (0x106)
- H3_EXCESSIVE_LOAD (0x107)
- H3_ID_ERROR (0x108)
- H3_SETTINGS_ERROR (0x109)
- H3_MISSING_SETTINGS (0x10a)
- H3_REQUEST_REJECTED (0x10b)
- H3_REQUEST_CANCELLED (0x10c)
- H3_REQUEST_INCOMPLETE (0x10d)
- H3_MESSAGE_ERROR (0x10e)
- H3_DATAGRAM_ERROR (0x33)
- QPACK_DECOMPRESSION_FAILED (0x200)
- QPACK_ENCODER_STREAM_ERROR (0x201)
- QPACK_DECODER_STREAM_ERROR (0x202)

### イベント

- `SettingsReceived` - SETTINGS フレーム受信
- `HeadersBegin` - ヘッダー受信開始
- `Header` - 個別ヘッダー受信
- `HeadersEnd` - ヘッダー受信完了
- `Data` - データ受信
- `StreamEnd` - ストリーム終了
- `StreamReset` - ストリームリセット (RESET_STREAM 受信)
- `StopSending` - 送信停止要求 (STOP_SENDING 受信)
- `GoawayReceived` - GOAWAY 受信
- `WebTransport` - WebTransport 関連イベント (`WebTransportEvent` にネスト)

接続エラーはイベントとしては通知されず、`poll_event` の `Err` として返ります。

### 未実装機能

- サーバープッシュ (CANCEL_PUSH / PUSH_PROMISE / MAX_PUSH_ID)
  - 受信時にエラーを返す

## QPACK (RFC 9204)

このライブラリが対応している QPACK の機能です。

### ヘッダー圧縮

- 静的テーブル (99 エントリ)
- 動的テーブル
- Huffman エンコーディング
- 整数エンコーディング (プレフィックス付き)

`Connection` が送信に使う QPACK エンコーダーは静的テーブルのみを使用し、動的テーブルへの挿入は行いません。動的テーブルは受信側 (`Decoder` / `DynamicDecoder`) と公開 API (`DynamicEncoder`) で利用できます。

### エンコーダーストリーム命令

- Set Dynamic Table Capacity
- Insert With Name Reference (静的 / 動的テーブル)
- Insert With Literal Name
- Duplicate

### デコーダーストリーム命令

- Section Acknowledgment
- Stream Cancellation
- Insert Count Increment

### フィールド行表現

- Indexed Field Line (静的 / 動的テーブル)
- Literal Field Line with Name Reference
- Literal Field Line with Literal Name
- Indexed Field Line with Post-Base Index

## WebTransport over HTTP/3

draft-ietf-webtrans-http3 の draft-02 / 07 / 14 / 15 / 16 / 17 に対応しています。ピアの SETTINGS から接続単位で版を選択します。

WebTransport を有効にするには、`Settings::enable_webtransport_server` または `enable_webtransport_client` に `webtransport::Settings` を渡します。

| draft | 備考 |
|-------|------|
| draft-02 | `SETTINGS_ENABLE_WEBTRANSPORT` / `:protocol = webtransport` |
| draft-07 | `SETTINGS_WEBTRANSPORT_MAX_SESSIONS` |
| draft-14 | `SETTINGS_WT_MAX_SESSIONS` / カプセルベースのフロー制御 |
| draft-15 | `SETTINGS_WT_ENABLED` (`0x2c7cf000`) / `:protocol = webtransport-h3` |
| draft-16 | draft-15 と同じコードポイント |
| draft-17 | `SETTINGS_WT_ENABLED` (`0x20b8933b`) / 本文版 |

本文版は draft-17 の仕様本文で定義された版です。draft-15 / 16 の `0x2c7cf000` とは別のコードポイントを使うため、SETTINGS で区別できます。

draft ごとの差分とバージョンネゴシエーションの詳細は [`docs/WT_HTTP3.md`](docs/WT_HTTP3.md) を参照してください。

### Capsule Protocol

- WT_CLOSE_SESSION (0x2843)
- WT_DRAIN_SESSION (0x78ae)
- WT_MAX_DATA (0x190B4D3D)
- WT_MAX_STREAMS (双方向, 0x190B4D3F)
- WT_MAX_STREAMS (単方向, 0x190B4D40)
- WT_DATA_BLOCKED (0x190B4D41)
- WT_STREAMS_BLOCKED (双方向, 0x190B4D43)
- WT_STREAMS_BLOCKED (単方向, 0x190B4D44)

### HTTP Datagram

- Quarter Stream ID のエンコード/デコード (RFC 9297)
- QUIC DATAGRAM フレームペイロードの処理

## 制限 (DoS 対策)

デフォルト値:

- 最大ヘッダーセクションサイズ: 16 KB
- QPACK 最大テーブル容量: 4096 バイト
- QPACK ブロックストリーム数: 100
- 受信ボディの累積上限: 16 MiB (超えた場合は `H3_EXCESSIVE_LOAD` で接続を閉じる)

`Limits` で各制限値を変更できます。受信ボディの上限は
`ClientConnection::set_max_received_body_size` /
`ServerConnection::set_max_received_body_size` でも変更できます。

## クレート構成

このリポジトリには以下のクレートが含まれています。

### shiguredo_http3

Sans I/O な HTTP/3 ライブラリ本体です。依存ライブラリ 0 で RFC 9114 および RFC 9204 に準拠しています。

### tokio-s2n-quic

shiguredo_http3 (Sans I/O) と [s2n-quic](https://github.com/aws/s2n-quic) (AWS) を組み合わせた HTTP/3 / WebTransport の [Tokio](https://github.com/tokio-rs/tokio) 統合です。

TLS には [Rustls](https://github.com/rustls/rustls) を、暗号ライブラリには [aws-lc-rs](https://github.com/aws/aws-lc-rs) を使用しています。

### tokio-ngtcp2

shiguredo_http3 (Sans I/O) と [shiguredo_ngtcp2_tokio](https://crates.io/crates/shiguredo_ngtcp2_tokio) (ngtcp2 の tokio 統合) を組み合わせた HTTP/3 / WebTransport の Tokio 統合です。相互運用性テスト用です。

QUIC は crates.io の `shiguredo_ngtcp2_tokio` を、HTTP/3 は shiguredo_http3 を使用します。

## サンプル

### wt_server

WebTransport のエコーサーバーです。workspace メンバーなので、リポジトリルートから `-p wt_server` で実行できます。

```console
cargo run -p wt_server -- --listen 127.0.0.1:4443
```

## 規格書

このライブラリが参照している RFC と Internet-Draft の一覧です。未対応の規定は本文に注記しています。

- RFC 9000 - QUIC: A UDP-Based Multiplexed and Secure Transport
  - <https://datatracker.ietf.org/doc/html/rfc9000>
  - 可変長整数エンコーディングのみ
- RFC 9114 - HTTP/3
  - <https://datatracker.ietf.org/doc/html/rfc9114>
- RFC 9204 - QPACK: Field Compression for HTTP/3
  - <https://datatracker.ietf.org/doc/html/rfc9204>
- RFC 9297 - HTTP Datagrams and the Capsule Protocol
  - <https://datatracker.ietf.org/doc/html/rfc9297>
  - Quarter Stream ID エンコーディング
- RFC 8441 - Bootstrapping WebSockets with HTTP/2
  - <https://datatracker.ietf.org/doc/html/rfc8441>
  - 拡張 CONNECT の定義
- RFC 9220 - Bootstrapping WebSockets with HTTP/3
  - <https://datatracker.ietf.org/doc/html/rfc9220>
  - HTTP/3 における拡張 CONNECT の適用
- draft-ietf-webtrans-overview-13 - WebTransport Protocol Framework
  - <https://datatracker.ietf.org/doc/html/draft-ietf-webtrans-overview-13>
- draft-ietf-webtrans-http3-17 - WebTransport over HTTP/3
  - <https://datatracker.ietf.org/doc/html/draft-ietf-webtrans-http3-17>

## ライセンス

Apache License 2.0

```text
Copyright 2026-2026, Shiguredo Inc.

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
```
