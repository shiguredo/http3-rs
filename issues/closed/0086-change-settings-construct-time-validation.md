# 0086: Setting / SettingsPayload を構築時検査型に変更する

Created: 2026-05-23
Completed: 2026-05-23
Model: Opus 4.7
Branch: feature/change-settings-construct-time-validation

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

注: 設計の初期案から実装段階で変更された項目は「(実装時に変更)」と併記する。
最終仕様は「## 解決方法」を参照。

- `Setting` enum が既知パラメータ + `Unknown(UnknownSetting)` variant で定義され、
  WebTransport の全 7 コードポイントをカバーしている
  (実装時に変更: `Unknown` を newtype `UnknownSetting` でラップし、フィールドを
   private 化することで HTTP/2 専用 / 予約 ID の混入を構造的に防ぐ)
- `Setting::from_wire` が既知 ID かつ値範囲内で `Ok`、HTTP/2 専用 / 予約済み /
  bool 値範囲外で `Err(SettingError)` を返す
- `SettingError` のフィールド型が `VarInt` になっている
- `SettingError` に `DuplicateId { id: VarInt }` を追加し、SETTINGS フレーム内の
  重複 ID 検出も `SettingError` で表現する
  (実装時に変更: 当初は `FrameDecodeError::DuplicateSettingsId` を予定したが、
   `SettingsPayload::add` で重複を構築時に弾くため `SettingError::DuplicateId` に統合)
- `Setting::as_wire` が wire 表現 `(VarInt, VarInt)` を返す
- `SettingsId` enum (src/settings.rs) が削除されている
- `webtransport::SettingsId` enum も `Setting` enum に統合され削除されている
- `MaxFieldSectionSize` 型は導入せず `VarInt` を直接使用している
- `FrameDecodeError::InvalidSettingsId` が削除され、`InvalidSetting(SettingError)`
  で HTTP/2 専用 / 予約 / bool 値域外 / 重複 ID の全 SETTINGS 検査エラーを伝播する
- decoder は重複 ID を `SettingsPayload::add(setting)` で弾き、エラーは
  `SettingError::DuplicateId { id }` 経由で報告する
- `SettingError` が `Display` + `std::error::Error` を実装している
- `SettingsPayload.entries` が `Vec<Setting>` 型で private 化されている
- `SettingsPayload::add` が `(setting: Setting) -> Result<(), SettingError>` の
  シグネチャになっている (重複検出を内部で行う)
  (実装時に変更: 当初は `add(setting: Setting)` (戻り値なし) を予定したが、
   構築時に重複を弾くため `Result` 化)
- `Settings::from_payload` は失敗しない (`Self` を返す)。重複検出は
  `SettingsPayload::add` 側に集約済み
- `webtransport::Settings::from_payload` が `&[Setting]` を受け取る形に変更されている
  (`pub` → `pub(crate)`、戻り値 `Result<Option<Self>, Error>` → `Option<Self>`)
- `Settings::iter()` および `webtransport::Settings::iter()` が
  `impl Iterator<Item = Setting> + '_` を返す
- `Settings::from_limits` が `Result<Self, VarIntError>` を返し、`Limits` の値が
  VarInt 範囲外でも panic しない
  (実装時に変更: 当初は `VarInt::new().unwrap()` で済ます予定だったが、堅牢性を
   優先して `Result` 化)
- `webtransport::ServerSettingsParams` のフィールド型を `u64` から `VarInt` に
  変更し、ビルダーで `expect` で panic する経路を解消する (実装時に追加)
- `Error` および `FrameDecodeError` に `#[non_exhaustive]` を付与し、将来の
  バリアント追加を後方互換に保つ (実装時に追加)
- `webtransport::Settings::flow_control_enabled` および
  `allows_multiple_sessions_with_peer` を削除する (実装時に追加、死にコード)
- 既存の全テスト・PBT・fuzz が通る
- PBT の不正値注入テスト (HTTP/2 専用 ID / bool 不正値) は単体テストと役割重複
  していたため削除し、`SettingsPayload::add` の重複検出 PBT (variant 横断) に置換

