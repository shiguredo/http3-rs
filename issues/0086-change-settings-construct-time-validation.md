# 0086: Setting / SettingsPayload を構築時検査型に変更する

Created: 2026-05-23
Model: Opus 4.7

## 概要

`SettingsPayload` (`entries: Vec<(u64, u64)>`) と `SettingsId` (enum、ID 解決のみ) /
`Settings` (型付きフィールド構造体) の三層構造を整理し、wire 表現に直結する
`Setting` を **既知パラメータの enum + ペイロード型の値域制約** で表現し直す。

リテラル定数向けに `const fn from_static` を各制約型に提供する。
`from_wire(id, value)` で wire からの検査つきパース、`as_wire()` で wire への
エンコードを担い、`SettingsPayload` の `(u64, u64)` 直書きを廃止する。

## 背景

現状の問題:

- `SettingsPayload::add(0x06, u64::MAX)` のように `SETTINGS_MAX_FIELD_SECTION_SIZE` に
  異常値を構築可能 (推奨は 16 KiB / 上限なし、ただし VarInt 範囲 `2^62 - 1` を超える
  値はそもそも wire に乗らない)
- `SettingsPayload::add(0x08, 2)` のように `SETTINGS_ENABLE_CONNECT_PROTOCOL` が
  0/1 以外の値を構築可能 (RFC 8441 §3 違反、H3_SETTINGS_ERROR の対象)
- `SettingsPayload::add(0x33, 2)` のように `SETTINGS_H3_DATAGRAM` が 0/1 以外を構築可能
  (RFC 9297 §2.1.1 違反)
- 不正値の検査は `Settings::from_payload` 内で事後実施されており、構築点で検出できない
- 重複パラメータの検出は decoder 側の `seen_ids` でのみ実施され、
  `Settings::from_payload` には重複検査がない (RFC 9114 §7.2.4.1: H3_SETTINGS_ERROR)
- `SettingsId` enum (`src/settings.rs:11-22`) は ID 解決のみで、値の型安全性に貢献していない
- `SettingsPayload.entries: Vec<(u64, u64)>` (`src/frame/mod.rs:129`) と
  `Settings` 構造体 (`src/settings.rs:45-58`) の二重表現で、変換が
  `from_settings` / `from_payload` / `iter` に分散している
- HTTP/2 専用 ID (0x02-0x05) の拒否 (RFC 9114 §7.2.4.1: H3_SETTINGS_ERROR) は
  decoder で実施しているが、構築 API には防御層がない

## 根拠

RFC 9114 §7.2.4 / RFC 9204 §5 / RFC 8441 / RFC 9220 / RFC 9297 /
draft-ietf-webtrans-http3-15 §3.1, §4.4, §4.5:

- **`SETTINGS_QPACK_MAX_TABLE_CAPACITY` (0x01)**: VarInt 値、推奨上限は実装依存 (RFC 9204 §5)
- **`SETTINGS_MAX_FIELD_SECTION_SIZE` (0x06)**: VarInt 値、推奨は 16 KiB
- **`SETTINGS_QPACK_BLOCKED_STREAMS` (0x07)**: VarInt 値、推奨は 0
- **`SETTINGS_ENABLE_CONNECT_PROTOCOL` (0x08)**: 0 または 1 のみ、それ以外は
  PROTOCOL_ERROR (RFC 8441 §3、HTTP/3 への適用は RFC 9220)
- **`SETTINGS_H3_DATAGRAM` (0x33)**: 0 または 1 のみ、それ以外は H3_SETTINGS_ERROR
  (RFC 9297 §2.1.1)
- **HTTP/2 専用 (0x02 / 0x03 / 0x04 / 0x05)**: 受信時 H3_SETTINGS_ERROR (RFC 9114 §7.2.4.1)
- **WebTransport `SETTINGS_WT_*` (0x2b603742 / 0xc671706a / 0x2b65 等)**:
  draft-ietf-webtrans-http3-15 §3.1, §4.4, §4.5 で定義、全て VarInt 値
- **予約済みパラメータ (0x0 / 0x2 / 0x3 / 0x4 / 0x5)**: 受信時 H3_SETTINGS_ERROR
  (RFC 9114 §7.2.4.1)
- **未知のパラメータ**: 受信側は無視 MUST (RFC 9114 §7.2.4.1)
- **重複パラメータ**: 受信時 H3_SETTINGS_ERROR (RFC 9114 §7.2.4.1)

## 設計

### `Setting` enum (既存の `SettingsId` enum + `(u64, u64)` ペアを統合)

既存の `SettingsId` enum は `Setting` enum に統合され、不要になるため削除する。
`Setting` enum の各 variant が ID と型安全な値を持つ。

