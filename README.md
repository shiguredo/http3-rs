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
- WebTransport over HTTP/3 (draft-ietf-webtrans-http3 の draft-02 / 07 / 14 / 15 / 16 / 17) サポート

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
    Header::new(b":method", b"GET").unwrap(),
    Header::new(b":path", b"/").unwrap(),
    Header::new(b":scheme", b"https").unwrap(),
    Header::new(b":authority", b"example.com").unwrap(),
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
                Header::new(b":status", b"200").unwrap(),
                Header::new(b"content-type", b"text/plain").unwrap(),
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
let mut encoder = Encoder::new();
let headers = vec![
    Header::new(b":method", b"GET").unwrap(),
    Header::new(b":path", b"/").unwrap(),
    Header::new(b":scheme", b"https").unwrap(),
    Header::new(b":authority", b"example.com").unwrap(),
];

let mut buf = vec![0u8; 1024];
let encoded_len = encoder.encode(&mut buf, &headers, 0).unwrap();

// デコード
let decoder = Decoder::new();
let decoded = decoder.decode(&buf[..encoded_len]).unwrap();
assert_eq!(decoded[0].name(), b":method");
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

// ピアの SETTINGS から得た送信側フロー制御リミットを初期化する
// (セッション確立前に 1 回だけ呼ぶ。以後の WT_MAX_STREAMS / WT_MAX_DATA の
//  単調性検証の基準にもなる)
session.initialize_remote_limits(FlowControlLimits {
    max_streams_uni: 10,
    max_streams_bidi: 10,
    max_data: 1_000_000,
});

// ストリームを追加
let stream = Stream::new(4, 0, true); // stream_id, session_id, bidirectional
session.add_stream(stream);

// ストリームヘッダーのエンコード/デコード
let header = StreamHeader::new(0).unwrap(); // session_id
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
assert_eq!(decoded.session_id, 0);
assert_eq!(decoded.payload, b"hello");
assert_eq!(consumed, buf.len());

// Capsule を送信キューに追加
session.queue_capsule(Capsule::MaxData { maximum: 2_000_000 });

// 送信待ち Capsule を取り出す
let capsules = session.take_pending_capsules();
assert_eq!(capsules.len(), 1);

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

`Connection` が送信に使う QPACK エンコーダーは静的テーブルのみを使用し、動的テーブルへの
挿入は行いません。動的テーブルは受信側 (`Decoder` / `DynamicDecoder`) および公開 API
(`DynamicEncoder`) で利用できます。

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

draft-ietf-webtrans-http3 の複数バージョンをサポートします。ピアの SETTINGS から接続単位で版を選択し、選択した版に合わせて CONNECT とストリームを処理します。

### 対応 draft バージョン

| draft | ステータス | 備考 |
|-------|-----------|------|
| draft-02 | 対応 | `SETTINGS_ENABLE_WEBTRANSPORT` / `:protocol = webtransport` |
| draft-07 | 対応 | `SETTINGS_WEBTRANSPORT_MAX_SESSIONS` / セッションゴーン / ドレイン |
| draft-14 | 対応 | `SETTINGS_WT_MAX_SESSIONS` / カプセルベースフロー制御 |
| draft-15 | 対応 | `SETTINGS_WT_ENABLED` (`0x2c7cf000`) / `:protocol = webtransport-h3` / ALPN |
| draft-16 | 対応 | 単調性チェック厳密化 / エラー種別変更 / `SETTINGS_WT_ENABLED > 1` 検証 |
| draft-17 | 対応 | 本文版 / `SETTINGS_WT_ENABLED` (`0x20b8933b`) / 前回接続の WT SETTINGS を破棄。部分配送付きリセットのアプリケーション指定オフセットは未対応 |

draft-17 の仕様は、その本文で定義された版 (以下、本文版) と、それ以前の draft で定義された版を `SETTINGS_WT_ENABLED` のコードポイントで区別します。draft-17 の時点では RFC 番号が確定していないため、本 README ではこの版を draft-17 の本文版と呼びます。本文版は `0x20b8933b` を使い、draft-15 / 16 の `0x2c7cf000` とは別のコードポイントで識別します。