## 依存

- [[0084-add-varint-constructor-type]] (`VarInt` 補助型を使用)

## 関連

- [[0084-add-varint-constructor-type]] (`VarInt` 補助型を使用)
- [[0085-change-header-construct-time-validation]]
- [[0087-change-frame-construct-time-validation]] (SettingsPayload を共有)
- [[0088-add-trybuild-and-pbt-construct-time-validation]] (`from_static` の compile_fail テスト)

## 解決方法

### `Setting` enum / `SettingError` の新設 (`src/settings.rs`)

- `Setting` enum (`#[non_exhaustive]`) に 12 個の既知 variant と `Unknown(UnknownSetting)` を追加し、ID と型安全な値 (`VarInt` または `bool`) を一体で保持する形にした
- `UnknownSetting` 構造体を private フィールド (`id: VarInt`, `value: VarInt`) + アクセサで定義し、`Setting::Unknown` への直接構築経路を `Setting::from_wire` の検査済みパスに限定。HTTP/2 専用 ID / 予約 ID が `Unknown` 経由で混入できない不変条件を保証
- `SettingError` (`#[non_exhaustive]`) に `Http2OnlyId { id }` / `ReservedId { id }` / `InvalidBooleanValue { id, value }` / `DuplicateId { id }` を定義し、`Display` + `core::error::Error` を実装した
- `Setting::from_wire(VarInt, VarInt) -> Result<Self, SettingError>` で HTTP/2 専用 ID (0x02-0x05, RFC 9114 §7.2.4.1 + §11.2.2 Table 3) / 予約済み ID (0x00, §11.2.2 Table 3) / bool 値域外を構築時に弾く。未知 ID は `Setting::Unknown(UnknownSetting)` として受理する (RFC 9114 §7.2.4 末尾: MUST ignore)
- `Setting::as_wire(self) -> (VarInt, VarInt)` と `Setting::id(self) -> VarInt` を提供。bool 値 → VarInt 変換は `as_wire` 内のローカル `const` (`V_ZERO`/`V_ONE`) で行うよう簡素化
- `is_http2_only_id(u64) -> bool` を `const fn` で公開し、外部からの ID 判定にも利用できるようにした

### `settings::SettingsId` / `webtransport::SettingsId` の削除

- `src/settings.rs` の `SettingsId` enum を完全に削除し、`Setting` enum に統合
- `src/webtransport/settings.rs` の `SettingsId` enum と `is_webtransport` 関数を削除し、ID 識別を `Setting` enum の variant に置き換え
- `src/lib.rs` の `pub use settings::SettingsId` を削除して `Setting` / `SettingError` を公開
- `src/webtransport/mod.rs` の `pub use settings::SettingsId` を削除

### `Settings` / `webtransport::Settings` の VarInt 化

- 値フィールドを `Option<u64>` → `Option<VarInt>` (H3) / `u64` → `VarInt` (WT) に変更
- ビルダーメソッド (`qpack_max_table_capacity` / `max_field_section_size` / `qpack_blocked_streams` / `wt_enabled` / `wt_initial_max_*` / `wt_max_sessions_draft14` / `webtransport_max_sessions_draft07`) のシグネチャを `VarInt` 受けに変更
- `Settings::from_limits` を `Result<Self, VarIntError>` に変更し、`Limits` の値が VarInt 範囲外でも panic しないようにした
- `Settings::iter()` / `webtransport::Settings::iter()` の戻り値型を `(u64, u64)` → `Setting` に変更
- `webtransport::Settings::from_payload(&[Setting]) -> Option<Self>` を `pub(crate)` で追加し、WebTransport 関連 variant を 1 箇所でマッピングする責務分離を回復
- `webtransport::Settings::flow_control_enabled` (互換性偽装の単純委譲) と `allows_multiple_sessions_with_peer` (未使用) を削除
- `webtransport::Settings::iter()` のゼロ判定を `.get() > 0` から `!= VarInt::ZERO` に変更し VarInt 型抽象を保つ

