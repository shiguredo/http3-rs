# Safari WebTransport 対応状況

## 確認環境

- Safari 26.4 (macOS)
- 初回確認日: 2026-03-29
- フロー制御カプセル確認日: 2026-04-03
- draft 分類・応答 SETTINGS 制約確認日: 2026-04-06

## TL;DR

- Safari 26.4 の WebTransport 実装は **draft-07 と draft-13/14 のハイブリッド**である。
  - SETTINGS では draft-07 の `0xc671706a` と draft-13/14 の `0x14e9cd29` / `WT_INITIAL_MAX_*` を併送する。
  - `:protocol: webtransport` (draft-14 まで) を使い、フロー制御はカプセルベース (draft-13〜)。
- **サーバーが返してよい WT 系応答 SETTINGS は `0xc671706a` 単体のみ**。`0x14e9cd29` や `WT_INITIAL_MAX_*` を返すと `H3_REQUEST_CANCELLED` (0x10C) で拒否される。
- **セッション確立 (200 応答) 直後に CONNECT ストリームで `WT_MAX_STREAMS` / `WT_MAX_DATA` カプセルを送る必要がある**。各カプセルは個別の H3 DATA フレームで包むこと。
- Datagram は http3-rs 側で問題なく動作する。一時的な不動作は W3C WebTransport API 側の問題だった。
- 本実装では `DraftVersion::Draft07` として検出しつつ、`Settings::requires_initial_capsule_flow_control_compat()` で draft-13/14 由来の互換カプセル送出を行う二段構えで対応している。

## Safari の draft 分類

Safari 26.4 の実装は仕様的には **draft-07 と draft-13/14 のハイブリッド** であり、単一の draft には一致しない。

`refs/webtrans/` 以下の各 draft を精査した結果、Safari が送る SETTINGS / カプセル / ヘッダーのコードポイントは次のように分類される:

| 要素 | コードポイント | 由来 draft | Safari 送信 |
|---|---|---|---|
| `SETTINGS_WEBTRANSPORT_MAX_SESSIONS` | `0xc671706a` | draft-07〜12 (draft-13 で廃止) | あり |
| `SETTINGS_WT_MAX_SESSIONS` | `0x14e9cd29` | draft-13〜14 (draft-15 で廃止) | あり |
| `SETTINGS_WT_INITIAL_MAX_STREAMS_UNI` | `0x2b64` | draft-13〜15 | あり |
| `SETTINGS_WT_INITIAL_MAX_STREAMS_BIDI` | `0x2b65` | draft-13〜15 | あり |
| `SETTINGS_WT_INITIAL_MAX_DATA` | `0x2b61` | draft-13〜15 | あり |
| `SETTINGS_WT_ENABLED` | `0x2c7cf000` | draft-15 のみ | **なし** |
| `SETTINGS_ENABLE_WEBTRANSPORT` | `0x2b603742` | draft-02 のみ | **なし** |
| `SETTINGS_H3_DATAGRAM` (RFC 9297) | `0x33` | — | あり (= 1) |
| `:protocol` 値 | `webtransport` | draft-02〜14 (draft-15 は `webtransport-h3`) | `webtransport` |
| `WT_MAX_STREAMS (bidi)` capsule | `0x190B4D3F` | draft-13〜15 | 受信側として必須 |
| `WT_MAX_STREAMS (uni)` capsule | `0x190B4D40` | draft-13〜15 | 受信側として必須 |
| `WT_MAX_DATA` capsule | `0x190B4D3D` | draft-13〜15 | 受信側として必須 |

### 結論

- **draft-15 ではない**: `SETTINGS_WT_ENABLED` (0x2c7cf000) を送らず、`:protocol` も旧名 `webtransport`。
- **draft-13/14 相当が実装の本体**: `WT_MAX_SESSIONS` / `WT_INITIAL_MAX_*` / カプセルのコードポイントは draft-13 で導入され draft-14 まで同一。draft-13 と draft-14 は SETTINGS だけでは区別不能。カプセルベースフロー制御の挙動は draft-14 の記述に近い。
- **draft-07 の `WEBTRANSPORT_MAX_SESSIONS` は後方互換フォールバック広告**: 古いサーバーとの互換のための併送と解釈できる。

## Safari が送信する H3 SETTINGS

Safari 26.4 のクライアント SETTINGS 実測例:

| ID | 名前 | 値 |
|---|---|---|
| `0x01` | `QPACK_MAX_TABLE_CAPACITY` | 16383 |
| `0x07` | `QPACK_BLOCKED_STREAMS` | 100 |
| `0x33` | `SETTINGS_H3_DATAGRAM` (RFC 9297) | 1 |
| `0x1d1e6bb27c` | GREASE (RFC 9114 reserved) | 接続ごとに変動 |
| `0xc671706a` | `SETTINGS_WEBTRANSPORT_MAX_SESSIONS` | 1 |
| `0x14e9cd29` | `SETTINGS_WT_MAX_SESSIONS` | 1 |
| `0x2b61` | `SETTINGS_WT_INITIAL_MAX_DATA` | 8388608 |
| `0x2b64` | `SETTINGS_WT_INITIAL_MAX_STREAMS_UNI` | 100 |
| `0x2b65` | `SETTINGS_WT_INITIAL_MAX_STREAMS_BIDI` | 100 |

