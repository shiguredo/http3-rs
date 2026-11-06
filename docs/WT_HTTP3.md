# WebTransport over HTTP/3: draft-02 / 07 / 14 / 15 / 16 / 17 の差分

本書は `draft-ietf-webtrans-http3` の draft-02 / 07 / 14 / 15 / 16 / 17 の主要な差分をまとめたものである。

draft-17 の仕様は、その本文で定義された版 (本文版) と、それ以前の draft で定義された版を `SETTINGS_WT_ENABLED` のコードポイントで区別する。draft-17 の時点では RFC 番号が確定していないため、本書では draft-17 の本文版と呼ぶ。

## 参照ドキュメント

- `refs/webtrans/draft-ietf-webtrans-http3-02.txt` (2021-10-25)
- `refs/webtrans/draft-ietf-webtrans-http3-07.txt` (2023-06-13)
- `refs/webtrans/draft-ietf-webtrans-http3-14.txt` (2025-10-20)
- `refs/webtrans/draft-ietf-webtrans-http3-15.txt` (2026-03-02)
- `refs/webtrans/draft-ietf-webtrans-http3-16.txt` (2026-07-06)
- `refs/webtrans/draft-ietf-webtrans-http3-17.txt` (2026-11-02)

## 総括

### 基本

| draft | `:protocol` 値 | SETTINGS パラメータ名 | SETTINGS コードポイント | アプリエラーコード幅 |
|-------|----------------|-----------------------|------------------------|----------------------|
| draft-02 | `webtransport` | `SETTINGS_ENABLE_WEBTRANSPORT` | `0x2b603742` | 8 bit |
| draft-07 | `webtransport` | `SETTINGS_WEBTRANSPORT_MAX_SESSIONS` | `0xc671706a` | 32 bit |
| draft-14 | `webtransport` | `SETTINGS_WT_MAX_SESSIONS` | `0x14e9cd29` | 32 bit |
| draft-15 | `webtransport-h3` | `SETTINGS_WT_ENABLED` | `0x2c7cf000` | 32 bit |
| draft-16 | `webtransport-h3` | `SETTINGS_WT_ENABLED` | `0x2c7cf000` | 32 bit |
| draft-17 | `webtransport-h3` | `SETTINGS_WT_ENABLED` | `0x20b8933b` | 32 bit |

### 前提条件

| draft | `SETTINGS_ENABLE_CONNECT_PROTOCOL` | `SETTINGS_H3_DATAGRAM` | QUIC `max_datagram_frame_size` | `reset_stream_at` |
|-------|-------------------------------------|------------------------|--------------------------------|-------------------|
| draft-02 | 暗黙 (不要) | 不要 | 不要 | 不要 |
| draft-07 | サーバーのみ必須 | 明示的に必須 | 明示的に必須 | 不要 |
| draft-14 | サーバーのみ必須 | 明示的に必須 | 明示的に必須 | 必須 |
| draft-15 | サーバーのみ必須 | 明示的に必須 | 明示的に必須 | 必須 |
| draft-16 | サーバーのみ必須 | 明示的に必須 | 明示的に必須 | 必須 |
| draft-17 | サーバーのみ必須 | 明示的に必須 | 明示的に必須 | 必須 |

### 機能

| draft | セッションレベルフロー制御 | ALPN ネゴシエーション | TLS エクスポーター | 優先度制御 |
|-------|----------------------------|-----------------------|--------------------|------------|
| draft-02 | なし | なし | なし | なし |
| draft-07 | なし | なし | なし | なし |
| draft-14 | あり (カプセルベース) | あり (`WT-Available-Protocols` / `WT-Protocol`) | あり (`EXPORTER-WebTransport`) | RFC 9218 推奨 |
| draft-15 | あり (カプセルベース) | あり (`WT-Available-Protocols` / `WT-Protocol`) | あり (`EXPORTER-WebTransport`) | RFC 9218 推奨 |
| draft-16 | あり (カプセルベース) | あり (`WT-Available-Protocols` / `WT-Protocol`) | あり (`EXPORTER-WebTransport`) | RFC 9218 推奨 |
| draft-17 | あり (カプセルベース) | あり (`WT-Available-Protocols` / `WT-Protocol`) | あり (`EXPORTER-WebTransport`) | RFC 9218 推奨 |

### 終了

| draft | セッション終了カプセル名 | ドレインカプセル | セッション終了時のエラーコード |
|-------|--------------------------|------------------|--------------------------------|
| draft-02 | `CLOSE_WEBTRANSPORT_SESSION` | なし | 指定なし |
| draft-07 | `CLOSE_WEBTRANSPORT_SESSION` | `DRAIN_WEBTRANSPORT_SESSION` | `WEBTRANSPORT_SESSION_GONE` (`0x170d7b68`) |
| draft-14 | `WT_CLOSE_SESSION` | `WT_DRAIN_SESSION` | `WT_SESSION_GONE` (`0x170d7b68`) |
| draft-15 | `WT_CLOSE_SESSION` | `WT_DRAIN_SESSION` | `WT_SESSION_GONE` (`0x170d7b68`) |
| draft-16 | `WT_CLOSE_SESSION` | `WT_DRAIN_SESSION` | `WT_SESSION_GONE` (`0x170d7b68`) |
| draft-17 | `WT_CLOSE_SESSION` | `WT_DRAIN_SESSION` | `WT_SESSION_GONE` (`0x170d7b68`) |

### 検証

| draft | `SETTINGS_WT_ENABLED` の値 | フロー制御カプセルの単調性 | `WT_MAX_STREAMS` > 2^60 |
|-------|----------------------------|----------------------------|--------------------------|
| draft-02 | — | — | — |
| draft-07 | — | — | — |
| draft-14 | — | 前回より小さい値はエラー | セッションエラー |
| draft-15 | 値 > 0 | 前回より小さい値はエラー | `H3_DATAGRAM_ERROR` (接続エラー) |
| draft-16 | 値は 1 (`> 1` は `H3_SETTINGS_ERROR`) | 増加しない値 (同値含む) はエラー | `WT_FLOW_CONTROL_ERROR` (セッションエラー) |
| draft-17 | 値は 1 (`> 1` は `H3_SETTINGS_ERROR`) | 増加しない値 (同値含む) はエラー。SETTINGS の初期値を基準に含む | `WT_FLOW_CONTROL_ERROR` (セッションエラー) |

