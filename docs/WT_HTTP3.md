# WebTransport over HTTP/3: draft 02 / 07 / 15 の差分

このドキュメントは `draft-ietf-webtrans-http3` の draft 02, 07, 15 の主要な差分をまとめたものである。

## 参照ドキュメント

- `refs/webtrans/draft-ietf-webtrans-http3-02.txt` (2021-10-25)
- `refs/webtrans/draft-ietf-webtrans-http3-07.txt` (2023-06-13)
- `refs/webtrans/draft-ietf-webtrans-http3-15.txt` (2026-03-02)

## 総括

| 項目 | draft-02 | draft-07 | draft-15 |
|------|----------|----------|----------|
| `:protocol` 値 | `webtransport` | `webtransport` | `webtransport-h3` |
| SETTINGS パラメータ名 | `SETTINGS_ENABLE_WEBTRANSPORT` | `SETTINGS_WEBTRANSPORT_MAX_SESSIONS` | `SETTINGS_WT_ENABLED` |
| SETTINGS コードポイント | `0x2b603742` | `0xc671706a` | `0x2c7cf000` |
| アプリエラーコード幅 | 8 bit | 32 bit | 32 bit |
| セッション終了カプセル名 | `CLOSE_WEBTRANSPORT_SESSION` | `CLOSE_WEBTRANSPORT_SESSION` | `WT_CLOSE_SESSION` |
| ドレインカプセル | なし | `DRAIN_WEBTRANSPORT_SESSION` | `WT_DRAIN_SESSION` |
| セッションレベルフロー制御 | なし | なし | あり (カプセルベース) |
| `SETTINGS_ENABLE_CONNECT_PROTOCOL` | 暗黙 (不要) | サーバーのみ必須 | サーバーのみ必須 |
| `SETTINGS_H3_DATAGRAM` | 不要 | 明示的に必須 | 明示的に必須 |
| QUIC `max_datagram_frame_size` | 不要 | 明示的に必須 | 明示的に必須 |
| `reset_stream_at` | 不要 | 不要 | 必須 |
| ALPN ネゴシエーション | なし | なし | あり (`WT-Available-Protocols` / `WT-Protocol`) |
| TLS エクスポーター | なし | なし | あり (`EXPORTER-WebTransport`) |
| 優先度制御 | なし | なし | RFC 9218 推奨 |

---

## 1. セッション確立

### 1.1 SETTINGS ネゴシエーション

**draft-02**:
- 双方が `SETTINGS_ENABLE_WEBTRANSPORT` (`0x2b603742`) を値 `1` で送信する
- 値が 0 でも 1 でもない場合は `H3_SETTINGS_ERROR`
- この設定が Extended CONNECT のサポートを暗黙に示す (別途 `SETTINGS_ENABLE_CONNECT_PROTOCOL` は不要)

**draft-07**:
- `SETTINGS_WEBTRANSPORT_MAX_SESSIONS` (`0xc671706a`) に変更。値 > 0 でサポートを表明
- サーバーは同時セッション上限を示す。クライアントは > 0 であればよい
- `SETTINGS_ENABLE_CONNECT_PROTOCOL` = 1 がサーバーに明示的に必須
- `SETTINGS_H3_DATAGRAM` = 1 が双方に明示的に必須
- QUIC `max_datagram_frame_size` > 0 が双方に明示的に必須

**draft-15**:
- `SETTINGS_WT_ENABLED` (`0x2c7cf000`) に変更。値 > 0 でサポートを表明
- draft-07 の前提条件に加え、`reset_stream_at` トランスポートパラメータが双方に必須
- `SETTINGS_ENABLE_CONNECT_PROTOCOL` はサーバーのみ送信義務がある (RFC 9220, RFC 8441 Section 3)。クライアントの送信リストには含まれない (Section 3.1)
- セッションレベルフロー制御用の SETTINGS を追加:
  - `SETTINGS_WT_INITIAL_MAX_STREAMS_UNI` (`0x2b64`)
  - `SETTINGS_WT_INITIAL_MAX_STREAMS_BIDI` (`0x2b65`)
  - `SETTINGS_WT_INITIAL_MAX_DATA` (`0x2b61`)

### 1.2 Extended CONNECT

**draft-02**:
- `:protocol` = `webtransport`
- バージョンネゴシエーション用ヘッダー: `Sec-Webtransport-Http3-Draft02: 1`
- サーバー応答: `Sec-Webtransport-Http3-Draft: draft02`