備考:

- `SETTINGS_ENABLE_CONNECT_PROTOCOL` (`0x08`, RFC 8441) は **クライアント側からは送られない**。これはサーバー側が送るべき設定なので仕様上問題はない。
- GREASE ID は RFC 9114 Section 7.2.4.1 の予約パターン (`0x1f * N + 0x21`)。値は接続ごとに変わる。サーバーは無視すること。

## Safari が送信する CONNECT リクエスト

```
:method: CONNECT
:scheme: https
:authority: <host>:<port>
:path: /<resource>
:protocol: webtransport
origin: <origin>
wt-available-protocols: <空>
```

- `:protocol` は `webtransport` (draft-02〜14 共通)。draft-15 以降の `webtransport-h3` ではない。
- `wt-available-protocols` は draft-14 Section 4 の拡張。現状は空で送られる。

## サーバーが返す応答 SETTINGS の制約

Safari との互換で最も重要な制約: **サーバーが応答 SETTINGS に入れてよい WebTransport 系 ID は `SETTINGS_WEBTRANSPORT_MAX_SESSIONS` (`0xc671706a`) 単体のみ**。

以下のいずれかを応答 SETTINGS に含めると Safari は CONNECT 双方向ストリームを `H3_REQUEST_CANCELLED` (0x10C = 268 decimal) でリセットする:

- `SETTINGS_WT_MAX_SESSIONS` (`0x14e9cd29`)
- `SETTINGS_WT_INITIAL_MAX_STREAMS_UNI` (`0x2b64`)
- `SETTINGS_WT_INITIAL_MAX_STREAMS_BIDI` (`0x2b65`)
- `SETTINGS_WT_INITIAL_MAX_DATA` (`0x2b61`)
- `SETTINGS_WT_ENABLED` (`0x2c7cf000`)

### 非対称挙動の観測

Safari は `0x14e9cd29` や `WT_INITIAL_MAX_*` を **自分からは送るが、サーバーが返すと拒否する** という非対称な挙動を取る。これは Safari Network.framework 側のバグか「旧サーバー互換のため応答 SETTINGS は draft-07 形式のみ受理する」という実装方針と思われるが、クローズドソースのため確認不能。

### 実測エラー例 (2026-04-06)

`detect_draft_pattern` を draft-14 優先に戻して `Draft14` 応答 builder (`wt_max_sessions_draft14 + webtransport_max_sessions_draft07` の 2 ID) を返した場合:

```
CONNECT stream receive error: StreamReset {
  error: application::Error(268)   // 268 = 0x10C = H3_REQUEST_CANCELLED
}
```

この実測結果から、仕様的正しさよりも **「応答 SETTINGS は draft-07 の 1 ID のみ」を厳守する必要がある** ことが確定した。

### 旧事例: 全 draft の SETTINGS を同時送信したケース

対応初期には、サーバーが draft-02/07/15 の全設定を応答 SETTINGS に含めて送信していた。この場合 Safari は CONNECT リクエストを送らずに制御ストリームレベルで同じ `H3_REQUEST_CANCELLED` (0x10C) でリセットし、`H3_NO_ERROR` (0x100) で切断していた。

クライアントの SETTINGS を先に受信してから draft を判定し、対応する draft 用の応答 SETTINGS のみを返すように修正した。現在は Safari 実測に合わせ、`Draft07` 応答として `0xc671706a` 1 個だけを返す形になっている。

## カプセルベースフロー制御 (必須)

Safari は draft-07 の SETTINGS で接続するが、ストリーム作成とデータ送信には **draft-13/14 で導入されたカプセルベースフロー制御** を要求する。サーバーがカプセルを送らないと、Safari はストリーム上限 / データ上限を 0 と判断して何もストリームを開けず、最終的にセッションを `WT_CLOSE_SESSION` でクローズする。

### 必要なカプセル

セッション確立 (200 応答送信) 直後に、CONNECT ストリーム経由で以下のカプセルを送信する:

| カプセル | タイプ ID | 説明 |
|---|---|---|
| `WT_MAX_STREAMS` (bidi) | `0x190B4D3F` | 双方向ストリーム上限 |
| `WT_MAX_STREAMS` (uni) | `0x190B4D40` | 単方向ストリーム上限 |
| `WT_MAX_DATA` | `0x190B4D3D` | セッションレベルのデータ上限 |

### 送信フォーマット

カプセルは H3 DATA フレーム (type=`0x00`) に包んで送信する:

```
[DATA frame type: 0x00] [length: varint] [capsule bytes]
```

### 厳守事項