```rust
/// 既知の SETTINGS パラメータ (RFC 9114 §7.2.4)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Setting {
    /// QPACK 最大テーブル容量 (RFC 9204 §5, ID = 0x01)
    QpackMaxTableCapacity(VarInt),
    /// 最大ヘッダーセクションサイズ (RFC 9114 §7.2.4.2, ID = 0x06)
    MaxFieldSectionSize(VarInt),
    /// QPACK ブロックストリーム数 (RFC 9204 §5, ID = 0x07)
    QpackBlockedStreams(VarInt),
    /// CONNECT プロトコル有効化 (RFC 8441 §3, RFC 9220, ID = 0x08)
    EnableConnectProtocol(bool),
    /// H3 Datagram 有効化 (RFC 9297 §2.1.1, ID = 0x33)
    H3Datagram(bool),

    // WebTransport (draft-ietf-webtrans-http3-15)
    /// draft-15: SETTINGS_WT_ENABLED (0x2c7cf000)
    WtEnabled(VarInt),
    /// draft-14: SETTINGS_WT_MAX_SESSIONS (0x14e9cd29)
    WtMaxSessionsDraft14(VarInt),
    /// draft-02: SETTINGS_ENABLE_WEBTRANSPORT (0x2b603742)
    EnableWebTransportDraft02(bool),
    /// draft-07: SETTINGS_WEBTRANSPORT_MAX_SESSIONS (0xc671706a)
    WebTransportMaxSessionsDraft07(VarInt),
    /// WT_INITIAL_MAX_DATA (0x2b61)
    WtInitialMaxData(VarInt),
    /// WT_INITIAL_MAX_STREAMS_UNI (0x2b64)
    WtInitialMaxStreamsUni(VarInt),
    /// WT_INITIAL_MAX_STREAMS_BIDI (0x2b65)
    WtInitialMaxStreamsBidi(VarInt),

    /// 未知の SETTINGS パラメータ (RFC 9114 §7.2.4.1: MUST ignore)
    Unknown { id: VarInt, value: VarInt },
}

impl Setting {
    /// wire 上の (id, value) ペアから構築する。
    /// 既知パラメータの値が範囲外の場合は Err(SettingError) を返す。
    /// 未知の ID の場合は Ok(Setting::Unknown { id, value }) を返す
    /// (RFC 9114 §7.2.4.1: 未知パラメータは無視 MUST)。
    /// HTTP/2 専用 ID (0x02-0x05) は Err(SettingError::Http2OnlyId) を返す。
    pub fn from_wire(id: VarInt, value: VarInt) -> Result<Self, SettingError>;

    /// wire 上の (id, value) ペアに変換する
    pub fn as_wire(self) -> (VarInt, VarInt);

    /// この Setting の ID を返す
    pub fn id(self) -> VarInt;
}
```

WebTransport variant の **正確な名前と対応コードポイント** は
`src/webtransport/settings.rs` の既存 `SettingsId` enum (draft 02/07/14/15) と
完全に一致させる。

注: bool 値 (`EnableConnectProtocol`, `H3Datagram`, `WtEnabledDraft02`) は
`from_wire` で `value > 1` を `SettingError::InvalidBooleanValue` として弾く。

### 補助型 (RFC 9114 / RFC 9204 の値域)

VarInt 範囲 (0..=2^62 - 1) は [[0084-add-varint-constructor-type]] の `VarInt` 型を
そのまま使用する。各 Setting variant が値を保持するため、追加のラッパー型は導入しない。
`Setting::MaxFieldSectionSize(VarInt)` と `Setting::QpackMaxTableCapacity(VarInt)` は
variant 名で区別されるため、薄いラッパー型は不要。

### `SettingError`

```rust
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingError {
    /// HTTP/2 専用の SETTINGS ID を受信した (0x02, 0x03, 0x04, 0x05)
    /// (RFC 9114 §7.2.4.1: H3_SETTINGS_ERROR)
    Http2OnlyId { id: VarInt },

    /// 予約済み SETTINGS ID を受信した (0x00)
    /// (RFC 9114 §7.2.4.1: H3_SETTINGS_ERROR)
    /// 注: 0x02-0x05 は HTTP/2 専用のため Http2OnlyId で検出する
    ReservedId { id: VarInt },

    /// bool 値の SETTINGS が 0/1 以外の値を持つ
    /// (RFC 8441 §3, RFC 9297 §2.1.1)
    InvalidBooleanValue { id: VarInt, value: VarInt },
}
```

### `SettingsPayload` の変更

`SettingsPayload.entries` の型を `Vec<(u64, u64)>` から `Vec<Setting>` に変更する。
構築時検査つきの `Setting` を保持するため、`SettingsPayload` 全体としても不正値を
持てない。