### 0-RTT とリセット

| draft | 0-RTT 再開時の WT SETTINGS | 不正な位置の `WT_STREAM` | `RESET_STREAM_AT` |
|-------|----------------------------|--------------------------|-------------------|
| draft-02 | 0-RTT 非対応 | — | 不要 |
| draft-07 | 最大セッション数を減らせない | `H3_FRAME_ERROR` | 不要 |
| draft-14 | サーバーは最大セッション数と初期フロー制御値を減らせない (サーバー側の義務)。減少は `H3_SETTINGS_ERROR` になる | `H3_FRAME_ERROR` | 必須 |
| draft-15 | 同上 | `H3_FRAME_ERROR` | 必須 |
| draft-16 | サーバーは初期フロー制御値を減らせない。クライアントも減少検出時に `H3_SETTINGS_ERROR` で接続を閉じる | `H3_FRAME_ERROR` | 必須 |
| draft-17 | 前回接続の値を破棄し、現在値だけを使う | `H3_FRAME_UNEXPECTED` | 必須 (アプリケーション指定オフセットを含む) |

> 本実装は、draft-16 で厳密化されたフロー制御カプセルの単調性判定 (増加しない値はエラー) と `WT_MAX_STREAMS > 2^60` のエラー種別を draft-14 以降のすべての版に適用する。フロー制御カプセルの判定にドラフト分岐はない。`SETTINGS_WT_ENABLED > 1` をクライアントが受信した場合の拒否は draft-16 / 17 の規則であり、本実装はコードポイントを共有する draft-15 を選んだ接続にも適用する。
> そのため仕様上の差分である draft-15 の `H3_DATAGRAM_ERROR` は使用せず、draft-15 / 16 / 17 のどれでも `WT_FLOW_CONTROL_ERROR` を返す (本書 Section 11 / 12 参照)。
> draft-07 の 0-RTT 再開時の最大セッション数減少の検出は未対応である (本書 Section 1.3)。

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

**draft-16**:
- SETTINGS コードポイントは draft-15 と同一 (`SETTINGS_WT_ENABLED` = `0x2c7cf000`)
- 値は 1。クライアントは 1 より大きい値を `H3_SETTINGS_ERROR` として扱う (Section 3.1)
- 0-RTT 再開時、クライアントはフロー制御値が前回より減少していたら `H3_SETTINGS_ERROR` で接続を閉じる (Section 3.2)
- クライアントはサーバー応答前にカプセルを楽観的に送信してよい。サーバーは 2xx を送るまで処理してはならない (Section 3.2)

**draft-17**:
- `SETTINGS_WT_ENABLED` のコードポイントを `0x2c7cf000` から `0x20b8933b` に変更 (Section 9.2)
- 値は 1。クライアントは 1 より大きい値を `H3_SETTINGS_ERROR` として扱う (Section 3.1)
- サーバーはクライアントが送信した `SETTINGS_WT_ENABLED` を通常は無視する。draft 版に対応するサーバーは、クライアントが広告したコードポイントを版選択に使う (Section 3.1 / 7.1)
- 複数の版に対応する端点は、版ごとに異なるコードポイントを 1 回ずつ広告する。本文版だけに対応するクライアントは `SETTINGS_WT_ENABLED` を送らない (Section 7.1)
- 0-RTT 再開時は前回接続の WT SETTINGS を破棄し、現在接続の値だけを使う (Section 3.2)
- 初期フロー制御用の SETTINGS (`SETTINGS_WT_INITIAL_MAX_STREAMS_UNI` など) と `SETTINGS_H3_DATAGRAM`、`max_datagram_frame_size`、`reset_stream_at` の扱いは draft-16 と同じ

### 1.2 Extended CONNECT

**draft-02**:
- `:protocol` = `webtransport`
- バージョンネゴシエーション用ヘッダー: `Sec-Webtransport-Http3-Draft02: 1`
- サーバー応答: `Sec-Webtransport-Http3-Draft: draft02`

**draft-07**:
- `:protocol` = `webtransport` (変更なし)
- バージョンネゴシエーションは `SETTINGS_WEBTRANSPORT_MAX_SESSIONS` のコードポイント変更で実施
- ヘッダーベースのバージョンネゴシエーションは廃止

**draft-14**:
- `:protocol` = `webtransport` (変更なし)
- バージョンネゴシエーションは `SETTINGS_WT_MAX_SESSIONS` (`0x14e9cd29`) のコードポイント変更で実施
- アプリケーションプロトコルネゴシエーション追加:
  - `WT-Available-Protocols` ヘッダー (クライアント → サーバー, Structured Fields List)
  - `WT-Protocol` ヘッダー (サーバー → クライアント, Structured Fields Item)
  - 不正な `WT-Protocol` は無視する。`WT_ALPN_ERROR` でセッションを閉じる規則は draft-15 で追加された

**draft-15 / draft-16**:
- `:protocol` = `webtransport-h3` に変更 (Capsule ベース方式の `webtransport` と明確に分離)
- アプリケーションプロトコルネゴシエーションは draft-14 と同様
- SETTINGS コードポイントは draft-15 と draft-16 で同一のため、SETTINGS だけでは区別できない

**draft-17**:
- `:protocol` = `webtransport-h3` (draft-15 / draft-16 と同じ)
- 版は `SETTINGS_WT_ENABLED` のコードポイントで決める。`:protocol` だけでは draft-15 / 16 / 17 を区別できない
- 本文版だけに対応するクライアントは `SETTINGS_WT_ENABLED` を送らないが、`:protocol = webtransport-h3` と本文版の要件で CONNECT する

### 1.3 0-RTT

**全 draft 共通**:
- CONNECT は safe メソッドではないため、0-RTT パケットで WebTransport を開始できない

**draft-07**:
- WebTransport の SETTINGS パラメータは前回セッションから保持できる (RFC 9114 Section 7.2.4.2)
- サーバーが 0-RTT を受け入れる場合、最大セッション数を前回より減らしてはならない