SETTINGS コードポイントが同一のため、draft-15 と draft-16 は SETTINGS だけでは区別できません。`SETTINGS_WT_ENABLED > 1` をクライアントが受信した場合の拒否は draft-16 / 17 の規則です。draft-15 と draft-16 はコードポイントを共有するため、本実装は draft-15 を選んだ接続にもこの規則を適用します。フロー制御カプセルの単調性判定はドラフト分岐を持たず、draft-14 以降のすべての版に適用します。

詳細な差分は [`docs/WT_HTTP3.md`](docs/WT_HTTP3.md) を参照してください。

### バージョンネゴシエーション

接続で使う版は、SETTINGS のコードポイントを交わして接続単位で決めます。端点は対応する版のコードポイントを 1 回ずつ広告し、`0x2c7cf000` (draft-15 / 16) と `0x20b8933b` (draft-17 の本文版) を区別します。

版の選択は、draft-17 の本文版、draft-15 / 16、draft-07、draft-14、draft-02 の優先順位で行います。この順位は「数値の大きい版を選ぶ」規則に対する例外を含みます。draft-07 と draft-14 のコードポイントが併送された場合、本実装は Safari 26.4 (Network.framework) との互換のため draft-07 を優先します。この例外は Safari 以外の端点が同じ組み合わせを広告した場合にも適用されます。詳細は [`docs/SAFARI_WT.md`](docs/SAFARI_WT.md) を参照してください。

- サーバーは、クライアントが送る `SETTINGS_WT_ENABLED` を通常は無視します。旧 draft にも対応するサーバーは、クライアントが広告したコードポイントを版の選択に使います
- サーバーは、クライアントが `0x20b8933b` を広告していれば本文版を選びます。広告していなければ、共通する旧 draft のうち優先順位の高い版を選びます。共通する旧 draft がなく、サーバーが本文版に対応していれば本文版を選びます
- 本文版だけに対応するクライアントは `SETTINGS_WT_ENABLED` を送りません。サーバーが `0x20b8933b` を広告していれば、クライアントは本文版を選びます。サーバーは `SETTINGS_WT_ENABLED` を送らないクライアントを本文版として扱います
- 本文版と旧 draft の両方に対応するクライアントは、対応するコードポイントをそれぞれ広告します。`0x20b8933b` を送った場合、サーバーは本文版を選びます。`0x20b8933b` を送らない場合、サーバーは共通する旧 draft のうち優先順位の高い版を選びます
- 旧 draft だけに対応するクライアントは、旧 draft のコードポイントだけを広告します。本文版に対応するサーバーは、共通する旧 draft があればそのうち優先順位の高い版を選びます。共通する旧 draft がなければ本文版を選びますが、そのクライアントは本文版に対応しないためセッションを確立できません
- クライアントは、ローカルとサーバーが共通して広告する版のうち優先順位の高い版を選びます。共通する版がなく、ローカルが版を 1 つも広告せず、サーバーが `0x20b8933b` を広告している場合は本文版を選びます。それ以外で共通する版がない場合、使用する版がないため WebTransport を開始しません
- サーバーは、クライアントの SETTINGS を受信するまで WebTransport リクエストの処理を保留します。本文版に対応せず、`SETTINGS_WT_ENABLED` を広告していながら共通する版がない場合、サーバーはクライアントの CONNECT を拒否します

### 設定方法

`webtransport::Settings::for_draft_versions` は、渡した版の SETTINGS コードポイントを広告する設定を組み立てます。

- `DraftVersion::Draft17` を渡すと `SETTINGS_WT_ENABLED` (`0x20b8933b`) を送ります。サーバーが本文版に対応することを示す場合は、この設定が必要です
- 本文版だけに対応するクライアントは `SETTINGS_WT_ENABLED` を送りません。`for_draft_versions` に `DraftVersion::Draft17` を渡さず、版を 1 つも含まない `webtransport::Settings::new()` を `enable_webtransport_client` に渡します。この場合も、サーバーが `0x20b8933b` を広告していれば本文版が選ばれます
- 本文版と旧 draft の両方に対応するクライアントは、対応する旧 draft のコードポイントと `DraftVersion::Draft17` の両方を含めます
- フロー制御を有効にする場合は、`wt_initial_max_streams_uni` などのビルダーメソッドで初期値を設定します。`for_draft_versions` はこれらの初期値を設定しません