### `SettingsPayload` のフィールド private 化 + 重複検出を構築時に集約

- `entries: Vec<(u64, u64)>` (`pub`) を削除し、`settings: Vec<Setting>` + `seen_ids: HashSet<VarInt>` (private) に置き換え
- `add(id: u64, value: u64)` → `add(setting: Setting) -> Result<(), SettingError>`。`SettingError::DuplicateId { id }` で同一フレーム内の重複 ID を構築時に弾く (RFC 9114 Section 7.2.4 MUST NOT)
- `settings()` / `len()` / `is_empty()` アクセサを追加
- `from_settings(&Settings)` は `add` 経由で組み立てるよう変更 (内部 invariant が常に `add` を通る)
- 重複検出を `SettingsPayload::add` に集約したことで、decoder 側の `seen_ids` 管理と `Settings::from_payload` 側の重複検出が冗長化していた問題を解消

### `Settings::from_payload` の整理

- `SettingsPayload` 内の各 `Setting` は構築時検査済みかつ ID 重複なしのため、`from_payload` は H3 フィールドへのマッピングと WT 設定の委譲のみに専念
- WebTransport 関連 variant は `webtransport::Settings::from_payload(&[Setting])` に委譲し、責務を 1 モジュールに集約
- `Setting::Unknown` は `from_payload` 側で無視 (MUST ignore, RFC 9114 §7.2.4 末尾)

### decoder / encoder の追従

- `src/frame/decoder.rs::decode_settings_frame` から HTTP/2 専用 ID チェック / bool 値検査 / 重複検査を全て削除し、`Setting::from_wire(id, value)?` と `SettingsPayload::add(setting)?` の組合せに集約
- `src/frame/encoder.rs::encoded_settings_payload_len` と `encode_settings_frame` を `Setting::as_wire()` 経由に書き換え。各 VarInt は `encoded_len()` を直接使えるため `VarInt::new` での再ラップ不要
- `src/error.rs::FrameDecodeError::InvalidSettingsId(u64)` を削除し、`InvalidSetting(SettingError)` 単一バリアントで HTTP/2 専用 / 予約 / bool 値域外 / 重複 ID の全ての SETTINGS 検査エラーを伝播。`From<SettingError> for FrameDecodeError` および `FrameDecodeError::source()` 実装で `SettingError` を辿れるようにした
- SETTINGS 検査エラーは `FrameDecodeError::InvalidSetting(SettingError)` 経由でのみ伝播する設計に統一。`From<SettingError> for crate::error::Error` および `Error::Settings(SettingError)` バリアントは dead code (decoder 経路は必ず `FrameDecodeError` を通る) のため新設せず、2 周目のレビューを経て初期案から削除した
- `src/error.rs::Error` および `FrameDecodeError` に `#[non_exhaustive]` を付与し、本 PR を含めた将来のバリアント追加を後方互換に保つ
- `src/stream/control.rs` の SETTINGS フレームデコードエラーの分岐を `InvalidSetting` のみで `H3_SETTINGS_ERROR` に変換する形に統一

### connection / WebTransport の追従

- `src/connection/mod.rs` 内の `Settings` フィールド参照 (`qpack_max_table_capacity` / `qpack_blocked_streams` / `max_field_section_size`) と WT 設定 (`wt_initial_max_streams_*` / `wt_initial_max_data` / `wt_enabled`) を `VarInt::get()` 経由で `u64` 取り出しに変更
- `Connection::new` 内の `Settings::from_limits(&limits)` は `Limits::default()` のフィールドが静的に VarInt 範囲内のため `.expect("Limits::default() values must fit VarInt (RFC 9000 Section 16)")` で受ける
- `src/webtransport/connect.rs` の `ServerSettingsParams` のフィールド型を `u64` から `VarInt` に変更し、`Default` 実装も `VarInt::from_static(...)` で構築。`DraftVersion::build_server_settings` / `build_client_settings` の panic 経路 (`params_varint` ヘルパー経由の `expect`) を完全に解消