**draft-14 / draft-15**:
- サーバーが 0-RTT を受け入れる場合、最大セッション数と初期フロー制御値 (`SETTINGS_WT_INITIAL_MAX_*`) を前回より減らしてはならない (draft-14 / 15 Section 3.2)
- 減少は互換でない変更とみなされ、`H3_SETTINGS_ERROR` 接続エラーになる。この義務はサーバー側にある

**draft-16**:
- サーバーが 0-RTT を受け入れる場合、初期フロー制御値を前回より減らしてはならない
- クライアントは、再開接続の SETTINGS がキャッシュしたフロー制御値を減少させていたら `H3_SETTINGS_ERROR` で接続を閉じなければならない (draft-16 Section 3.2)。接続を閉じる義務がクライアントに明示されたのは draft-16 からである

**draft-17**:
- クライアントは前回接続から保持していた WT SETTINGS の値を破棄し、現在接続で受信した値だけを使う (draft-17 Section 3.2)
- サーバーは前回送信した値に関係なく 0-RTT を受理できる。RFC 9114 Section 7.2.4.2 の「互換でない、または省略された SETTINGS を接続エラーとする」規則は、WT SETTINGS には適用しない

**実装**:
- 本実装は、クライアント側で draft-14 / 15 / 16 を選択した接続について、初期フロー制御値 (draft-14 では最大セッション数も) の減少を検出し、`H3_SETTINGS_ERROR` で接続を閉じる。draft-14 / 15 の仕様上の義務はサーバー側だが、クライアントは受信した SETTINGS から同じ違反を検出できる
- 版を決定できない場合も、ローカルが draft-14 / 15 に対応して draft-17 に対応せず、ピアも draft-17 を広告しないときに限り、同じように減少を検出する
- draft-17 を選択した接続では、前回値との比較を行わない
- draft-02 は 0-RTT に対応しない。draft-07 の最大セッション数の減少検出は未対応である

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

**draft-07 以降 (draft-16 まで)**:
- ストリームの最初のバイトでのみ送信可能。それ以外の場所で受信した場合は `H3_FRAME_ERROR`

**draft-17**:
- 許可される位置が「双方向ストリームの開始者が最初に送るバイト列」に明確化された (Section 4.3)
- それ以外の場所で受信した場合は `H3_FRAME_UNEXPECTED` (接続エラー) に変更された

**実装**:
- `WT_STREAM` (`0x41`) を不正な位置で受信した場合、draft を問わず `H3_FRAME_UNEXPECTED` を返す。旧 draft の仕様値 `H3_FRAME_ERROR` は使用しない
- サーバーが受信側となるクライアント開始双方向ストリームの先頭は、許可された位置として扱う
- 制御ストリームで SETTINGS を受信する前にフレームタイプ `0x41` を受信した場合は、`H3_MISSING_SETTINGS` を優先する (RFC 9114 Section 6.2.1)。SETTINGS 受信後に制御ストリームでフレームタイプ `0x41` を受信した場合は、`H3_FRAME_UNEXPECTED` になる

### 2.3 ストリームリセットとエラーコードマッピング

**draft-02**:
- アプリケーションエラーコードは **8 bit** (0x00 - 0xFF)
- HTTP/3 エラーコード範囲: `0x52e4a40fa8db` - `0x52e4a40fa9e2`

**draft-07 以降**:
- アプリケーションエラーコードは **32 bit** (0x00000000 - 0xFFFFFFFF) に拡張
- HTTP/3 エラーコード範囲: `0x52e4a40fa8db` - `0x52e5ac983162`
- マッピング時 `0x1f * N + 0x21` 形式の GREASE コードポイントをスキップ

**draft-14 以降**:
- `RESET_STREAM_AT` フレームの使用が必須
- ストリームヘッダー (Session ID 等) の確実な配信のため、Reliable Size をストリームヘッダーサイズ以上に設定する
- 範囲外のエラーコードを受信した場合、ストリームはリセットされるがアプリケーションエラーコードなしとして扱う

**draft-17**:
- `RESET_STREAM_AT` の Reliable Size を、ストリームヘッダーの長さにアプリケーション指定オフセットを加えた値にする (Section 4.4)
- アプリケーションが配送を希望するペイロード先頭部分も、リセット時に確実に配送する
- 中継者が `RESET_STREAM_AT` を転送する場合、hop ごとに WT ヘッダー長が異なることを考慮して Reliable Size を補正する
- `WT_APPLICATION_ERROR` 範囲外のエラーコードは、`RESET_STREAM` / `RESET_STREAM_AT` / `STOP_SENDING` のいずれでもアプリケーションエラーコードに変換しない

**実装状況**:
- WT ストリームのヘッダー長は `Connection::wt_stream_header_len` で取得でき、セッション終了時の `WtStreamReset::reliable_size` に反映する。`RESET_STREAM_AT` フレーム自体の送出は QUIC 層 (統合層) の責務である
- アプリケーション指定オフセットは未対応。下位 QUIC API が任意の Reliable Size / オフセットを受け取れないため
  - `shiguredo_ngtcp2_tokio` (固定依存 `2026.1.0-canary.4`) の `reset_stream_reliable` はストリーム ID とエラーコードだけを受け取り、アプリケーション指定オフセットを指定できない
  - s2n-quic 1.91 は `RESET_STREAM_AT` の送出 API も、ピアの `reset_stream_at` transport parameter を取得する API も公開していない
- 中継機能は本リポジトリにない。中継時の Reliable Size 補正は対象外
- 範囲外のエラーコードをアプリケーションエラーコードへ変換しない動作は実装済み (`ApplicationErrorCode::from_http3_code` が `None` を返し、`webtransport::Error::from_http3_code` が未知コードを `Error::Unknown` として保持する)

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
| draft-14 | `WT_CLOSE_SESSION` |
| draft-15 | `WT_CLOSE_SESSION` |
| draft-16 | `WT_CLOSE_SESSION` |
| draft-17 | `WT_CLOSE_SESSION` |

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
| draft-14 | `WT_DRAIN_SESSION` |
| draft-15 | `WT_DRAIN_SESSION` |
| draft-16 | `WT_DRAIN_SESSION` |
| draft-17 | `WT_DRAIN_SESSION` |