```rust
use shiguredo_http3::Settings;
use shiguredo_http3::webtransport::{DraftVersion, Settings as WtSettings};

// 本文版に対応するサーバー: SETTINGS_WT_ENABLED (0x20b8933b) を広告する
let wt = WtSettings::for_draft_versions(
    &[DraftVersion::Draft17],
    shiguredo_http3::VarInt::from_static(1),
);
let server = Settings::default().enable_webtransport_server(wt);

// 本文版だけに対応するクライアント: SETTINGS_WT_ENABLED を送らない
let wt = WtSettings::new();
let client = Settings::default().enable_webtransport_client(wt);
```

### draft 毎の主要な挙動の違い

#### プロトコルとバージョン

| draft | `:protocol` 値 | アプリケーションエラーコード幅 | SETTINGS パラメータ (コードポイント) |
|-------|----------------|--------------------------------|--------------------------------------|
| draft-02 | `webtransport` | 8 bit | `SETTINGS_ENABLE_WEBTRANSPORT` (`0x2b603742`) |
| draft-07 | `webtransport` | 32 bit | `SETTINGS_WEBTRANSPORT_MAX_SESSIONS` (`0xc671706a`) |
| draft-14 | `webtransport` | 32 bit | `SETTINGS_WT_MAX_SESSIONS` (`0x14e9cd29`) |
| draft-15 / draft-16 | `webtransport-h3` | 32 bit | `SETTINGS_WT_ENABLED` (`0x2c7cf000`) |
| draft-17 | `webtransport-h3` | 32 bit | `SETTINGS_WT_ENABLED` (`0x20b8933b`) |

バージョンネゴシエーションは、draft-02 が `Sec-Webtransport-Http3-Draft02` ヘッダー、draft-07 以降が SETTINGS コードポイントです。draft-14 以降は ALPN (`WT-Available-Protocols` / `WT-Protocol`) も併用します。draft-17 は本文版へのフォールバックを含みます。

#### 前提条件

| draft | `SETTINGS_ENABLE_CONNECT_PROTOCOL` | `SETTINGS_H3_DATAGRAM` | QUIC `max_datagram_frame_size` | `reset_stream_at` transport parameter |
|-------|-------------------------------------|------------------------|--------------------------------|---------------------------------------|
| draft-02 | 暗黙 (不要) | 不要 | 不要 | 不要 |
| draft-07 | サーバーのみ必須 | 明示的に必須 | 明示的に必須 | 不要 |
| draft-14 | サーバーのみ必須 | 明示的に必須 | 明示的に必須 | 必須 |
| draft-15 / draft-16 | サーバーのみ必須 | 明示的に必須 | 明示的に必須 | 必須 |
| draft-17 | サーバーのみ必須 | 明示的に必須 | 明示的に必須 | 必須 |

#### セッション機能

| draft | セッションレベルフロー制御 | ALPN ネゴシエーション | TLS エクスポーター | 優先度制御 |
|-------|----------------------------|-----------------------|--------------------|------------|
| draft-02 | なし | なし | なし | なし |
| draft-07 | なし | なし | なし | なし |
| draft-14 | カプセルベース (`WT_MAX_DATA` / `WT_MAX_STREAMS`) | `WT-Available-Protocols` / `WT-Protocol` | `EXPORTER-WebTransport` | RFC 9218 推奨 |
| draft-15 / draft-16 | カプセルベース (`WT_MAX_DATA` / `WT_MAX_STREAMS`) | `WT-Available-Protocols` / `WT-Protocol` | `EXPORTER-WebTransport` | RFC 9218 推奨 |
| draft-17 | カプセルベース (`WT_MAX_DATA` / `WT_MAX_STREAMS`) | `WT-Available-Protocols` / `WT-Protocol` | `EXPORTER-WebTransport` | RFC 9218 推奨 |

#### 終了とデータグラム