**draft-07**:
- `:protocol` = `webtransport` (変更なし)
- バージョンネゴシエーションは `SETTINGS_WEBTRANSPORT_MAX_SESSIONS` のコードポイント変更で実施
- ヘッダーベースのバージョンネゴシエーションは廃止

**draft-15**:
- `:protocol` = `webtransport-h3` に変更 (Capsule ベース方式の `webtransport` と明確に分離)
- アプリケーションプロトコルネゴシエーション追加:
  - `WT-Available-Protocols` ヘッダー (クライアント → サーバー, Structured Fields List)
  - `WT-Protocol` ヘッダー (サーバー → クライアント, Structured Fields Item)
  - 失敗時は `WT_ALPN_ERROR` でセッションを閉じる

### 1.3 0-RTT

**全 draft 共通**:
- CONNECT は safe メソッドではないため、0-RTT パケットで WebTransport を開始できない

**draft-07 以降**:
- SETTINGS パラメータは前回セッションから保持可能
- サーバーが 0-RTT を受け入れる場合、最大セッション数を前回より減らしてはならない

**draft-15**:
- フロー制御値も 0-RTT 受け入れ時に減らしてはならない

---

## 2. ストリーム

### 2.1 単方向ストリーム

**全 draft 共通**:
- HTTP/3 ストリームタイプ `0x54`
- フォーマット: `Stream Type (0x54) | Session ID | Stream Body`

### 2.2 双方向ストリーム

**全 draft 共通**:
- シグナル値 (フレームタイプ) `0x41` (`WEBTRANSPORT_STREAM` / `WT_STREAM`)
- フォーマット: `Signal Value (0x41) | Session ID | Stream Body`
- 長さフィールドを持たない (正式な HTTP/3 フレームではない)

**draft-02**:
- `WEBTRANSPORT_STREAM` はフレームとして定義
- リクエストストリーム上で使用

**draft-07 以降**:
- ストリームの最初のバイトでのみ送信可能。それ以外の場所で受信した場合は `H3_FRAME_ERROR`

### 2.3 ストリームリセットとエラーコードマッピング

**draft-02**:
- アプリケーションエラーコードは **8 bit** (0x00 - 0xFF)
- HTTP/3 エラーコード範囲: `0x52e4a40fa8db` - `0x52e4a40fa9e2`

**draft-07 以降**:
- アプリケーションエラーコードは **32 bit** (0x00000000 - 0xFFFFFFFF) に拡張
- HTTP/3 エラーコード範囲: `0x52e4a40fa8db` - `0x52e5ac983162`
- マッピング時 `0x1f * N + 0x21` 形式の GREASE コードポイントをスキップ

**draft-15 のみ**:
- `RESET_STREAM_AT` フレームの使用が必須
- ストリームヘッダー (Session ID 等) の確実な配信のため、Reliable Size をストリームヘッダーサイズ以上に設定する
- 範囲外のエラーコードを受信した場合、ストリームはリセットされるがアプリケーションエラーコードなしとして扱う

---

## 3. データグラム

**draft-02**:
- HTTP Datagrams (`draft-ietf-masque-h3-datagram`) を使用
- Datagram Format Type: `WEB_TRANSPORT` (`0xff7c00`)
- データグラム登録カプセルで `Datagram Format Type` を設定する方式

**draft-07 以降**:
- HTTP Datagrams (RFC 9297) を使用
- Quarter Stream ID フィールドでセッションを識別する方式に変更 (Datagram Format Type は廃止)
- WebTransport ペイロードは HTTP Datagram Payload フィールドにそのまま格納

---

## 4. カプセルプロトコル

### 4.1 セッション終了カプセル

**全 draft 共通** (名称は異なる):
- コードポイント: `0x2843`
- フォーマット: `Application Error Code (32 bit) | Application Error Message (UTF-8, 最大 1024 バイト)`
- 送信後に FIN を送信しなければならない (MUST)
- カプセル後の追加データは `H3_MESSAGE_ERROR` でリセット

| draft | カプセル名 |
|-------|-----------|
| draft-02 | `CLOSE_WEBTRANSPORT_SESSION` |
| draft-07 | `CLOSE_WEBTRANSPORT_SESSION` |
| draft-15 | `WT_CLOSE_SESSION` |

### 4.2 ドレインカプセル

