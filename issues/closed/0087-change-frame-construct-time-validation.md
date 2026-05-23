# 0087: 各 Frame ペイロードを構築時検査型に変更する

Created: 2026-05-23
Completed: 2026-05-24
Model: Opus 4.7
Branch: feature/change-frame-construct-time-validation

## 概要

HTTP/3 フレーム (`DataPayload`, `HeadersPayload`, `SettingsPayload`,
`GoawayPayload`, `Frame::MaxPushId`) のコンストラクタを構築時検査つきに作り変える。

`VarInt` ([[0084-add-varint-constructor-type]]) と検査済み `Header`
([[0085-change-header-construct-time-validation]]) を引数型として要求することで、
構築時に検査される。各構築時検査型に `pub(crate) from_validated_parts` を統一して
導入し、decoder 経路の二重検査を避ける。

`FrameHeader.payload_len` / `FrameHeader.frame_type` の VarInt 化、`MaxPushId` /
`GoawayPayload.id` の VarInt 化もまとめて扱う。

## 背景

現状の問題:

- `GoawayPayload::new(1 << 62)` のように VarInt 範囲外 (>= 2^62) の id を構築可能
- `Frame::MaxPushId(1 << 62)` も同様 (variant ペイロードが `u64` 直)
- `HeadersPayload.encoded_field_section: Vec<u8>` が `pub` のため、構築後に
  任意のバイト列を代入可能
- `DataPayload.data: Vec<u8>` も同様に `pub`
- `SettingsPayload.entries: Vec<(u64, u64)>` は [[0086-change-settings-construct-time-validation]]
  で `Vec<Setting>` 化される
- `FrameHeader.frame_type: u64` / `FrameHeader.payload_len: u64` は wire 上 VarInt
  だが型では表現していない

これらの値は最終的に wire に encode する直前で範囲検査される。構築時に検査できれば、
利用者は `?` で早期に検出できる。

## 根拠

RFC 9114 §7.2 各フレーム定義:

- **DATA (§7.2.1)**: request stream 上のみ、データ自体に追加検査なし
- **HEADERS (§7.2.2)**: request stream 上のみ、ペイロードは QPACK 符号化済みバイト列
- **SETTINGS (§7.2.4)**: control stream 上のみ、エントリは検査済み `Setting`
- **GOAWAY (§7.2.6)**: control stream 上のみ、id (Stream ID または Push ID) は VarInt
  - サーバー送信時: id = client-initiated bidi stream ID (奇数, mod 4 == 0)
  - クライアント送信時: id = Push ID
  - 単調減少 MUST (issue 0067 で対応済み、ランタイム検査として残す)
- **MAX_PUSH_ID (§7.2.7)**: control stream 上のみ、id は VarInt、単調増加 MUST
  (サーバープッシュ非対応のため送信 API を提供しない方針だが、受信は decode する)
- **`Reserved Frame Types`**: HTTP/2 専用 (0x02, 0x06, 0x08, 0x09) は受信時
  `H3_FRAME_UNEXPECTED` (RFC 9114 §11.2.1)

RFC 9000 §16: VarInt は 0..=2^62 - 1。
RFC 9114 §7.2: フレームの Frame Type / Frame Length / GOAWAY ID / MAX_PUSH_ID ID は
全て QUIC VarInt 形式。

## 設計

### DataPayload

```rust
pub struct DataPayload {
    data: Vec<u8>,  // フィールド private 化
}

impl DataPayload {
    /// データから DATA ペイロードを構築する
    pub fn new(data: Vec<u8>) -> Self;

    pub fn data(&self) -> &[u8];
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;

    /// 検証済みバイト列から構築する (crate 内部専用)
    pub(crate) fn from_validated_parts(data: Vec<u8>) -> Self;
}
```

DATA ペイロードは RFC 上「任意のバイト列」のため検査は無いが、フィールド private 化
で構築後の改ざんを防ぐ。`const fn from_static` は内部表現が `Vec<u8>` のため
実装できず、issue 0059 (Bytes 化) 完了後に検討する。

### HeadersPayload

```rust
pub struct HeadersPayload {
    encoded_field_section: Vec<u8>,  // フィールド private 化
}

impl HeadersPayload {
    /// QPACK 符号化済みバイト列から HEADERS ペイロードを構築する
    pub fn new(encoded_field_section: Vec<u8>) -> Self;

    pub fn encoded_field_section(&self) -> &[u8];
    pub fn len(&self) -> usize;
}
```

`HeadersPayload` 自体は QPACK 符号化済みバイト列を保持するため、構築時に検査できる
内容は無い。検査は `Header::new` ([[0085]]) で field 単位に行う。
内部の `from_validated_parts` は不要 (`new` と等価のため)。