### 4.3 フロー制御カプセル (draft-14 以降)

draft-14 で追加されたセッションレベルフロー制御のためのカプセル:

| カプセル | コードポイント | 用途 |
|---------|---------------|------|
| `WT_MAX_DATA` | `0x190B4D3D` | セッション全体のデータ量上限 |
| `WT_MAX_STREAMS` (bidi) | `0x190B4D3F` | 双方向ストリーム累積上限 |
| `WT_MAX_STREAMS` (uni) | `0x190B4D40` | 単方向ストリーム累積上限 |
| `WT_DATA_BLOCKED` | `0x190B4D41` | データ送信がフロー制御でブロック |
| `WT_STREAMS_BLOCKED` (bidi) | `0x190B4D43` | 双方向ストリームのブロック通知 |
| `WT_STREAMS_BLOCKED` (uni) | `0x190B4D44` | 単方向ストリームのブロック通知 |

- 前回の値より小さい値の `WT_MAX_STREAMS` / `WT_MAX_DATA` を受信した場合は `WT_FLOW_CONTROL_ERROR` (draft-16 以降では増加しない値、同値を含む)。本実装はドラフト分岐を持たず、draft-14 以降のすべての版で draft-16 以降の「増加しない値」判定を適用する
- draft-17 では、最初の `WT_MAX_STREAMS` / `WT_MAX_DATA` についても、SETTINGS で宣言した初期値より増加していなければ `WT_FLOW_CONTROL_ERROR` とする (draft-17 Section 5.6.2 / 5.6.4)。本実装はドラフト分岐を持たず、draft-14 以降のすべての版で `Session::initialize_remote_limits` に設定した初期値を比較の基準にする
- `WT_MAX_STREAMS` の Maximum Streams が 2^60 を超えた場合、draft-15 では `H3_DATAGRAM_ERROR`、draft-16 / 17 では `WT_FLOW_CONTROL_ERROR`。本実装はフロー制御カプセルの判定を draft-14 以降に共通で適用するため、draft-14 以降のどの版でも `WT_FLOW_CONTROL_ERROR` (セッションエラー) を返す
- 全フロー制御カプセルは hop-by-hop (中継者が消費して自身の信号を生成)

### 4.4 禁止カプセル (draft-14 以降)

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

### 5.4 実装のセッション状態

本実装の `webtransport::Session` は `SessionState` で状態を管理する。状態は `Pending` (CONNECT 送信前)、`Connecting` (CONNECT 送信済み、応答待ち)、`Established`、`Draining` (グレースフルシャットダウン中)、`Closed` の順に遷移する。

- `Session::drain` は `Established` のとき `WT_DRAIN_SESSION` カプセルを送信キューに積み、`Draining` へ遷移する
- `Session::close_with_error` は `WT_CLOSE_SESSION` カプセルを送信キューに積み、`Closed` へ遷移する

---

## 6. エラーコード

### 6.1 プロトコルエラーコード

| エラーコード | コードポイント | 対応 draft |
|-------------|---------------|-----------|
| `H3_WEBTRANSPORT_BUFFERED_STREAM_REJECTED` / `WT_BUFFERED_STREAM_REJECTED` | `0x3994bd84` | 全 draft |
| `WEBTRANSPORT_SESSION_GONE` / `WT_SESSION_GONE` | `0x170d7b68` | draft-07 以降 |
| `WT_FLOW_CONTROL_ERROR` | `0x045d4487` | draft-14 以降 |
| `WT_ALPN_ERROR` | `0x0817b3dd` | draft-15 以降 |
| `WT_REQUIREMENTS_NOT_MET` | `0x212c0d48` | draft-15 以降 |

本実装は `WT-Protocol` の検証に失敗した場合、draft-14 を選択していても `WT_ALPN_ERROR` でセッションを終了する (draft-14 の「無視する」規則より厳しい側に揃えている)。draft を問わず同じ経路 (`src/connection/wt_session.rs`) で判定するためである。

### 6.2 HTTP/3 既存エラーコードの使用

| エラーコード | 用途 | 適用 draft |
|-------------|------|-----------|
| `H3_ID_ERROR` | 不正なセッション ID | 全 draft |
| `H3_FRAME_UNEXPECTED` | `WEBTRANSPORT_STREAM` / `WT_STREAM` の不正な位置での受信 | draft-07 以降 (仕様は draft-16 まで `H3_FRAME_ERROR`、draft-17 で `H3_FRAME_UNEXPECTED` に変更)。本実装は全版で `H3_FRAME_UNEXPECTED` を返す |
| `H3_FRAME_ERROR` | フレームの長さ不整合など、一般のフレームエラー | 全 draft (WT_STREAM の不正位置には使用しない) |
| `H3_MISSING_SETTINGS` | 制御ストリームの最初のフレームが SETTINGS でない場合と、制御ストリームで SETTINGS 受信前にフレームタイプ `0x41` を受信した場合 (RFC 9114 Section 6.2.1) | 全 draft |
| `H3_MESSAGE_ERROR` | セッション終了カプセル後の追加データ受信 | 全 draft |
| `H3_SETTINGS_ERROR` | SETTINGS 値の不正。draft-14 / 15 は 0-RTT 再開時のフロー制御値減少 (仕様上はサーバーの義務)、draft-16 はクライアントも減少検出時に接続を閉じる | 全 draft (0-RTT の減少検出は draft-17 では適用しない) |
| `H3_REQUEST_REJECTED` / `HTTP_REQUEST_REJECTED` | セッション数超過時のリクエスト拒否 | 全 draft |
| `H3_DATAGRAM_ERROR` | `WT_MAX_STREAMS` の値超過 | draft-15 の仕様上のコード。本実装は draft-14 以降に draft-16 以降のフロー制御カプセル判定を適用するため `WT_FLOW_CONTROL_ERROR` を返す |
| `H3_EXCESSIVE_LOAD` | 不審な使用パターン | draft-14 以降 |

---

