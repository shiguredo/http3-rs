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
  - 任意の QUIC 実装と組み合わせ可能
- RFC 9114 (HTTP/3) および RFC 9204 (QPACK) 準拠
- WebTransport over HTTP/3 (draft-ietf-webtrans-http3-15) サポート

## 使い方

### クライアント (リクエスト送信、レスポンス受信)

```rust
use shiguredo_http3::{ClientConnection, Header, Event, Settings};

// クライアント接続を作成
let mut conn = ClientConnection::new(Settings::default());

// H3 ストリームを初期化 (制御 + QPACK encoder/decoder)
// ストリーム ID は QUIC から割り当てられた単方向ストリーム
let init = conn.init_h3_streams(2, 6, 10).unwrap();
// init.control_data / init.encoder_data / init.decoder_data を各ストリームで送信

// リクエストを送信
let stream_id = conn.send_request(&[
    Header::new(b":method", b"GET"),
    Header::new(b":path", b"/"),
    Header::new(b":scheme", b"https"),
    Header::new(b":authority", b"example.com"),
], true).unwrap();

// 送信データを取得して QUIC で送信 (FIN はデータ消費後の追加呼び出しで交付される)
loop {
    if let Some((data, fin)) = conn.get_stream_data(stream_id) {
        // quic.send_stream_data(stream_id, &data, fin);
        conn.consume_stream_data(stream_id, data.len());
        if fin {
            break;
        }
    } else {
        break;
    }
}

// QUIC からデータを受信
// conn.feed_stream(stream_id, &response_data, fin).unwrap();

// イベントを処理
while let Some(event) = conn.poll_event().unwrap() {
    match event {
        Event::HeadersEnd { stream_id } => {
            println!("Headers received on stream {}", stream_id);
        }
        Event::Data { stream_id, data } => {
            println!("Data: {} bytes", data.len());
        }
        Event::StreamEnd { stream_id } => {
            println!("Stream {} finished", stream_id);
        }
        _ => {}
    }
}
```

### サーバー (リクエスト受信、レスポンス送信)

```rust
use shiguredo_http3::{ServerConnection, Header, Event, Settings};

// サーバー接続を作成
let mut conn = ServerConnection::new(Settings::default());

// H3 ストリームを初期化 (制御 + QPACK encoder/decoder)
let init = conn.init_h3_streams(3, 7, 11).unwrap();
// init.control_data / init.encoder_data / init.decoder_data を各ストリームで送信

// QUIC からデータを受信
// conn.feed_stream(stream_id, &request_data, fin).unwrap();

// イベントを処理
while let Some(event) = conn.poll_event().unwrap() {
    match event {
        Event::HeadersEnd { stream_id } => {
            // リクエストヘッダー受信完了
            // レスポンスを送信
            conn.send_response(stream_id, &[
                Header::new(b":status", b"200"),
                Header::new(b"content-type", b"text/plain"),
            ], false).unwrap();
            conn.send_body(stream_id, b"Hello, World!", true).unwrap();
        }
        _ => {}
    }
}

// 送信データを取得して QUIC で送信 (FIN はデータ消費後の追加呼び出しで交付される)
loop {
    if let Some((data, fin)) = conn.get_stream_data(stream_id) {
        // quic.send_stream_data(stream_id, &data, fin);
        conn.consume_stream_data(stream_id, data.len());
        if fin {
            break;
        }
    } else {
        break;
    }
}
```

### QPACK エンコード/デコード

```rust
use shiguredo_http3::qpack::{Encoder, Decoder, Header};

// エンコード
let encoder = Encoder::new();
let headers = vec![
    Header::new(b":method", b"GET"),
    Header::new(b":path", b"/"),
    Header::new(b":scheme", b"https"),
    Header::new(b":authority", b"example.com"),
];

let mut buf = vec![0u8; 1024];
let encoded_len = encoder.encode(&mut buf, &headers).unwrap();

// デコード
let decoder = Decoder::new();
let decoded = decoder.decode(&buf[..encoded_len]).unwrap();
```

### WebTransport over HTTP/3