### GoawayPayload

```rust
pub struct GoawayPayload {
    id: VarInt,  // フィールド private 化、型を VarInt に変更
}

impl GoawayPayload {
    /// GOAWAY ペイロードを構築する
    pub const fn new(id: VarInt) -> Self;

    /// 静的値から GOAWAY ペイロードを構築する (const fn)
    pub const fn from_static(id: u64) -> Self;

    pub const fn id(&self) -> VarInt;

    /// 検証済みの id から構築する (crate 内部専用)
    pub(crate) const fn from_validated_parts(id: VarInt) -> Self;
}
```

id の単調減少検査 (RFC 9114 §5.2) は接続状態依存のため `Connection` 側に残す
(issue 0067 で対応済み)。

### MaxPushId (Frame variant)

```rust
pub enum Frame {
    ...
    /// MAX_PUSH_ID フレーム (RFC 9114 §7.2.7)
    MaxPushId(VarInt),
    ...
}
```

variant ペイロードを `u64` から `VarInt` に変更する。サーバープッシュ非対応のため
構築 API は decoder 専用 (`pub(crate)`) とする方針を維持する。

### SettingsPayload

[[0086-change-settings-construct-time-validation]] で扱う。本 issue では再掲しない。

### Unknown variant

```rust
pub enum Frame {
    ...
    Unknown { frame_type: VarInt, payload: Vec<u8> },
}
```

`frame_type` を `VarInt` に変更する。decoder からの構築は `from_validated_parts`
パターンを使う。

### FrameHeader

```rust
pub struct FrameHeader {
    frame_type: VarInt,        // u64 → VarInt
    payload_len: VarInt,       // u64 → VarInt
    header_len: usize,
}

impl FrameHeader {
    pub const fn frame_type(&self) -> VarInt;
    pub const fn payload_len(&self) -> VarInt;
    pub const fn header_len(&self) -> usize;
    pub const fn total_len(&self) -> usize;

    /// 検証済みの値から構築する (crate 内部専用、decoder が使用)
    pub(crate) const fn from_validated_parts(
        frame_type: VarInt,
        payload_len: VarInt,
        header_len: usize,
    ) -> Self;
}
```

フィールドを private 化し、アクセサで読み取る。decoder は `varint::decode` の
戻り値が既に `VarInt` ([[0084]]) なので、`from_validated_parts` で組み立てる。

### 検査内容のまとめ

| 検査 | 実施場所 |
|---|---|
| VarInt 範囲 (0..=2^62 - 1) | `VarInt::new` / `VarInt::from_static` (型構築時) |
| 重複 Setting 検出 | `Settings::from_payload` (リスト整合性) |
| HTTP/2 専用 ID 拒否 | `Setting::from_wire` (構築時) |
| Stream-type 別の発生位置検査 | `Connection` 側 (例: SETTINGS は control stream 上のみ) |
| GOAWAY id の単調減少 | `Connection` 側 (issue 0067) |
| MAX_PUSH_ID の単調増加 | `Connection` 側 |

### `from_validated_parts` の統一導入

decoder 経路の二重検査を避けるため、以下の型に `pub(crate) from_validated_parts` を
導入する:

- `DataPayload` (decoder が直接 Vec<u8> を渡す)
- `GoawayPayload` (VarInt 型レベルで保証済み、`debug_assert!` 不要)
- `FrameHeader` (`debug_assert!(header_len <= 16)` で不変条件チェック)

`SettingsPayload` は `Setting::from_wire` ([[0086]]) のスコープ。本 issue は Frame
ペイロード / FrameHeader のみを対象とする。

### const fn 利用例

```rust
// OK: コンパイル成功
const GOAWAY: GoawayPayload = GoawayPayload::from_static(100);

// NG: コンパイル時に "VarInt value must be <= 2^62 - 1" で fail
const BAD: GoawayPayload = GoawayPayload::from_static(1u64 << 62);
```

## 影響範囲

- `src/frame/mod.rs`:
  - `DataPayload` / `HeadersPayload` / `GoawayPayload` のフィールド private 化、
    アクセサ追加、`from_validated_parts` (GoawayPayload) 追加
  - `Frame::MaxPushId(u64)` → `Frame::MaxPushId(VarInt)`
  - `Frame::Unknown { frame_type, ... }` の型を VarInt に変更
  - `Frame::frame_type()` の戻り値型を `VarInt` に変更
  - `GoawayPayload::from_static` を追加 (const fn、VarInt 経由でコンパイル時検出)