## 7. フロー制御 (draft-14 以降)

draft-14 以降ではセッションレベルのフロー制御が追加された。これは QUIC のコネクションレベルフロー制御と類似の仕組みをセッション単位で提供する。

### 7.1 有効化条件

**draft-14 以降**:

- 両エンドポイントが `SETTINGS_WT_INITIAL_MAX_STREAMS_UNI`, `SETTINGS_WT_INITIAL_MAX_STREAMS_BIDI`, `SETTINGS_WT_INITIAL_MAX_DATA` のうち少なくとも 1 つを非ゼロ値で送信した場合に有効 (draft-15 / 16 / 17 Section 5.1)
- draft-15 / 16 / 17 のサーバーと、旧 draft にも対応するクライアントは `SETTINGS_WT_ENABLED` (`0x2c7cf000` または `0x20b8933b`) を送信する。本文版だけに対応するクライアントは送らない
- `SETTINGS_WT_ENABLED` を送信する端点は、互換のために `SETTINGS_WT_MAX_SESSIONS` (`0x14e9cd29`) を併送してもフロー制御の宣言とはみなさない (draft-15 / 16 / 17 Section 5.1)
- `SETTINGS_WT_ENABLED` を送らない draft-14 以前の端点では、draft-14 Section 5.1 の `SETTINGS_WT_MAX_SESSIONS > 1` も宣言条件に含む。本文版だけに対応するクライアントは `SETTINGS_WT_MAX_SESSIONS` を送らないため、初期値 (`SETTINGS_WT_INITIAL_MAX_*`) だけで宣言する
- フロー制御が無効の場合、同時セッションは 1 つに制限される

### 7.2 制御の種類

| 制御対象 | 初期値の SETTINGS | 動的更新カプセル | ブロック通知カプセル |
|---------|------------------|-----------------|-------------------|
| 単方向ストリーム数 | `SETTINGS_WT_INITIAL_MAX_STREAMS_UNI` | `WT_MAX_STREAMS` (uni) | `WT_STREAMS_BLOCKED` (uni) |
| 双方向ストリーム数 | `SETTINGS_WT_INITIAL_MAX_STREAMS_BIDI` | `WT_MAX_STREAMS` (bidi) | `WT_STREAMS_BLOCKED` (bidi) |
| データ量 | `SETTINGS_WT_INITIAL_MAX_DATA` | `WT_MAX_DATA` | `WT_DATA_BLOCKED` |

**実装での扱い**:

- sans-I/O 層では、ピアの `SETTINGS` 受信後・セッション確立前に
  `webtransport::Session::initialize_remote_limits(FlowControlLimits)` を 1 回呼ぶ。
  この値が送信側の上限 (`can_create_*` / `can_send_data` / `try_open_stream` /
  `try_send_data`) と、`WT_MAX_STREAMS` / `WT_MAX_DATA` の単調性検証の基準になる
  (draft-ietf-webtrans-http3-16 / 17 Section 5.5, Section 5.6.2, Section 5.6.4)
- `Connection` はフロー制御カプセルを `WebTransportEvent::Capsule` として上位層へ
  転送するだけで、値域・単調性の検証は行わない。上位層が
  `webtransport::Session::process_capsule` に渡すこと
- 単調性違反は `WT_FLOW_CONTROL_ERROR` によるセッションエラーであり、接続エラーでは
  ないため、`Connection` は接続を閉じない

### 7.3 ストリームレベルフロー制御

- HTTP/3 版では QUIC ネイティブのストリームレベルフロー制御を使用する
- `WT_MAX_STREAM_DATA` / `WT_STREAM_DATA_BLOCKED` カプセルは HTTP/3 版では禁止

---

## 8. draft-14 以降の新機能

これらの機能は draft-14 で導入され、draft-15 / 16 / 17 でも維持されている。draft-15 固有の変更は `:protocol` の `webtransport-h3` 化と `SETTINGS_WT_ENABLED` への置き換えである。

### 8.1 RESET_STREAM_AT の必須化

`reset_stream_at` トランスポートパラメータが前提条件に追加された。ストリームリセット時にストリームヘッダー (Session ID) の確実な配信を保証するため、Reliable Size をヘッダーサイズ以上に設定する必要がある。draft-17 では、これにアプリケーション指定オフセットを加える (本書 Section 2.3)。

参照: `draft-ietf-quic-reliable-stream-reset-07` (draft-14 / 15)。draft-16 は `-09`、draft-17 は `-11` を参照する。

**実装での扱い**:

- `shiguredo_http3` は RESET_STREAM_AT の Reliable Size を決めるための WT ストリームのヘッダー長を上位層へ提供する (`Connection::wt_stream_header_len`)。RESET_STREAM_AT フレームの送出自体は QUIC 層 (統合層) の責務である
- QUIC 層が RESET_STREAM_AT を公開していない場合、統合層は通常の RESET_STREAM を使うしかなく、この MUST を満たせない。接続自体は維持し、既知の制限として扱う。
- `examples/wt_server` が使う s2n-quic 1.91 は RESET_STREAM_AT の送出 API も、ピアの `reset_stream_at` トランスポートパラメータを取得する API も公開していない。そのため `examples/wt_server` は `reset_stream_at` 非対応 (`false` 固定) で動作する。`shiguredo_http3` は、ネゴシエートした draft が `reset_stream_at` を要求する場合に CONNECT を `H3_MESSAGE_ERROR` で拒否するため (`src/connection/wt_session.rs` の `validate_wt_connect_request_server`)、`examples/wt_server` に接続できるのは `reset_stream_at` を要求しない draft-02 / draft-07 クライアントに限られる。draft-14 / 15 / 16 / 17 クライアントとの接続は、s2n-quic が両 API を公開して `reset_stream_at` 対応へ切り替えるまで行えない。

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
| `SETTINGS_WT_MAX_SESSIONS` | `0x14e9cd29` | draft-14 |
| `SETTINGS_WT_ENABLED` | `0x2c7cf000` | draft-15 / draft-16 |
| `SETTINGS_WT_ENABLED` | `0x20b8933b` | draft-17 |
| `SETTINGS_WT_INITIAL_MAX_STREAMS_UNI` | `0x2b64` | draft-14 以降 |
| `SETTINGS_WT_INITIAL_MAX_STREAMS_BIDI` | `0x2b65` | draft-14 以降 |
| `SETTINGS_WT_INITIAL_MAX_DATA` | `0x2b61` | draft-14 以降 |