### テスト整備

- `src/settings.rs` の `#[cfg(test)] mod tests` に `Setting::from_wire` / `as_wire` / 各 variant の境界 / HTTP/2 専用 / 予約 / bool 不正値 / 未知 ID (`UnknownSetting` アクセサ経由) / `SettingError::Display` / `SettingsPayload::add` の重複検出 / `Setting::Unknown` の無視 / `from_limits` の VarInt 範囲外エラーを網羅
- `src/webtransport/settings.rs` の `#[cfg(test)] mod tests` を VarInt 化し、`from_payload(&[Setting])` の WT variant 反映 / 非 WT variant 無視 / Unknown 無視 / WT エントリ無し時 `None` 返却を検証
- `src/frame/decoder.rs::tests` に `ReservedId` / `InvalidBooleanValue(0x08)` / `InvalidBooleanValue(0x33)` の decoder エラーパス単体テストを追加
- `pbt/tests/prop_settings.rs` を全面的に書き直し、Strategy で生成する VarInt 中規模域 (`0..2^30`) でラウンドトリップ / iter 合計 / `enable_webtransport_server` の自動設定 / `webtransport_draft_pattern` 一貫性を検証。`Setting::from_wire` の wire ↔ Setting ラウンドトリップは VarInt 全域 (`arbitrary_varint_full`) で叩き、`encoded_settings_payload_len` の合算境界もカバー。`SettingsPayload::add` の重複検出を PBT で検証
- HTTP/2 専用 ID / bool 不正値の PBT は単体テストと重複していたため削除 (CLAUDE.md「PBT で実現できるものを単体テストで書かない / 単体テストで十分なものを PBT に書かない」の役割分担遵守)
- `pbt/tests/prop_frame.rs::valid_settings_entries` の ID プールに WebTransport 系 ID (0x2b61 / 0x2b64 / 0x2b65 / 0x14e9cd29 / 0x2c7cf000 / 0x2b603742 / 0xc671706a) を追加し、bool 値正規化対象に draft-02 (0x2b603742) を含める
- `pbt/tests/prop_webtransport.rs` の `SettingsId` 参照を削除し、`Setting` variant ベースの比較に書き換え。`flow_control_enabled` 呼び出しを `declares_flow_control` に置換
- `fuzz/fuzz_targets/fuzz_settings.rs` を改善し、`Setting::from_wire` 拒否ケースも `Err` パスを意図的に叩いて `SettingError::Display` の panic 安全性まで検証。`Settings::from_payload` 経路への伝播も維持
- `tests/integration.rs` / `tests/test_webtransport_draft_connect.rs` を VarInt 化 (`vi(u64) -> VarInt` ヘルパー導入)

### サンプル / 相互運用クレートの追従

- `examples/wt_server` (`main.rs` / `webtransport.rs`) / `crates/tokio-s2n-quic/examples/wt_echo_*.rs` / `interop/wt/src/lib.rs` の `webtransport::Settings` ビルダー呼び出しを `VarInt::from_static` / `VarInt::new(_).expect(_)` 経由に書き換え
- `examples/wt_server/src/webtransport.rs` の SETTINGS パース箇所は `SettingsPayload::add(id, value)` から `Setting::from_wire(id, value)?` + `payload.add(setting)?` の組合せに変更

### CHANGES.md 更新

- `## develop` に `[ADD]` 3 件 (`Setting` / `UnknownSetting` / `SettingError`) と `[CHANGE]` 11 件 (`SettingsId` 削除 / VarInt 化 / `iter()` 戻り値 / `from_payload` 再設計 / `from_limits` Result 化 / `SettingsPayload` private 化 + `add` Result 化 / `ServerSettingsParams` VarInt 化 / `FrameDecodeError` 整理 / `webtransport::Settings::from_payload` の `pub` → `pub(crate)` 化 / `Error` / `FrameDecodeError` `#[non_exhaustive]` 付与 / `flow_control_enabled` / `allows_multiple_sessions_with_peer` 削除) を追記