```rust
pub struct SettingsPayload {
    settings: Vec<Setting>,  // 全フィールド private 化
}

impl SettingsPayload {
    pub fn new() -> Self;

    /// 検査済みの Setting を追加する
    pub fn add(&mut self, setting: Setting);

    pub fn settings(&self) -> &[Setting];
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;

    /// Settings (型付き構造体) から SettingsPayload を生成
    pub fn from_settings(settings: &Settings) -> Self;
}
```

`SettingsPayload::add(id: u64, value: u64)` の旧シグネチャは削除する。
利用者は `Setting::from_wire(id, value)?` で `Setting` を作ってから `add` する。

### `Settings` 構造体の整理

`Settings` 構造体は「型付きフィールドのコレクション」として維持するが、内部値の型を
新補助型に置き換える:

```rust
pub struct Settings {
    pub qpack_max_table_capacity: Option<VarInt>,
    pub max_field_section_size: Option<VarInt>,
    pub qpack_blocked_streams: Option<VarInt>,
    pub enable_connect_protocol: Option<bool>,
    pub h3_datagram: Option<bool>,
    pub wt_settings: Option<webtransport::Settings>,
}
```

ビルダーメソッドのシグネチャ:

```rust
impl Settings {
    pub fn qpack_max_table_capacity(mut self, capacity: VarInt) -> Self;
    pub fn max_field_section_size(mut self, size: VarInt) -> Self;
    pub fn qpack_blocked_streams(mut self, streams: VarInt) -> Self;
    pub fn enable_connect_protocol(mut self, enable: bool) -> Self;
    pub fn h3_datagram(mut self, enable: bool) -> Self;
}
```

`Settings::from_payload(&SettingsPayload)` は `SettingsPayload` 内の各 `Setting` が
既に検証済みのため、エラーを返す可能性がほぼなくなる。残るエラーは:

- **重複パラメータ検出** (RFC 9114 §7.2.4.1): H3_SETTINGS_ERROR
- WebTransport 側の整合性検査 (`webtransport::Settings::from_payload`)

これらは `Settings::from_payload` の責務として維持する。

### `webtransport::Settings` の扱い

`src/webtransport/settings.rs` の `SettingsId` enum も同様に `Setting` enum の
WebTransport variant に統合される。`webtransport::Settings` 構造体自体は
H3 の `Settings` と同様に「型付きコレクション」として維持し、内部値の型を補助型に
置き換える。詳細は本 issue のスコープ内で実施する。

### decoder のエラー変換

`src/frame/decoder.rs` の SETTINGS フレームパース箇所で
`Setting::from_wire(id, value)` が `Err(SettingError)` を返した場合、
`From<SettingError> for FrameDecodeError` で変換する。
`FrameDecodeError::InvalidSettingsId` は削除し、`SettingError` 経由で
`H3_SETTINGS_ERROR` に到達させる。

重複 ID の検出は decoder 側の `seen_ids: HashSet<VarInt>` で継続し、
重複時は新設する `FrameDecodeError::DuplicateSettingsId { id: VarInt }` を返す
(上位の control stream ハンドラで `H3_SETTINGS_ERROR` に変換)。

未知 ID は `Ok(Setting::Unknown { id, value })` を返すため、decoder はエラーにしない。
`Settings::from_payload` で `Setting::Unknown` を無視する (RFC 9114 §7.2.4.1: MUST ignore)。

### const fn from_static の使用例

```rust
// OK: コンパイル成功 (16384 は VarInt 範囲内)
const MFSS: Setting = Setting::MaxFieldSectionSize(
    VarInt::from_static(16384)
);

// NG: コンパイル時に "VarInt value must be <= 2^62 - 1" で fail
// (2^62 = 4611686018427387904 > VarInt::MAX = 4611686018427387903)
const BAD: Setting = Setting::QpackMaxTableCapacity(
    VarInt::from_static(1u64 << 62)
);
```

## 影響範囲

- `src/settings.rs`:
  - `SettingsId` enum を削除し、`Setting` enum に統合
  - `Setting::from_wire` / `as_wire` / `id` を実装
  - `SettingError` 追加
  - `Settings` 構造体のフィールド型を `Option<VarInt>` に置き換え
  - `from_payload`: `Vec<Setting>` からの構築に変更。`bool` 値検査は
    `Setting::from_wire` が実施済みのため削除。
    重複パラメータ検出は `from_payload` 内で新規実装
  - `from_limits`: `Limits` の `u64` フィールド → `VarInt::new().unwrap()` で変換
    (Limits 側の VarInt 化は 0084 のスコープ外のため)
  - `iter()`: 戻り値型を `impl Iterator<Item = Setting> + '_` に変更
  - `is_http2_only` free function を追加 (Setting enum に統合されてもなお
    `SettingsId::is_http2_only` の代替として必要。`Setting::from_wire` 内で呼び出し)