**draft-02**: なし

**draft-07 以降**:
- コードポイント: `0x78ae`
- 長さ 0
- HTTP/3 GOAWAY の単一セッション版
- 受信後もセッション使用は継続可能だが、可能な限り早くグレースフルに終了すべき (SHOULD)

| draft | カプセル名 |
|-------|-----------|
| draft-07 | `DRAIN_WEBTRANSPORT_SESSION` |
| draft-15 | `WT_DRAIN_SESSION` |

### 4.3 フロー制御カプセル (draft-15 のみ)

draft-15 で追加されたセッションレベルフロー制御のためのカプセル:

| カプセル | コードポイント | 用途 |
|---------|---------------|------|
| `WT_MAX_DATA` | `0x190B4D3D` | セッション全体のデータ量上限 |
| `WT_MAX_STREAMS` (bidi) | `0x190B4D3F` | 双方向ストリーム累積上限 |
| `WT_MAX_STREAMS` (uni) | `0x190B4D40` | 単方向ストリーム累積上限 |
| `WT_DATA_BLOCKED` | `0x190B4D41` | データ送信がフロー制御でブロック |
| `WT_STREAMS_BLOCKED` (bidi) | `0x190B4D43` | 双方向ストリームのブロック通知 |
| `WT_STREAMS_BLOCKED` (uni) | `0x190B4D44` | 単方向ストリームのブロック通知 |

- 前回の値より小さい値の `WT_MAX_STREAMS` / `WT_MAX_DATA` を受信した場合は `WT_FLOW_CONTROL_ERROR`
- `WT_MAX_STREAMS` の Maximum Streams が 2^60 を超えた場合は `H3_DATAGRAM_ERROR`
- 全フロー制御カプセルは hop-by-hop (中継者が消費して自身の信号を生成)

### 4.4 禁止カプセル (draft-15 のみ)

HTTP/3 版では QUIC ネイティブのストリームレベルフロー制御を使うため、以下のカプセルは禁止:

| カプセル | コードポイント |
|---------|---------------|
| `WT_MAX_STREAM_DATA` | `0x190B4D3E` |
| `WT_STREAM_DATA_BLOCKED` | `0x190B4D42` |

受信した場合はセッションエラーとして処理する。

---

## 5. セッション終了

### 5.1 終了条件

**全 draft 共通**:
1. CONNECT ストリームがクリーンまたは異常に閉じられた場合
2. セッション終了カプセルが送信または受信された場合

### 5.2 終了時の処理

**draft-02**:
- 関連する全ストリームをリセットする (MUST)
- エラーコードの指定なし

**draft-07 以降**:
- 関連する全ストリームの送信側をリセットし、受信側の読み取りを中断する (MUST)
- エラーコードとして `WEBTRANSPORT_SESSION_GONE` (`0x170d7b68`) / `WT_SESSION_GONE` を使用

### 5.3 QUIC CONNECTION_CLOSE との連携

**draft-07 以降**:
- セッション終了カプセル送信後に即座に QUIC CONNECTION_CLOSE を送るとピアがカプセルを受信する前に接続が閉じる可能性がある
- CONNECT ストリームのデータが ACK されるまで待ってから CONNECTION_CLOSE を送るべき (SHOULD)

---

## 6. エラーコード

### 6.1 プロトコルエラーコード

| エラーコード | コードポイント | draft-02 | draft-07 | draft-15 |
|-------------|---------------|----------|----------|----------|
| `H3_WEBTRANSPORT_BUFFERED_STREAM_REJECTED` / `WT_BUFFERED_STREAM_REJECTED` | `0x3994bd84` | あり | あり | あり |
| `WEBTRANSPORT_SESSION_GONE` / `WT_SESSION_GONE` | `0x170d7b68` | なし | あり | あり |
| `WT_FLOW_CONTROL_ERROR` | `0x045d4487` | なし | なし | あり |
| `WT_ALPN_ERROR` | `0x0817b3dd` | なし | なし | あり |
| `WT_REQUIREMENTS_NOT_MET` | `0x212c0d48` | なし | なし | あり |

### 6.2 HTTP/3 既存エラーコードの使用