- `src/frame/decoder.rs`:
  - `FrameHeader` のフィールド private 化、`from_validated_parts` で構築
  - `varint::decode` の戻り値を直接 `VarInt` として使う
  - `Frame::Data` / `Frame::Headers` / `Frame::Goaway` / `Frame::MaxPushId` /
    `Frame::Unknown` の構築を `from_validated_parts` 経由に変更
- `src/frame/encoder.rs`: `VarInt::get()` または `VarInt::encoded_len()` 経由で
  エンコード。アクセサ経由でフィールド読み取り。
  `encode_max_push_id_frame` はサーバープッシュ非対応で送信 API を提供しないため、
  到達不能コードとなる。削除するか `#[allow(dead_code)]` を付与する
- `src/connection/`: フレーム送受信箇所で API 追従。GOAWAY id の単調減少検査
  (issue 0067 既存) を維持。`send_goaway(id: u64)` の引数型を `VarInt` に変更。
  `peer_goaway_last_id: Option<u64>` も `Option<VarInt>` に変更。
  `payload.id` の算術演算 (`% 4` 等) に `.get()` を追加
- `src/event.rs`: `Event::GoawayReceived { id: u64 }` の `id` 型を `VarInt` に変更
- `src/lib.rs`: 必要に応じて re-export 整理
- `src/stream/control.rs`: `send_goaway(id: u64)` の引数型変更に追従
- `examples/`, `tests/`, `pbt/tests/prop_frame.rs`, `fuzz/`, `interop/`: API 追従。
  PBT では構造体リテラルでのフィールド直接構築を `new()` または
  `from_validated_parts` 経由に変更

## CHANGES.md エントリ

```
- [ADD] `GoawayPayload::from_static` を追加し、不正リテラルを
  コンパイル時に検出可能にする
  - @担当者
- [CHANGE] `DataPayload` / `HeadersPayload` / `GoawayPayload` のフィールドを
  private 化し、アクセサメソッドを提供する
  - @担当者
- [CHANGE] `GoawayPayload.id` / `Frame::MaxPushId` / `Frame::Unknown.frame_type` /
  `FrameHeader.frame_type` / `FrameHeader.payload_len` / `Frame::frame_type()` の
  型を `u64` から `VarInt` に変更し、RFC 9000 §16 の値域を型で表現する
  - @担当者
- [CHANGE] `Event::GoawayReceived.id` の型を `u64` から `VarInt` に変更する
  - @担当者
- [CHANGE] `send_goaway` 系列 API の引数型を `u64` から `VarInt` に変更する
  - @担当者
```

## 受け入れ条件

- `DataPayload` / `HeadersPayload` / `GoawayPayload` の全フィールドが private で、
  アクセサ経由でのみ読み取れる
- `GoawayPayload.id` / `Frame::MaxPushId` / `Frame::Unknown.frame_type` /
  `Frame::frame_type()` の型が `VarInt` になっている
- `FrameHeader.frame_type` / `payload_len` の型が `VarInt` で private 化され、
  アクセサが提供されている
- `GoawayPayload.from_static` が `const fn` で実装されている
- decoder が `from_validated_parts` 経由で `GoawayPayload` / `FrameHeader` を
  組み立てている
- `DataPayload` の decoder 経路は `from_validated_parts(data: Vec<u8>)` 経由
- `HeadersPayload` の decoder 経路は `new(encoded_field_section: Vec<u8>)` 経由
- `Event::GoawayReceived.id` の型が `VarInt` になっている
- `send_goaway(id: VarInt)` のシグネチャ変更が反映されている
- `encode_max_push_id_frame` が削除または dead-code 許容されている
- 既存の GOAWAY 単調減少検査 (issue 0067) が維持されている
- サーバープッシュ非対応の方針 (構築 API を提供しない) が維持されている
- 既存の全テスト・PBT・fuzz が通る

## 依存

- [[0084-add-varint-constructor-type]] (`VarInt` 補助型を使用)
- [[0085-change-header-construct-time-validation]] (`Header` 構築時検査)
- [[0086-change-settings-construct-time-validation]] (`SettingsPayload` を共有)

## 関連

- [[0067-bug-fix-goaway-monotonic-decrease]] (GOAWAY 単調減少 — 接続状態依存のためランタイム検査)
- [[0088-add-trybuild-and-pbt-construct-time-validation]] (`from_static` の compile_fail テスト)

## 解決方法

### Frame ペイロードの private 化とアクセサ追加

- `DataPayload` / `HeadersPayload` / `GoawayPayload` の全フィールドを private 化
  し、`data()` / `into_data()` / `encoded_field_section()` /
  `into_encoded_field_section()` / `id()` / `len()` / `is_empty()` のアクセサを
  提供する