```rust
use shiguredo_http3::webtransport::{
    Session, SessionState, Capsule, Stream, StreamHeader, Datagram,
    FlowControlLimits,
};

// セッションを作成 (session_id = CONNECT ストリーム ID)
let mut session = Session::new(0);
assert_eq!(session.state(), SessionState::Pending);

// セッション状態を遷移
session.set_connecting();
assert_eq!(session.state(), SessionState::Connecting);
session.set_established();
assert_eq!(session.state(), SessionState::Established);

// ローカルのフロー制御リミットを初期化
session.initialize_local_limits(FlowControlLimits {
    max_streams_uni: 10,
    max_streams_bidi: 10,
    max_data: 1_000_000,
});

// ストリームを追加
let stream = Stream::new(4, 0, true); // stream_id, session_id, bidirectional
session.add_stream(stream);

// ストリームヘッダーのエンコード/デコード
let header = StreamHeader::new(0); // session_id
let mut buf = Vec::new();
// 単方向ストリーム: Stream Type (0x54) + Session ID
header.encode_unidirectional(&mut buf);
// 双方向ストリーム: Signal Value (0x41) + Session ID
header.encode_bidirectional(&mut buf);

// Datagram のエンコード/デコード (Quarter Stream ID = session_id / 4)
let datagram = Datagram::new(0, b"hello".to_vec()).unwrap();
let mut buf = Vec::new();
datagram.encode(&mut buf);
let (decoded, consumed) = Datagram::decode(&buf).unwrap();

// Capsule を送信キューに追加
session.queue_capsule(Capsule::MaxData { maximum: 2_000_000 });

// 送信待ち Capsule を取り出す
let capsules = session.take_pending_capsules();

// グレースフルシャットダウン
session.drain();
assert_eq!(session.state(), SessionState::Draining);

// エラーでクローズ (WT_CLOSE_SESSION Capsule を生成)
// session.close_with_error(0, "bye");
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

### エラーコード (RFC 9114 Section 8.1)

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
- H3_CONNECT_ERROR (0x10f)
- H3_VERSION_FALLBACK (0x110)
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
- `ConnectionError` - 接続エラー

### 未実装機能

以下の機能は実装していません:

- サーバープッシュ (CANCEL_PUSH, PUSH_PROMISE, MAX_PUSH_ID)
  - 受信時にエラーを返す

## QPACK (RFC 9204)

このライブラリが対応している QPACK の機能です。

### ヘッダー圧縮

- 静的テーブル (99 エントリ)
- 動的テーブル
- Huffman エンコーディング
- 整数エンコーディング (プレフィックス付き)

### エンコーダーストリーム命令

- Set Dynamic Table Capacity
- Insert With Name Reference (静的/動的テーブル)
- Insert With Literal Name
- Duplicate

### デコーダーストリーム命令

- Section Acknowledgment
- Stream Cancellation
- Insert Count Increment

### フィールド行表現

- Indexed Field Line (静的/動的テーブル)
- Literal Field Line with Name Reference
- Literal Field Line with Literal Name
- Indexed Field Line with Post-Base Index

## WebTransport over HTTP/3

draft-ietf-webtrans-http3 の複数バージョンをサポートし、ピアの SETTINGS から自動的に draft を判定して応答します。

### 対応 draft バージョン

| draft | ステータス | 備考 |
|-------|-----------|------|
| draft-02 | 対応 | `SETTINGS_ENABLE_WEBTRANSPORT` / `:protocol = webtransport` |
| draft-07 | 対応 | `SETTINGS_WEBTRANSPORT_MAX_SESSIONS` / セッションゴーン / ドレイン |
| draft-14 | 対応 | `SETTINGS_WT_MAX_SESSIONS` / カプセルベースフロー制御 |
| draft-15 | 対応 | `SETTINGS_WT_ENABLED` / `:protocol = webtransport-h3` / ALPN |
| draft-16 | 対応 | 単調性チェック厳密化 / エラー種別変更 / `SETTINGS_WT_ENABLED > 1` 検証 |

詳細な差分は [`docs/WT_HTTP3.md`](docs/WT_HTTP3.md) を参照してください。

### draft 毎の主要な挙動の違い

| 項目 | draft-02 | draft-07 | draft-15 |
|------|----------|----------|----------|
| `:protocol` 値 | `webtransport` | `webtransport` | `webtransport-h3` |
| SETTINGS パラメータ | `SETTINGS_ENABLE_WEBTRANSPORT` (`0x2b603742`) | `SETTINGS_WEBTRANSPORT_MAX_SESSIONS` (`0xc671706a`) | `SETTINGS_WT_ENABLED` (`0x2c7cf000`) |
| バージョンネゴシエーション | `Sec-Webtransport-Http3-Draft02` ヘッダー | SETTINGS コードポイント | SETTINGS コードポイント + ALPN ヘッダー |
| アプリケーションエラーコード幅 | 8 bit | 32 bit | 32 bit |
| `SETTINGS_ENABLE_CONNECT_PROTOCOL` | 暗黙 (不要) | サーバーのみ必須 | サーバーのみ必須 |
| `SETTINGS_H3_DATAGRAM` | 不要 | 明示的に必須 | 明示的に必須 |
| QUIC `max_datagram_frame_size` | 不要 | 明示的に必須 | 明示的に必須 |
| `reset_stream_at` transport parameter | 不要 | 不要 | 必須 |
| データグラム形式 | Datagram Format Type `WEB_TRANSPORT` (`0xff7c00`) | Quarter Stream ID (RFC 9297) | Quarter Stream ID (RFC 9297) |
| セッション終了カプセル | `CLOSE_WEBTRANSPORT_SESSION` | `CLOSE_WEBTRANSPORT_SESSION` | `WT_CLOSE_SESSION` |
| ドレインカプセル | なし | `DRAIN_WEBTRANSPORT_SESSION` | `WT_DRAIN_SESSION` |
| セッション終了時のエラーコード | 指定なし | `WEBTRANSPORT_SESSION_GONE` (`0x170d7b68`) | `WT_SESSION_GONE` (`0x170d7b68`) |
| セッションレベルフロー制御 | なし | なし | カプセルベース (`WT_MAX_DATA` / `WT_MAX_STREAMS`) |
| ALPN ネゴシエーション | なし | なし | `WT-Available-Protocols` / `WT-Protocol` |
| TLS エクスポーター | なし | なし | `EXPORTER-WebTransport` |
| 優先度制御 | なし | なし | RFC 9218 推奨 |
| `RESET_STREAM_AT` の使用 | 不要 | 不要 | 必須 (ストリームヘッダー確実配信のため) |

draft-02 / draft-07 ではセッションレベルフロー制御が存在しないため、同時セッションは原則 1 つに制限されます。draft-15 ではカプセルベースのフロー制御 (`WT_MAX_DATA`, `WT_MAX_STREAMS`, `WT_DATA_BLOCKED`, `WT_STREAMS_BLOCKED`) によって複数セッションを扱えます。

### CONNECT リクエスト/レスポンス

- 拡張 CONNECT (RFC 8441, RFC 9220) によるセッション確立
- ドラフトバージョンネゴシエーション (draft-02/07/14/15)
- `:protocol` 疑似ヘッダー (`webtransport` / `webtransport-h3`)
- トランスポートケイパビリティ検証 (SETTINGS / QUIC パラメータ)
- CONNECT リクエスト/レスポンスバリデーション

### ストリーム

- 双方向ストリーム (WT_STREAM フレーム, シグナル値 0x41)
- 単方向ストリーム (ストリームタイプ 0x54)
- ストリームヘッダーのエンコード/デコード
- 単方向ストリーム分類 (WebTransport / HTTP/3 の判定)

### Capsule Protocol

- WT_CLOSE_SESSION (0x2843)
- WT_DRAIN_SESSION (0x78ae)
- WT_MAX_DATA (0x190B4D3D)
- WT_MAX_STREAMS_BIDI (0x190B4D3F)
- WT_MAX_STREAMS_UNI (0x190B4D40)
- WT_DATA_BLOCKED (0x190B4D41)
- WT_STREAMS_BLOCKED_BIDI (0x190B4D43)
- WT_STREAMS_BLOCKED_UNI (0x190B4D44)

### HTTP Datagram

- Quarter Stream ID エンコード/デコード (RFC 9297)
- QUIC DATAGRAM フレームペイロードの処理

### フロー制御

- セッションレベルのデータフロー制御
- ストリーム数の制限
- ブロック通知

### セッション管理

- セッション状態: Pending → Connecting → Established → Draining → Closed
- セッション確立前のストリーム/データグラムバッファリング
- グレースフルシャットダウン (WT_DRAIN_SESSION)

### エラーコード

- BUFFERED_STREAM_REJECTED (0x3994bd84)
- SESSION_GONE (0x170d7b68)
- FLOW_CONTROL_ERROR (0x045d4487)
- ALPN_ERROR (0x0817b3dd)
- REQUIREMENTS_NOT_MET (0x212c0d48)
- アプリケーションエラーコード変換 (0x00000000-0xffffffff)

### SETTINGS

- SETTINGS_ENABLE_WEBTRANSPORT (0x2b603742) - draft-02
- SETTINGS_WEBTRANSPORT_MAX_SESSIONS (0xc671706a) - draft-07
- SETTINGS_WT_MAX_SESSIONS (0x14e9cd29) - draft-14
- SETTINGS_WT_ENABLED (0x2c7cf000) - draft-15
- SETTINGS_WT_INITIAL_MAX_STREAMS_UNI (0x2b64) - draft-15
- SETTINGS_WT_INITIAL_MAX_STREAMS_BIDI (0x2b65) - draft-15
- SETTINGS_WT_INITIAL_MAX_DATA (0x2b61) - draft-15

## 制限 (DoS 対策)

デフォルト値:

- 最大ヘッダーセクションサイズ: 16KB
- QPACK 最大テーブル容量: 4096 バイト
- QPACK ブロックストリーム数: 100

`Limits` で各制限値をカスタマイズ可能です。

## クレート構成

このリポジトリには以下のクレートが含まれています。

### shiguredo_http3

Sans I/O な HTTP/3 ライブラリ本体です。依存ライブラリ 0 で RFC 9114 および RFC 9204 に準拠しています。

### tokio-s2n-quic

shiguredo_http3 (Sans I/O) と [s2n-quic](https://github.com/aws/s2n-quic) (AWS) を組み合わせた HTTP/3 / WebTransport の [Tokio](https://github.com/tokio-rs/tokio) 統合です。

TLS には [Rustls](https://github.com/rustls/rustls) を、暗号ライブラリには [aws-lc-rs](https://github.com/aws/aws-lc-rs) を使用しています。

### ngtcp2-sys

[ngtcp2](https://github.com/ngtcp2/ngtcp2) C ライブラリへの低レベル FFI バインディングです。WebTransport 対応のため [webtransport ブランチ](https://github.com/ngtcp2/ngtcp2/tree/webtransport)を使用しています。リリースされたらタグに切り替える予定です。

### nghttp3-sys

[nghttp3](https://github.com/ngtcp2/nghttp3) C ライブラリへの低レベル FFI バインディングです。WebTransport 対応のため [webtransport ブランチ](https://github.com/ngtcp2/nghttp3/tree/webtransport)を使用しています。リリースされたらタグに切り替える予定です。

### shiguredo_ngtcp2

ngtcp2/nghttp3 の Rust バインディングです。ngtcp2-sys/nghttp3-sys の上に安全な Rust API を提供します。

TLS には [aws-lc-sys](https://github.com/aws/aws-lc-rs) (BoringSSL 互換) を使用しています。

### tokio-ngtcp2

shiguredo_ngtcp2 を [Tokio](https://github.com/tokio-rs/tokio) と統合し、非同期 HTTP/3 クライアント/サーバーを提供します。

tokio-s2n-quic との差異は、QUIC および HTTP/3 プロトコル処理に ngtcp2/nghttp3 を使用している点です。相互運用性テスト用です。

## 規格書

このライブラリが準拠している RFC 一覧です。

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
- draft-ietf-webtrans-overview-12 - WebTransport Protocol Framework
  - <https://datatracker.ietf.org/doc/html/draft-ietf-webtrans-overview-12>
- draft-ietf-webtrans-http3-15 - WebTransport over HTTP/3
  - <https://datatracker.ietf.org/doc/html/draft-ietf-webtrans-http3-15>

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