| draft | データグラム形式 | セッション終了カプセル | ドレインカプセル | セッション終了時のエラーコード |
|-------|------------------|------------------------|------------------|--------------------------------|
| draft-02 | Datagram Format Type `WEB_TRANSPORT` (`0xff7c00`) | `CLOSE_WEBTRANSPORT_SESSION` | なし | 指定なし |
| draft-07 | Quarter Stream ID (RFC 9297) | `CLOSE_WEBTRANSPORT_SESSION` | `DRAIN_WEBTRANSPORT_SESSION` | `WEBTRANSPORT_SESSION_GONE` (`0x170d7b68`) |
| draft-14 | Quarter Stream ID (RFC 9297) | `WT_CLOSE_SESSION` | `WT_DRAIN_SESSION` | `WT_SESSION_GONE` (`0x170d7b68`) |
| draft-15 / draft-16 | Quarter Stream ID (RFC 9297) | `WT_CLOSE_SESSION` | `WT_DRAIN_SESSION` | `WT_SESSION_GONE` (`0x170d7b68`) |
| draft-17 | Quarter Stream ID (RFC 9297) | `WT_CLOSE_SESSION` | `WT_DRAIN_SESSION` | `WT_SESSION_GONE` (`0x170d7b68`) |

#### 0-RTT とリセット

| draft | 0-RTT 再開時の WT SETTINGS | 不正な位置の `WT_STREAM` | `RESET_STREAM_AT` の使用 |
|-------|----------------------------|--------------------------|---------------------------|
| draft-02 | 0-RTT 非対応 | — | 不要 |
| draft-07 | 最大セッション数を減らせない | `H3_FRAME_ERROR` | 不要 |
| draft-14 | サーバーは最大セッション数と初期フロー制御値を減らせない。減少は `H3_SETTINGS_ERROR` | `H3_FRAME_ERROR` | 必須 (ストリームヘッダー確実配信のため) |
| draft-15 | 同上 (サーバー側の義務) | `H3_FRAME_ERROR` | 必須 (ストリームヘッダー確実配信のため) |
| draft-16 | サーバーは初期フロー制御値を減らせない。クライアントも減少検出時に `H3_SETTINGS_ERROR` で接続を閉じる | `H3_FRAME_ERROR` | 必須 (ストリームヘッダー確実配信のため) |
| draft-17 | 前回接続の値を破棄し、現在値だけを使う | `H3_FRAME_UNEXPECTED` | 必須 (ストリームヘッダーとアプリケーション指定オフセットの確実配信のため) |

draft-02 / draft-07 ではセッションレベルフロー制御が存在しないため、同時セッションは原則 1 つに制限されます。draft-14 以降ではカプセルベースのフロー制御 (`WT_MAX_DATA`, `WT_MAX_STREAMS`, `WT_DATA_BLOCKED`, `WT_STREAMS_BLOCKED`) によって複数セッションを扱えます。

sans-I/O 層では、ピアの SETTINGS 受信後・セッション確立前に `webtransport::Session::initialize_remote_limits` を 1 回呼ぶことで、ピアが宣言した初期上限 (`SETTINGS_WT_INITIAL_MAX_*`) を送信側フロー制御と `WT_MAX_STREAMS` / `WT_MAX_DATA` の単調性検証の基準に反映できます。

不正な位置の `WT_STREAM` について、旧 draft の仕様値は `H3_FRAME_ERROR` ですが、本実装は全版で `H3_FRAME_UNEXPECTED` を返します。

### 0-RTT 再開

CONNECT は safe メソッドではないため、0-RTT パケットで WebTransport セッションを開始できません。この制約はすべての版で共通です。

- draft-07 では、サーバーが 0-RTT を受け入れる場合、最大セッション数を前回より減らしてはなりません
- draft-14 / 15 では、サーバーが 0-RTT を受け入れる場合、最大セッション数と初期フロー制御値 (`SETTINGS_WT_INITIAL_MAX_*`) を前回より減らしてはなりません。減少は互換でない変更とみなされ、`H3_SETTINGS_ERROR` 接続エラーになります。この義務はサーバー側にあります
- draft-16 では、サーバーが 0-RTT を受け入れる場合に初期フロー制御値を前回より減らせないことに加え、クライアントは再開接続の SETTINGS がキャッシュしたフロー制御値を減少させていたら `H3_SETTINGS_ERROR` で接続を閉じなければなりません。接続を閉じる義務がクライアントに明示されたのは draft-16 からです
- draft-17 の本文版では、クライアントは前回接続から保持していた WT SETTINGS の値を破棄し、現在接続で受信した値だけを使います。仕様上、サーバーは前回送信した値に関係なく 0-RTT を受理できます。本実装は、接続で draft-17 を選択した場合に前回値と比較しません
- draft-02 は 0-RTT に対応しません。draft-07 の最大セッション数の減少検出は未対応です