- `src/webtransport/settings.rs`: `SettingsId` enum を削除し `Setting` enum に統合。
  `Settings` 構造体のフィールド型を `Option<VarInt>` に置き換え。
  `from_payload` のシグネチャを `&[Setting]` 受けに変更し、
  内部の `for (id, value) in &payload.entries` を `for setting in settings`
  match に書き換え。
  `is_webtransport` free function も削除 (`Setting` enum の variant で判別可能)
- `src/frame/mod.rs`: `SettingsPayload.entries` の型を `Vec<Setting>` に変更、
  フィールド private 化、`add(id, value)` → `add(setting: Setting)` に変更、
  `from_settings` を `Setting` variant 生成ベースに書き換え
- `src/frame/decoder.rs`: SETTINGS フレームパースで `Setting::from_wire` 経由に変更。
  `FrameDecodeError::InvalidSettingsId` を削除し、
  新設 `FrameDecodeError::DuplicateSettingsId { id: VarInt }` で重複を検出
- `src/frame/encoder.rs`: `Setting::as_wire` 経由でエンコード
- `src/connection/`: SETTINGS 関連の値検査ロジックを `Setting::from_wire` に統合
- `src/lib.rs`: `Setting` / `SettingError` の `pub use` 追加、
  `SettingsId` (src/settings.rs) の `pub use` 削除
- `examples/`, `tests/`, `pbt/tests/prop_settings.rs`, `fuzz/`, `interop/`: API 追従。
  PBT では `SettingsPayload::add(id, value)` を使った不正値注入テストを
  `Setting::from_wire` の単体テストに移行

## CHANGES.md エントリ

```
- [ADD] `Setting` 系の `const fn from_static` で不正リテラルをコンパイル時に検出可能にする
  - @担当者
- [ADD] `SettingError` を追加し、SETTINGS パラメータの値域制約を型で表現する
  - @担当者
- [CHANGE] `Setting` enum を新設し、wire の (id, value) ペアを既知パラメータの enum で表現する
  - @担当者
- [CHANGE] `src/settings::SettingsId` enum と `src/webtransport::SettingsId` enum の
  両方を `Setting` enum に統合し削除する
  - @担当者
- [CHANGE] `SettingsPayload.entries` の型を `Vec<(u64, u64)>` から `Vec<Setting>` に変更し、
  フィールドを private 化する
  - @担当者
- [CHANGE] `SettingsPayload::add` のシグネチャを `(id: u64, value: u64)` から
  `(setting: Setting)` に変更する
  - @担当者
```

## 受け入れ条件

- `Setting` enum が既知パラメータ + `Unknown` variant で定義され、
  WebTransport の全 7 コードポイントをカバーしている
- `Setting::from_wire` が既知 ID かつ値範囲内で `Ok`、HTTP/2 専用 / 予約済み /
  bool 値範囲外で `Err(SettingError)` を返す
- `SettingError` のフィールド型が `VarInt` になっている
- `Setting::as_wire` が wire 表現 `(VarInt, VarInt)` を返す
- `SettingsId` enum (src/settings.rs) が削除されている
- `webtransport::SettingsId` enum も `Setting` enum に統合され削除されている
- `MaxFieldSectionSize` 型は導入せず `VarInt` を直接使用している
- `FrameDecodeError::InvalidSettingsId` が削除され、
  代わりに `DuplicateSettingsId { id: VarInt }` が追加されている
- decoder の `seen_ids` 重複チェックが維持され、`DuplicateSettingsId` を返す
- `SettingError` が `Display` + `std::error::Error` を実装している
- `SettingsPayload.entries` が `Vec<Setting>` 型で private 化されている
- `SettingsPayload::add` が `(setting: Setting)` を受け取るシグネチャになっている
- `Settings::from_payload` で重複パラメータが `H3_SETTINGS_ERROR` を返す
- `webtransport::Settings::from_payload` が `&[Setting]` を受け取る形に変更されている
- `Settings::iter()` が `impl Iterator<Item = Setting> + '_` を返す
- `Settings::from_limits` が `VarInt::new().unwrap()` 経由で変換している
- 既存の全テスト・PBT・fuzz が通る
- PBT の不正値注入テストが `Setting::from_wire` の単体テストへ移行されている

## 依存

- [[0084-add-varint-constructor-type]] (`VarInt` 補助型を使用)

## 関連

- [[0084-add-varint-constructor-type]] (`VarInt` 補助型を使用)
- [[0085-change-header-construct-time-validation]]
- [[0087-change-frame-construct-time-validation]] (SettingsPayload を共有)
- [[0088-add-trybuild-and-pbt-construct-time-validation]] (`from_static` の compile_fail テスト)