draft-17 で変わったのは `SETTINGS_WT_ENABLED` のコードポイントだけであり、ほかの初期値 SETTINGS のコードポイントは変わらない。

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
| `WT_MAX_DATA` | `0x190B4D3D` | draft-14 以降 |
| `WT_MAX_STREAM_DATA` (禁止) | `0x190B4D3E` | draft-14 以降 |
| `WT_MAX_STREAMS` (bidi) | `0x190B4D3F` | draft-14 以降 |
| `WT_MAX_STREAMS` (uni) | `0x190B4D40` | draft-14 以降 |
| `WT_DATA_BLOCKED` | `0x190B4D41` | draft-14 以降 |
| `WT_STREAM_DATA_BLOCKED` (禁止) | `0x190B4D42` | draft-14 以降 |
| `WT_STREAMS_BLOCKED` (bidi) | `0x190B4D43` | draft-14 以降 |
| `WT_STREAMS_BLOCKED` (uni) | `0x190B4D44` | draft-14 以降 |

### HTTP/3 エラーコード

| エラーコード | コードポイント | draft |
|-------------|---------------|-------|
| `WT_BUFFERED_STREAM_REJECTED` | `0x3994bd84` | 全 draft |
| `WT_SESSION_GONE` | `0x170d7b68` | draft-07 以降 |
| `WT_FLOW_CONTROL_ERROR` | `0x045d4487` | draft-14 以降 |
| `WT_ALPN_ERROR` | `0x0817b3dd` | draft-15 以降 |
| `WT_REQUIREMENTS_NOT_MET` | `0x212c0d48` | draft-15 以降 |
| `WT_APPLICATION_ERROR` 範囲 | `0x52e4a40fa8db` - `0x52e5ac983162` | draft-07 以降 (draft-02 は 8 bit 範囲) |

draft-17 では `WT_SESSION_GONE` の登録書式に Reference、Change Controller、Contact が追加されたが、コードポイントと意味は変わらない。

---

## 10. SETTINGS_ENABLE_CONNECT_PROTOCOL の送信義務

`SETTINGS_ENABLE_CONNECT_PROTOCOL` (0x08) は RFC 8441 Section 3 で定義されたサーバーからクライアントへの設定であり、クライアントに送信義務はない。

> Upon receipt of SETTINGS_ENABLE_CONNECT_PROTOCOL with a value of 1, a client MAY use the Extended CONNECT as defined in this document when creating new streams. Receipt of this parameter by a server does not have any impact.
> --- RFC 8441 Section 3

RFC 9220 も同様に「a new HTTP/2 setting sent by a server to allow the client to use Extended CONNECT」と定義している。

draft-ietf-webtrans-http3-16 / 17 Section 3.1 では、サーバーとクライアントの送信リストを明確に分離している:

**サーバーが送信するもの:**
- `SETTINGS_WT_ENABLED` (値 1。draft-15 では値 > 0。draft-15 / 16 は `0x2c7cf000`、draft-17 は `0x20b8933b`)
- `SETTINGS_ENABLE_CONNECT_PROTOCOL` (値 1)
- `SETTINGS_H3_DATAGRAM` (値 1)
- `max_datagram_frame_size` transport parameter (値 > 0)
- `reset_stream_at` transport parameter (空)

**クライアントが送信するもの:**
- `SETTINGS_H3_DATAGRAM` (値 1)
- `max_datagram_frame_size` transport parameter (値 > 0)
- `reset_stream_at` transport parameter (空)
- ドラフト版のみ: `SETTINGS_WT_ENABLED` (ドラフト固有コードポイント)。本文版だけに対応するクライアントは送信しない

クライアントの送信リストに `SETTINGS_ENABLE_CONNECT_PROTOCOL` は含まれない。サーバーはクライアントが `SETTINGS_ENABLE_CONNECT_PROTOCOL` を送信しなくても WebTransport CONNECT リクエストを受理しなければならない。

nghttp3 (ngtcp2) は RFC に準拠し、クライアント側では `enable_connect_protocol = 0` に強制設定して SETTINGS フレームに含めない (`nghttp3_conn.c` line 420)。

---

## 11. draft-16 固有の変更

SETTINGS コードポイントと `:protocol` は draft-15 と draft-16 で同一である。本実装は draft-15 と draft-16 を `DraftVersion::Draft15` として同じに扱い、draft-16 の規則を採用する。下表は draft-16 で変更された規則を示す。

| 項目 | draft-15 | draft-16 (本実装が採用する規則) |
|------|----------|----------|
| `SETTINGS_WT_ENABLED` | 値 > 0 | 値は 1。クライアントは > 1 を `H3_SETTINGS_ERROR` として扱う |
| `WT_MAX_DATA` / `WT_MAX_STREAMS` の単調性 | 前回より小さい値はエラー | 増加しない値 (同値を含む) は `WT_FLOW_CONTROL_ERROR` |
| `WT_MAX_STREAMS` > 2^60 | `H3_DATAGRAM_ERROR` (接続エラー) | `WT_FLOW_CONTROL_ERROR` (セッションエラー) |
| `WT_STREAMS_BLOCKED` > 2^60 | 検証なし | `WT_FLOW_CONTROL_ERROR` (セッションエラー) |
| 非対応リソースへの応答 | 404 を推奨 | 405 を推奨 |
| 0-RTT 再開時のフロー制御値減少 | サーバー側の義務 | クライアントも `H3_SETTINGS_ERROR` で接続を閉じる |
| 楽観的カプセル送信 | 規定なし | クライアントは 2xx 受信前にカプセルを送ってよい。サーバーは 2xx 送信まで処理しない |

フロー制御カプセルの単調性と `WT_MAX_STREAMS > 2^60` の判定はドラフト分岐を持たず、draft-14 以降のすべての版に適用される。`SETTINGS_WT_ENABLED > 1` をクライアントが受信した場合の拒否は draft-16 / 17 の規則であり、本実装は draft-15 を選んだ接続にも適用する。