本実装は、クライアント側で draft-14 / 15 / 16 を選択した接続の減少を検出し、`H3_SETTINGS_ERROR` で接続を閉じます。draft-14 / 15 の仕様上の義務はサーバー側ですが、クライアントは受信した SETTINGS から同じ違反を検出できます。版を決定できない場合も、ローカルが draft-14 / 15 に対応して draft-17 に対応せず、ピアも draft-17 を広告しないときに限り、同じ減少検出を行います。

### 接続共有時の制約 (draft-17 Section 8)

複数の WebTransport セッションに 1 本の QUIC 接続を共有させる端点は、WebTransport のフロー制御を実装しなければなりません。この MUST は draft-16 で既に規定されています (Section 5.1 / Section 8)。draft-17 で追加されたのは、フロー制御を無効にした場合の停滞とデッドロックの説明です。

フロー制御を有効にせずにセッションを接続へ相乗りさせると、セッションは下位 QUIC 接続のストリーム数とデータ量の上限まで使えます。同じ接続の HTTP 応答と QPACK ストリームも接続レベルのフロー制御を共有するため、次のようなデッドロックが起こり得ます。

- アプリケーションが WebTransport の読み取りを、同じ接続上の別の HTTP 応答の完了まで待つ
- その HTTP 応答が、WebTransport セッションが使い切った接続レベルのフロー制御のために進まない
- 双方が相手の進行を待ち続ける

回避策は、WebTransport のフロー制御を有効にしてセッションごとの上限を設けるか、WebTransport セッションを別の接続へ分離することです。本ライブラリは SETTINGS とカプセルによるフロー制御を提供しますが、有効にするかどうかはアプリケーションと各アダプターの設定に依存します。

### 先行ストリームとデータグラムの拒否 (draft-17 Section 4.6)

クライアントは 2xx 応答を待たずに、WebTransport のストリームやデータグラムを送れます。サーバーも 2xx 応答を送る前からストリームやデータグラムを送れます。これらは CONNECT の応答より先に到着し得るため、端点はセッションへ関連付けられるまでバッファリングします。バッファリングは必須ではなく、上限を超えた場合は仕様上 `WT_BUFFERED_STREAM_REJECTED` でストリームを閉じ、データグラムを破棄します。

本実装の sans-I/O 層は、上限を超えたストリームを `WebTransportEvent::BufferedStreamRejected` で通知するだけで、`RESET_STREAM` / `STOP_SENDING` を送出しません。通知を受けた利用側 (アダプターまたはアプリケーション) が、イベントの `error_code` を使ってリセットを送る責務を負います。tokio-s2n-quic の `H3Server` は、このイベントを受けて `RESET_STREAM` と `STOP_SENDING` を送ります。WebTransport のセッション API はイベントをアプリケーションへ転送するため、アプリケーションがリセットを送ります。

ストリームの拒否は、そのストリームを送ろうとしたセッションを終了させません。アプリケーションは拒否されたデータを、同じセッションの新しいストリームで送り直せます。本実装は拒否とセッション終了を分けて扱いますが、再送は自動では行いません。アプリケーションが新しいストリームを開いて再送してください。

### draft-15 と draft-16 の差分

SETTINGS コードポイントと `:protocol` は同一です。draft-16 で変わった検証とエラー処理は次のとおりです。