| エラーコード | 用途 | 適用 draft |
|-------------|------|-----------|
| `H3_ID_ERROR` | 不正なセッション ID | 全 draft |
| `H3_FRAME_ERROR` | `WEBTRANSPORT_STREAM` / `WT_STREAM` の不正な位置での受信 | draft-07 以降 |
| `H3_MESSAGE_ERROR` | セッション終了カプセル後の追加データ受信 | 全 draft |
| `H3_SETTINGS_ERROR` | SETTINGS 値の不正 / 0-RTT でのフロー制御値減少 | 全 draft |
| `H3_REQUEST_REJECTED` / `HTTP_REQUEST_REJECTED` | セッション数超過時のリクエスト拒否 | 全 draft |
| `H3_DATAGRAM_ERROR` | `WT_MAX_STREAMS` の値超過 | draft-15 |
| `H3_EXCESSIVE_LOAD` | 不審な使用パターン | draft-15 |

---

## 7. フロー制御 (draft-15 のみ)

draft-15 ではセッションレベルのフロー制御が追加された。これは QUIC のコネクションレベルフロー制御と類似の仕組みをセッション単位で提供する。

### 7.1 有効化条件

- 両エンドポイントが `SETTINGS_WT_INITIAL_MAX_STREAMS_UNI`, `SETTINGS_WT_INITIAL_MAX_STREAMS_BIDI`, `SETTINGS_WT_INITIAL_MAX_DATA` のうち少なくとも1つを非ゼロ値で送信した場合に有効
- フロー制御が無効の場合、同時セッションは 1 つに制限される

### 7.2 制御の種類

| 制御対象 | 初期値の SETTINGS | 動的更新カプセル | ブロック通知カプセル |
|---------|------------------|-----------------|-------------------|
| 単方向ストリーム数 | `SETTINGS_WT_INITIAL_MAX_STREAMS_UNI` | `WT_MAX_STREAMS` (uni) | `WT_STREAMS_BLOCKED` (uni) |
| 双方向ストリーム数 | `SETTINGS_WT_INITIAL_MAX_STREAMS_BIDI` | `WT_MAX_STREAMS` (bidi) | `WT_STREAMS_BLOCKED` (bidi) |
| データ量 | `SETTINGS_WT_INITIAL_MAX_DATA` | `WT_MAX_DATA` | `WT_DATA_BLOCKED` |

### 7.3 ストリームレベルフロー制御

- HTTP/3 版では QUIC ネイティブのストリームレベルフロー制御を使用する
- `WT_MAX_STREAM_DATA` / `WT_STREAM_DATA_BLOCKED` カプセルは HTTP/3 版では禁止

---

## 8. draft-15 のみの新機能

### 8.1 RESET_STREAM_AT の必須化

`reset_stream_at` トランスポートパラメータが前提条件に追加された。ストリームリセット時にストリームヘッダー (Session ID) の確実な配信を保証するため、Reliable Size をヘッダーサイズ以上に設定する必要がある。

参照: `draft-ietf-quic-reliable-stream-reset-07`

### 8.2 TLS キーイングマテリアルエクスポーター

セッションごとの TLS エクスポーターの導出メカニズム:
- ラベル: `EXPORTER-WebTransport`
- コンテキスト: WebTransport Exporter Context 構造体 (Session ID 64 bit + アプリケーション提供のラベルとコンテキスト)

### 8.3 優先度制御

RFC 9218 の Extensible Priorities を WebTransport セッション全体 (ストリーム、データグラム、カプセル含む) に適用することが推奨される。セッション内の優先度シグナリングはアプリケーションプロトコルに委ねられる。

### 8.4 WT_REQUIREMENTS_NOT_MET エラーコード

WebTransport の前提条件 (SETTINGS / トランスポートパラメータ) が満たされない場合に接続を閉じるための専用エラーコード (`0x212c0d48`)。

---

## 9. IANA 登録コードポイントの変遷

### HTTP/3 SETTINGS

| SETTINGS | コードポイント | draft |
|----------|---------------|-------|
| `SETTINGS_ENABLE_WEBTRANSPORT` | `0x2b603742` | draft-02 |
| `SETTINGS_WEBTRANSPORT_MAX_SESSIONS` | `0xc671706a` | draft-07 |
| `SETTINGS_WT_ENABLED` | `0x2c7cf000` | draft-15 |
| `SETTINGS_WT_INITIAL_MAX_STREAMS_UNI` | `0x2b64` | draft-15 |
| `SETTINGS_WT_INITIAL_MAX_STREAMS_BIDI` | `0x2b65` | draft-15 |
| `SETTINGS_WT_INITIAL_MAX_DATA` | `0x2b61` | draft-15 |