draft-17 では、この表の 0-RTT 再開時のフロー制御値減少の規則を適用しない。前回接続の WT SETTINGS を破棄し、現在接続の値だけを使う (本書 Section 1.3)。WT_STREAM の不正な位置のエラーコードも `H3_FRAME_UNEXPECTED` に変わっている (本書 Section 2.2)。

---

## 12. draft-17 の変更

### 12.1 draft-16 から draft-17 への差分

draft-16 と draft-17 の本文を比較し、規範的な変更、既存実装で満たす明確化、編集上の変更を整理した。対応欄は本リポジトリでの扱いを示す。

| 節 | 差分 | 対応 |
|---|---|---|
| 表紙、目次、ページヘッダー、索引 | 公開日、有効期限、版番号、ページ位置の更新 | 変更不要 (原文を `refs/` に保存) |
| 2.2 | サーバーの識別を authority / path の組から HTTPS URI の参照へ変更 | 変更不要 (CONNECT の `:authority` / `:path` と `https` の検証は変わらない) |
| 3.1 | サーバーはクライアントが送る `SETTINGS_WT_ENABLED` を通常は無視し、draft 版では版選択に使う | 実装済み (本書 Section 1.1) |
| 3.1 | 対応する draft と本文版のコードポイントをクライアントが広告する規則を明確化 | 実装済み (本書 Section 1.1 / 12.2) |
| 3.2 | 0-RTT 再開時に前回接続の WT SETTINGS を破棄する | 実装済み (本書 Section 1.3) |
| 3.3 | 重複していた `WT-Available-Protocols` の導入文を削除 | 変更不要 (編集上の変更) |
| 4.3 | `WT_STREAM` の許可位置を双方向ストリーム開始者の最初のバイト列と明記し、違反を `H3_FRAME_UNEXPECTED` に変更 | 実装済み (本書 Section 2.2。旧 draft の接続でも `H3_FRAME_UNEXPECTED` を返す) |
| 4.4 | Reliable Size にアプリケーション指定オフセットを含める | 未対応 (今後対応、本書 Section 2.3) |
| 4.4 | `WT_APPLICATION_ERROR` 範囲外の扱いに `RESET_STREAM_AT` を追加 | 実装済み (範囲外はアプリケーションエラーコードに変換しない) |
| 4.4 | 中継者が `RESET_STREAM_AT` を転送するときの Reliable Size 補正 | 変更不要 (中継機能なし) |
| 4.6 | バッファ拒否がセッションに影響しないこと、先行送信が拒否され得ること、新ストリームで再送できることを補足 | 通知まで実装済み (本書 Section 12.3。リセットの送出は利用側の責務) |
| 5.6.1 | 中継が送るフロー制御カプセルも上限を増加させる必要があると補足 | 変更不要 (中継機能なし) |
| 5.6.2 / 5.6.4 | 最初の `WT_MAX_STREAMS` / `WT_MAX_DATA` も SETTINGS の初期値を基準に含めて増加を判定すると明記 | 実装済み (`Session::initialize_remote_limits` / `process_capsule`) |
| 7.1 | 版ごとの広告、本文版と draft の混在選択、本文版へのフォールバックを明記 | 実装済み (本書 Section 12.2) |
| 8 | フロー制御無効時の HTTP / QPACK の停滞とデッドロックを補足 | 変更不要 (本書 Section 12.4。フロー制御実装 MUST は draft-16 Section 5.1 / Section 8 に既にある) |
| 9.2 | `SETTINGS_WT_ENABLED` の登録 ID を `0x2c7cf000` から `0x20b8933b` に変更 | 実装済み (本書 Section 1.1 / 9) |
| 9.5 | `WT_SESSION_GONE` の登録書式を整備 | 変更不要 (コードポイントと意味は不変) |
| 10 | `OVERVIEW` を 12 → 13、`RESET-STREAM-AT` を 09 → 11、HTTP/2 を 14 → 15 に更新し、RFC 3986 の直接参照を削除 | 変更不要 (原文を保持。ローカルの参照は最新版に更新済み) |

4.1 / 4.2 / 4.5 / 4.7 / 4.8、フロー制御の残りの本文、セッション終了、カプセルのコードポイント、アプリケーションプロトコルの解析規則には、上表以外の意味上の変更はない。

### 12.2 バージョンネゴシエーション

接続で使う版は、SETTINGS のコードポイントを交わして接続単位で決める。draft-17 Section 7.1 がその規則を定める。

#### コードポイントの広告

- 端点は、対応する版のコードポイントを 1 回ずつ広告する。draft-15 / 16 は `0x2c7cf000`、draft-17 の本文版は `0x20b8933b` を使う
- draft-02 は `Sec-Webtransport-Http3-Draft02` ヘッダーで版を通知する。draft-07 以降は SETTINGS コードポイントを使い、draft-14 以降は ALPN (`WT-Available-Protocols` / `WT-Protocol`) も併用する (本書 Section 1.1 / 1.2 を参照)
- サーバーは、クライアントが送る `SETTINGS_WT_ENABLED` を通常は無視する。旧 draft にも対応するサーバーは、クライアントが広告したコードポイントを版の選択に使う

#### サーバー側の選択

- 本文版に対応するサーバーは、クライアントが `0x20b8933b` を広告していれば本文版を選ぶ
- 広告していない場合、サーバーは共通する旧 draft のうち優先順位の高い版を選ぶ
- 共通する旧 draft がなく、サーバーが本文版に対応していれば、本文版を選ぶ。クライアントが本文版に対応しない場合、セッションは確立できない
- 本文版に対応せず、`SETTINGS_WT_ENABLED` を広告しているサーバーで共通する版がない場合、サーバーはクライアントの CONNECT を拒否する
- サーバーは、クライアントの SETTINGS を受信するまで WebTransport リクエストの処理を保留する。ストリームヘッダーの形式が版に依存するためである

#### クライアント側の選択