- 構築後の改ざんを防止し、利用者は所有権付き取り出しを `into_*` 経由で行う

### VarInt 型化 (RFC 9000 Section 16 の値域を型レベルで担保)

- `GoawayPayload.id` / `Frame::MaxPushId` / `Frame::frame_type()` の型を
  `u64` から `VarInt` に変更
- `FrameHeader.frame_type` / `payload_len` の型を `VarInt` に変更し、
  フィールドを private 化、`frame_type()` / `payload_len()` / `header_len()`
  アクセサを提供
- `FrameHeader::total_len()` を `Option<usize>` 化し、32bit プラットフォームで
  `payload_len` が `usize` を超える silent truncation を防止
- `Event::GoawayReceived.id` / `Connection::send_goaway` /
  `ClientConnection::send_goaway` / `ServerConnection::send_goaway` の
  引数型を `VarInt` に変更
- `frame::encode_frame_header` の引数を `(buf, frame_type: VarInt, payload_len: VarInt)`
  に変更

### Frame::Unknown を newtype 化し既知タイプ偽装を防止

- `Frame::Unknown { frame_type, payload }` を `Frame::Unknown(UnknownFrame)`
  tuple variant に変更
- `UnknownFrame::new(frame_type, payload) -> Result<Self, UnknownFrameError>`
  で構築時に以下を弾く:
  - 既知の HTTP/3 フレームタイプ (RFC 9114 Section 7.2: DATA / HEADERS /
    CANCEL_PUSH / SETTINGS / PUSH_PROMISE / GOAWAY / MAX_PUSH_ID)
  - HTTP/2 専用 ID (RFC 9114 Section 11.2.1 Table 2 で Reserved 登録、
    Section 7.2.8 で受信時 H3_FRAME_UNEXPECTED: 0x02 / 0x06 / 0x08 / 0x09)
- `UnknownFrameError` を `#[non_exhaustive]` で公開、各 variant にも
  `#[non_exhaustive]` 付与
- decoder は `UnknownFrame::new(...).expect(...)` 経由で構築 (match の
  `None` arm に達するのは既知/HTTP2 専用を除外した後のため `Ok` 確実)

### GoawayPayload::from_static (const fn)

- 不正リテラルの GOAWAY ID をコンパイル時 panic として検出可能にする
- ロール依存の制約 (`4 の倍数` / push ID `0` のみ) は `Connection::send_goaway`
  のランタイム検査で別途行う

### FrameHeader::from_validated_parts と非最小 VarInt エンコード対応

- decoder で `varint::decode` の消費長 (`type_len + len_len`) を保持し
  `FrameHeader::from_validated_parts(frame_type, payload_len, header_len)` で
  wire 上の実バイト長を渡す
- RFC 9000 Section 16 は非最小 VarInt エンコードを許容する (frame type 例外は
  QUIC layer のみで HTTP/3 frame type には適用されない) ため、値からの最小長
  (`encoded_len()` の合計) と一致しない場合がある
- `debug_assert!` で下限 (最小エンコード長) と上限 (16 バイト) のみ検査

### CHANGES.md 更新

- `[ADD]` 3 件 (GoawayPayload::from_static / DataPayload・HeadersPayload
  アクセサ / UnknownFrame・UnknownFrameError)
- `[CHANGE]` 8 件 (private 化、Unknown tuple variant 化、VarInt 化、
  FrameHeader 型変更、encode_frame_header シグネチャ、GoawayReceived 型変更、
  send_goaway 引数型変更)

### 受け入れ条件との対応

- `DataPayload::from_validated_parts` は実装時に削除 (`new` と完全同一で意味なし)
- `GoawayPayload::from_validated_parts` も同様に削除 (`new(VarInt)` で型レベル保証)
- `UnknownFrame::from_validated_parts` も削除し decoder は `UnknownFrame::new(...).expect(...)`
- `encode_max_push_id_frame` は `encode_frame` の match arm から呼ばれるため残置
  (送信 API は提供しないが decode → re-encode のループバックで使用)

### テスト

- 単体テスト: `UnknownFrame::new` の既知タイプ拒否 (全 7 種類) / HTTP/2 専用拒否
  (全 4 種類) / Reserved Frame Type 受理 (0x21)
- 単体テスト: 非最小 VarInt エンコードの decoder ラウンドトリップ
- 統合テスト: GOAWAY 単調減少 (同値再送 OK / 減少 OK / 増加 NG)
- PBT: `prop_unknown_frame_preserved` を `prop_filter` で全 VarInt 範囲から
  既知 / HTTP/2 専用を除外する形に拡張