- **各カプセルは個別の H3 DATA フレームで送る**。複数カプセルを 1 つの DATA フレームにまとめると Safari が処理しない。
- **raw カプセルバイトを DATA フレームで包まずに送ると Safari Network.framework がクラッシュする**。
- **応答 SETTINGS に `WT_INITIAL_MAX_*` を含めない**。前述の通り `H3_REQUEST_CANCELLED` で拒否される。

### カプセル未送信時の Safari の挙動

1. Safari はストリーム上限 / データ上限を 0 と判断する。
2. ストリーム作成時に `WT_STREAMS_BLOCKED` カプセル (type=`0x190B4D43`) を CONNECT ストリームで送信する。
3. データ送信時に `WT_DATA_BLOCKED` カプセル (type=`0x190B4D41`) を送信する。
4. ストリームが開けず、最終的にセッションを `WT_CLOSE_SESSION` でクローズする。

## http3-rs 実装での対応方針

本リポジトリでは次の二段構えで Safari 互換を実現している:

1. **`detect_draft_pattern` は `DraftVersion::Draft07` を返す**: Safari が送る `0xc671706a` を優先して判定する (draft-14 優先で判定すると応答 SETTINGS を組み立てる際に問題が出ることが実測で確認されているため)。判定順は `draft-15 → draft-07 → draft-14 → draft-02`。
2. **`Settings::requires_initial_capsule_flow_control_compat()` で draft-13/14 互換カプセル送出を判定する**: draft-07 として検出した場合でも `WT_INITIAL_MAX_*` が peer から来ていれば `true` を返し、セッション確立直後に `WT_MAX_STREAMS` / `WT_MAX_DATA` カプセルを pending キューに積む。

関連コード:

- `src/webtransport/settings.rs`: `detect_draft_pattern`, `requires_initial_capsule_flow_control_compat`
- `src/webtransport/connect.rs`: `DraftVersion::build_server_settings` (`Draft14` でも `WT_INITIAL_MAX_*` を含めない)
- `src/connection/mod.rs`: `WtSession::initialize_flow_control` (初期カプセル pending 積み)

### 参考: カプセル送出の実装例

```rust
use shiguredo_http3::webtransport::capsule::Capsule;

let capsules = [
    Capsule::MaxStreams { bidirectional: true, maximum: 100 },
    Capsule::MaxStreams { bidirectional: false, maximum: 100 },
    Capsule::MaxData { maximum: 8 * 1024 * 1024 },
];

let mut buf = Vec::new();
for capsule in &capsules {
    let mut capsule_bytes = Vec::new();
    capsule.encode(&mut capsule_bytes);
    // H3 DATA フレームで包む (各カプセルを個別に)
    buf.push(0x00); // DATA frame type
    // varint encode capsule_bytes.len()
    // buf.extend_from_slice(&capsule_bytes);
}
connect_send.send(Bytes::from(buf)).await?;
```

### 将来の見通し

Safari が `:protocol: webtransport-h3` と `SETTINGS_WT_ENABLED` を送るように更新されれば、この互換レイヤ自体が不要になる。その時点で `requires_initial_capsule_flow_control_compat` と `Draft07` 優先判定を撤去し、素直な draft-15 対応に戻せる。

## Datagram の動作

Safari での WebTransport Datagram は **http3-rs 側では問題なく動作する**。一時 Safari 上で Datagram が受信できない事象があったが、原因は W3C WebTransport API (ブラウザの JavaScript API) 側の扱いの問題であり、H3 / QUIC レイヤーの実装互換とは無関係だった。http3-rs としての対応は不要。

## Safari の WebTransport 実装構造

Safari の WebTransport は Apple の **Network.framework** (`nw_*` API) 経由で実装されている。H3 / QUIC のプロトコル処理は WebKit 自身ではなく macOS の Network.framework (クローズドソース) 内で行われる。WebKit はカプセル送受信やストリーム管理を Network.framework に委譲する。

このため Safari 固有の挙動 (非対称な応答 SETTINGS 制約、DATA フレームで包まないとクラッシュ等) はクローズドソース側の実装に起因しており、WebKit のソースを読んでも原因特定ができない。

## 参考: Chrome の draft

Chrome は **draft-02** を使用する (`SETTINGS_ENABLE_WEBTRANSPORT` (`0x2b603742`) = 1)。

## 参照

- `refs/webtrans/draft-ietf-webtrans-http3-02.txt`
- `refs/webtrans/draft-ietf-webtrans-http3-07.txt`
- `refs/webtrans/draft-ietf-webtrans-http3-13.txt`
- `refs/webtrans/draft-ietf-webtrans-http3-14.txt`
- `refs/webtrans/draft-ietf-webtrans-http3-15.txt`
- RFC 9114 (HTTP/3)
- RFC 9220 (WebSocket over HTTP/3)
- RFC 9297 (HTTP Datagrams)
- RFC 8441 (Extended CONNECT)