- ローカルとサーバーが共通して広告する版のうち、優先順位の高い版を選ぶ
- 共通する版がなく、ローカルが版を 1 つも広告せず、サーバーが `0x20b8933b` を広告している場合は、本文版を選ぶ
- それ以外で共通する版がない場合、使用する版がないため WebTransport を開始しない

#### 端点の種類ごとの扱い

- 本文版と旧 draft の両方に対応するクライアントは、対応するコードポイントをそれぞれ広告する。`0x20b8933b` を送った場合、サーバーは本文版を選ぶ。送らない場合、サーバーは共通する旧 draft のうち優先順位の高い版を選ぶ
- 本文版だけに対応するクライアントは `SETTINGS_WT_ENABLED` を送らない。サーバーが `0x20b8933b` を広告していれば、クライアントは本文版を選ぶ。サーバーは、`SETTINGS_WT_ENABLED` を送らないクライアントを本文版として扱う
- 旧 draft だけに対応するクライアントは、旧 draft のコードポイントだけを広告する。本文版に対応するサーバーは、共通する旧 draft があればそのうち優先順位の高い版を選ぶ。共通する旧 draft がなければ本文版を選ぶが、そのクライアントは本文版に対応しないためセッションを確立できない

#### 本実装の選択順序

本実装の優先順位は、draft-17 の本文版、draft-15 / 16、draft-07、draft-14、draft-02 である。

draft-17 Section 7.1 の「両端点が対応する最も高い版を選ぶ」規則に対する例外として、draft-07 と draft-14 のコードポイントが併送された場合は draft-07 を優先する。Safari 26.4 (Network.framework) との互換のためであり、Safari 以外の端点が同じ組み合わせを広告した場合にも適用する (`SAFARI_WT.md` を参照)。

接続単位の選択結果は `Connection::negotiated_wt_draft_version` で取得できる。

### 12.3 先行ストリームとデータグラムの拒否

draft-17 Section 4.6 は、CONNECT の応答が到着する前に届いたストリームやデータグラムの扱いを補足する。クライアントは SETTINGS の受信後に CONNECT を送るが、複数の CONNECT、WT ストリーム、データグラムを 1 つのフライトで送れるため、サーバーは対応するセッションがまだないストリームやデータグラムを受信し得る。クライアントも、2xx 応答より先にサーバー開始のストリームやデータグラムを受信し得る。

- 端点は、ストリームとデータグラムをセッションに関連付けられるまでバッファリングすべき (SHOULD)
- バッファリングするストリーム数とデータグラム数には上限を設けなければならない (MUST)
- ストリーム数の上限を超えた場合、仕様上は `WT_BUFFERED_STREAM_REJECTED` で `RESET_STREAM` または `STOP_SENDING` を送って閉じる
- データグラム数の上限を超えた場合、データグラムは破棄する
- ストリームを拒否しても、そのストリームを送ろうとしたセッションには影響しない
- 2xx 応答を待たずにストリームやデータグラムを送った端点は、拒否または破棄されることを前提にする。必要なら同じセッションの新しいストリームでデータを送り直せる

本実装の sans-I/O 層は、拒否したストリームを `WebTransportEvent::BufferedStreamRejected` として通知し、セッション終了とは分けて扱う。`RESET_STREAM` / `STOP_SENDING` の送出は行わない。通知を受けた利用側 (統合層またはアプリケーション) が、イベントの `error_code` を使ってリセットを送る。tokio-s2n-quic の `H3Server` は、このイベントを受けて `RESET_STREAM` と `STOP_SENDING` を送る。WebTransport のセッション API はイベントをアプリケーションへ転送するため、アプリケーションがリセットを送る。再送は自動では行わない。バッファリングの上限は実装の固定値 (ストリーム数 100、データグラム数 100、1 ストリームあたり 64 KiB) であり、アプリケーションが変更する API はない。

### 12.4 接続共有時のデッドロック

draft-17 Section 8 は、フロー制御を有効にせずに WebTransport セッションを HTTP/3 接続へ相乗りさせた場合のデッドロックを説明する。複数の WebTransport セッションに接続を共有させる端点は、フロー制御を実装しなければならない (MUST)。この MUST は draft-16 Section 5.1 / Section 8 に既にある。draft-17 で追加されたのは、フロー制御を無効にした場合の停滞とデッドロックの段落である。WebTransport セッションは下位 QUIC 接続のストリーム数とデータ量の上限まで使えるため、同じ接続の HTTP 応答と QPACK ストリームの進行を止め得る。HTTP 応答と QPACK ストリームは接続レベルのフロー制御を共有するため、アプリケーションが WebTransport セッションの読み取りを、同じ接続上の HTTP 応答の完了まで待つと、双方が進めなくなる。

回避策は、WebTransport のフロー制御を有効にしてセッションごとの上限を設けるか、WebTransport セッションを別の接続へ分離することである。本実装の sans-I/O 層は SETTINGS とカプセルによるフロー制御を提供するが、実際に有効にできるかはアプリケーションと各アダプターの設定に依存する。

---

## 13. 実装の設定方法

`webtransport::Settings::for_draft_versions` は、渡した版の SETTINGS コードポイントを広告する設定を組み立てる。

- `DraftVersion::Draft17` を渡すと `SETTINGS_WT_ENABLED` (`0x20b8933b`) を送る。サーバーが本文版に対応することを示す場合は、この設定が必要である
- 本文版だけに対応するクライアントは `SETTINGS_WT_ENABLED` を送らない。`for_draft_versions` に `DraftVersion::Draft17` を渡さず、版を 1 つも含まない `webtransport::Settings::new()` を `enable_webtransport_client` に渡す。この場合も、サーバーが `0x20b8933b` を広告していれば本文版が選ばれる
- 本文版と旧 draft の両方に対応するクライアントは、対応する旧 draft のコードポイントと `DraftVersion::Draft17` の両方を含める
- フロー制御を有効にする場合は、`wt_initial_max_streams_uni` などのビルダーメソッドで初期値を設定する。`for_draft_versions` はこれらの初期値を設定しない

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

draft-16 は draft-15 と同じコードポイントを使うため、`DraftVersion::Draft15` として扱う (本書 Section 11 を参照)。