| 項目 | draft-15 | draft-16 |
|------|----------|----------|
| `SETTINGS_WT_ENABLED` の値 | 値 > 0 でサポート表明 | 値は 1。クライアントは > 1 を `H3_SETTINGS_ERROR` として扱う |
| `WT_MAX_DATA` / `WT_MAX_STREAMS` の単調性 | 前回より小さい値はエラー | 増加しない値 (同値を含む) は `WT_FLOW_CONTROL_ERROR` |
| `WT_MAX_STREAMS` > 2^60 | `H3_DATAGRAM_ERROR` (接続エラー) | `WT_FLOW_CONTROL_ERROR` (セッションエラー) |
| `WT_STREAMS_BLOCKED` > 2^60 | 検証なし | `WT_FLOW_CONTROL_ERROR` (セッションエラー) |
| 非対応リソースへの応答 | 404 を推奨 | 405 を推奨 |
| 0-RTT 再開時のフロー制御値減少 | サーバー側の義務 | クライアントも `H3_SETTINGS_ERROR` で接続を閉じる |
| 楽観的カプセル送信 | 規定なし | クライアントは 2xx 受信前にカプセルを送ってよい。サーバーは 2xx 送信まで処理しない |

本実装は、フロー制御カプセルの単調性判定 (増加しない値はエラー) と `WT_MAX_STREAMS > 2^60` のエラー種別を draft-14 以降のすべての版に適用します。ドラフト分岐はありません。`SETTINGS_WT_ENABLED > 1` をクライアントが受信した場合の拒否は draft-16 / 17 の規則で、本実装は draft-15 を選んだ接続にも適用します。そのため、draft-15 の仕様上のコードである `H3_DATAGRAM_ERROR` は使用せず、`WT_MAX_STREAMS > 2^60` も `WT_FLOW_CONTROL_ERROR` (セッションエラー) として扱います。

### draft-16 と draft-17 の差分

| 項目 | draft-16 | draft-17 |
|------|----------|----------|
| `SETTINGS_WT_ENABLED` のコードポイント | `0x2c7cf000` | `0x20b8933b` |
| 不正な位置の `WT_STREAM` | `H3_FRAME_ERROR` | `H3_FRAME_UNEXPECTED` |
| 0-RTT 再開時の WT SETTINGS | 前回値の減少を `H3_SETTINGS_ERROR` とする | 前回値を破棄し、現在値だけを使う |
| 部分配送付きリセット | Reliable Size をストリームヘッダー長以上にする | Reliable Size にアプリケーション指定オフセットを含める |
| 接続共有時のフロー制御 | セッション共有時のフロー制御実装 MUST と公平性 | MUST に加え、フロー制御無効時の HTTP / QPACK の停滞とデッドロックを明記 |
| 先行ストリームの拒否 | 上限超過時の拒否を規定 | 拒否がセッションに影響しないことと、新ストリームでの再送を明記 |

部分配送付きリセットのアプリケーション指定オフセットは未対応です。下位 QUIC API が任意の Reliable Size を受け取れないため、現状の sans-I/O 層はストリームヘッダー長のみを Reliable Size として通知します。詳細は [`docs/WT_HTTP3.md`](docs/WT_HTTP3.md) を参照してください。

### CONNECT リクエスト/レスポンス

- 拡張 CONNECT (RFC 8441, RFC 9220) によるセッション確立
- ドラフトバージョンネゴシエーション (draft-02 / 07 / 14 / 15 / 16 / 17)
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
- SETTINGS_WT_ENABLED (0x2c7cf000) - draft-15 / draft-16
- SETTINGS_WT_ENABLED (0x20b8933b) - draft-17 の本文版
- SETTINGS_WT_INITIAL_MAX_STREAMS_UNI (0x2b64) - draft-14 以降
- SETTINGS_WT_INITIAL_MAX_STREAMS_BIDI (0x2b65) - draft-14 以降
- SETTINGS_WT_INITIAL_MAX_DATA (0x2b61) - draft-14 以降

## 制限 (DoS 対策)

デフォルト値:

- 最大ヘッダーセクションサイズ: 16KB
- QPACK 最大テーブル容量: 4096 バイト
- QPACK ブロックストリーム数: 100
- 受信ボディの累積上限: 16 MiB (超えた場合は `H3_EXCESSIVE_LOAD` で接続を閉じる)

`Limits` で各制限値をカスタマイズ可能です。受信ボディの上限は
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