### HTTP/3 フレームタイプ

| フレーム | コードポイント | draft |
|---------|---------------|-------|
| `WEBTRANSPORT_STREAM` / `WT_STREAM` | `0x41` | 全 draft |

### HTTP/3 ストリームタイプ

| ストリーム | コードポイント | draft |
|-----------|---------------|-------|
| WebTransport 単方向ストリーム | `0x54` | 全 draft |

### カプセルタイプ

| カプセル | コードポイント | draft |
|---------|---------------|-------|
| `CLOSE_WEBTRANSPORT_SESSION` / `WT_CLOSE_SESSION` | `0x2843` | 全 draft |
| `DRAIN_WEBTRANSPORT_SESSION` / `WT_DRAIN_SESSION` | `0x78ae` | draft-07 以降 |
| `WT_MAX_DATA` | `0x190B4D3D` | draft-15 |
| `WT_MAX_STREAM_DATA` (禁止) | `0x190B4D3E` | draft-15 |
| `WT_MAX_STREAMS` (bidi) | `0x190B4D3F` | draft-15 |
| `WT_MAX_STREAMS` (uni) | `0x190B4D40` | draft-15 |
| `WT_DATA_BLOCKED` | `0x190B4D41` | draft-15 |
| `WT_STREAM_DATA_BLOCKED` (禁止) | `0x190B4D42` | draft-15 |
| `WT_STREAMS_BLOCKED` (bidi) | `0x190B4D43` | draft-15 |
| `WT_STREAMS_BLOCKED` (uni) | `0x190B4D44` | draft-15 |

### HTTP/3 エラーコード

| エラーコード | コードポイント | draft |
|-------------|---------------|-------|
| `WT_BUFFERED_STREAM_REJECTED` | `0x3994bd84` | 全 draft |
| `WT_SESSION_GONE` | `0x170d7b68` | draft-07 以降 |
| `WT_FLOW_CONTROL_ERROR` | `0x045d4487` | draft-15 |
| `WT_ALPN_ERROR` | `0x0817b3dd` | draft-15 |
| `WT_REQUIREMENTS_NOT_MET` | `0x212c0d48` | draft-15 |
| `WT_APPLICATION_ERROR` 範囲 | `0x52e4a40fa8db` - `0x52e5ac983162` | draft-07 以降 (draft-02 は 8 bit 範囲) |

---

## 10. SETTINGS_ENABLE_CONNECT_PROTOCOL の送信義務

`SETTINGS_ENABLE_CONNECT_PROTOCOL` (0x08) は RFC 8441 Section 3 で定義されたサーバーからクライアントへの設定であり、クライアントに送信義務はない。

> Upon receipt of SETTINGS_ENABLE_CONNECT_PROTOCOL with a value of 1, a client MAY use the Extended CONNECT as defined in this document when creating new streams. Receipt of this parameter by a server does not have any impact.
> --- RFC 8441 Section 3

RFC 9220 も同様に「a new HTTP/2 setting sent by a server to allow the client to use Extended CONNECT」と定義している。

draft-ietf-webtrans-http3-15 Section 3.1 では、サーバーとクライアントの送信リストを明確に分離している:

**サーバーが送信するもの:**
- `SETTINGS_WT_ENABLED` (値 > 0)
- `SETTINGS_ENABLE_CONNECT_PROTOCOL` (値 1)
- `SETTINGS_H3_DATAGRAM` (値 1)
- `max_datagram_frame_size` transport parameter (値 > 0)
- `reset_stream_at` transport parameter (空)

**クライアントが送信するもの:**
- `SETTINGS_H3_DATAGRAM` (値 1)
- `max_datagram_frame_size` transport parameter (値 > 0)
- `reset_stream_at` transport parameter (空)
- ドラフト版のみ: `SETTINGS_WT_ENABLED` (ドラフト固有コードポイント)

クライアントの送信リストに `SETTINGS_ENABLE_CONNECT_PROTOCOL` は含まれない。サーバーはクライアントが `SETTINGS_ENABLE_CONNECT_PROTOCOL` を送信しなくても WebTransport CONNECT リクエストを受理しなければならない。

nghttp3 (ngtcp2) は RFC に準拠し、クライアント側では `enable_connect_protocol = 0` に強制設定して SETTINGS フレームに含めない (`nghttp3_conn.c` line 420)。
