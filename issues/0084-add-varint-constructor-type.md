# 0084: VarInt 補助型を導入し RFC 9000 §16 の値域を型で表現する

Created: 2026-05-23
Model: Opus 4.7

## 概要

HTTP/3 と QUIC で使われる可変長整数 (VarInt, RFC 9000 §16) の値域
`0..=2^62 - 1` を、現状 `u64` 直渡しで扱っている箇所を補助型 `VarInt` に置き換える。

リテラル定数向けに `const fn VarInt::from_static` を提供し、2^62 を超えるリテラルを
**コンパイル時に検出** できるようにする。ランタイム値向けには
`VarInt::new(u64) -> Result<Self, VarIntError>` を提供する。

本 issue は VarInt 補助型の定義と `src/varint.rs` の全公開 API のシグネチャ変更のみを扱う。
各構築時検査型 (Settings / Frame 等) への適用は後続 issue (0086 / 0087) で扱う。

注意: QPACK 内部で使われる整数符号化は RFC 7541 §5.1 のプレフィックス付き整数であり、
RFC 9000 §16 の QUIC VarInt とは方式が異なる。QPACK の `encode_integer` / `decode_integer`
は本 issue の対象外である。

## 背景

HTTP/3 のあらゆるフレーム / 設定 / ストリームタイプ ID は QUIC の VarInt 形式で
エンコードされ、`u64` で取り回されている:

- `src/varint.rs`: `encode` / `decode` / `encode_into_vec` は引数に `u64` を受ける
- `src/frame/mod.rs`: `GoawayPayload.id: u64` / `Frame::MaxPushId(u64)` /
  `Unknown { frame_type: u64, payload: Vec<u8> }`
- `src/frame/decoder.rs`: `FrameHeader { frame_type: u64, payload_len: u64, header_len: usize }`
- `src/frame/encoder.rs`: `encode_frame_header(buf, frame_type: u64, payload_len: u64)`
- `src/connection/mod.rs`: WebTransport ストリーム分類、カプセルエンコード等で `varint::decode` /
  `varint::encode_into_vec` を直接使用
- `src/webtransport/`: `datagram.rs` / `capsule.rs` / `stream.rs` で `varint::decode` /
  `varint::encode_into_vec` を直接使用

問題:

- `GoawayPayload::new(1 << 62)` のように VarInt 範囲外 (`>= 2^62`) の値を構築可能
- encoder 経路で範囲外値を渡された場合、`EncodeError::ValueTooLarge` が返るが、
  値の保証が呼び出し側に委ねられている
- リテラル定数で書かれる値 (例: WebTransport SETTINGS の ID `0x2b603742`) が
  「 VarInt として正しい範囲か」をコンパイル時に保証できない

## 根拠

- RFC 9000 §16: "The QUIC variable-length integer encoding ... allows up to
  62 bits of representation"
- RFC 9000 §16 Table 4: 8/16/32/64 bit プレフィックスで `2^6 - 1` / `2^14 - 1` /
  `2^30 - 1` / `2^62 - 1` まで表現可能
- RFC 9000 §16: Frame Type フィールドに対しては最小バイト数でのエンコードが MUST で
  要求されている (§12.4 参照)
- RFC 9114 §2.2: "This document uses the variable-length integer encoding from
  [QUIC-TRANSPORT]" — HTTP/3 の Frame Type / Frame Length / Setting ID /
  Setting Value は全て QUIC VarInt

## 設計

### 型定義

```rust
/// QUIC VarInt (RFC 9000 §16)
///
/// 0..=2^62 - 1 の整数を表現する。エンコード長は値域に応じて 1/2/4/8 バイト。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VarInt(u64);

impl VarInt {
    /// VarInt が表現可能な最大値 (2^62 - 1)
    pub const MAX: Self = Self((1u64 << 62) - 1);

    /// VarInt の最小値 (0)
    pub const ZERO: Self = Self(0);

    /// ランタイム値から検査つきで構築する
    pub const fn new(value: u64) -> Result<Self, VarIntError>;

    /// 静的値から検査つきで構築する (const fn)
    ///
    /// `value > VarInt::MAX.get()` の場合、コンパイル時 panic (= コンパイルエラー) になる。
    #[track_caller]
    pub const fn from_static(value: u64) -> Self;

    /// 内部値を取得する
    pub const fn get(self) -> u64;

    /// wire 表現のエンコード長 (1/2/4/8 バイト) を返す
    pub const fn encoded_len(self) -> usize;
}

impl core::fmt::Display for VarInt { ... }
impl From<u8> for VarInt { ... }    // 常に成功 (2^8 - 1 < 2^62 - 1)
impl From<u16> for VarInt { ... }   // 常に成功
impl From<u32> for VarInt { ... }   // 常に成功
impl TryFrom<u64> for VarInt { ... } // VarInt::new と等価
impl TryFrom<usize> for VarInt { ... } // 64bit 環境では u64 と同等
```

### エラー型

```rust
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarIntError {
    /// 値が VarInt の最大値 (2^62 - 1) を超えている
    OutOfRange { value: u64 },
}

impl core::fmt::Display for VarIntError { ... }
impl std::error::Error for VarIntError {}
```

### `const fn` の実装

```rust
impl VarInt {
    pub const fn new(value: u64) -> Result<Self, VarIntError> {
        if value > Self::MAX.get() {
            Err(VarIntError::OutOfRange { value })
        } else {
            Ok(Self(value))
        }
    }

    #[track_caller]
    pub const fn from_static(value: u64) -> Self {
        // const eval が panic を含むコードを評価するとコンパイルエラーになるため、
        // 利用者から見れば「不正リテラル = コンパイル不能」になる
        assert!(value <= Self::MAX.get(), "VarInt value must be <= 2^62 - 1");
        Self(value)
    }
}
```

MSRV 1.88 では `const Try` (`?`) は未安定。`const fn new` 内で `if` での分岐は可能だが、
`from_static` の panic 化は `assert!` で実現する (`const_panic` は 1.57 で安定済み)。

### encoded_len の実装

```rust
impl VarInt {
    pub const fn encoded_len(self) -> usize {
        // RFC 9000 §16 Table 4 に基づく値域と prefix の対応:
        //   0 ..=       63 (2^6  - 1) → 1 byte (00 prefix)
        //  64 ..=    16383 (2^14 - 1) → 2 byte (01 prefix)
        // 16384 ..= 1073741823 (2^30 - 1) → 4 byte (10 prefix)
        // 1073741824 ..= 4611686018427387903 (2^62 - 1) → 8 byte (11 prefix)
        match self.0 {
            0..=63 => 1,
            64..=16383 => 2,
            16384..=1073741823 => 4,
            1073741824..=4611686018427387903 => 8,
            _ => unreachable!(),  // VarInt 構築時に上限を保証済み
        }
    }
}
```

`unreachable!()` は const 文脈で panic するが、`VarInt` が `new` / `from_static` 経由で
構築されている限り到達不能。

### 内部用 from_validated_parts

```rust
impl VarInt {
    /// 検証済みの u64 から検査をスキップして構築する (crate 内部専用)
    ///
    /// release ビルドでは `debug_assert!` が除去されるため、
    /// 呼び出し側が値の正当性を論理的に保証できていることが前提。
    pub(crate) const fn from_validated_parts(value: u64) -> Self {
        debug_assert!(value <= Self::MAX.get());
        Self(value)
    }
}
```

decoder 経路 (`varint::decode`) は wire 上のバイトから値を組み立てる過程で
上限を構造的に保証する (最上位 2 ビットがエンコード長を示すため、組み立て後の値は
必ず `2^62 - 1` 以下) ため、`from_validated_parts` で再検査をスキップできる。

### `src/varint.rs` 既存 API との関係

現行の公開 API (6 関数 + 1 定数):

```rust
pub const MAX_VALUE: u64 = (1 << 62) - 1;
pub fn encoded_len(value: u64) -> usize;               // 範囲外で panic
pub fn try_encoded_len(value: u64) -> Result<usize, EncodeError>;
pub fn encode(buf: &mut [u8], value: u64) -> Result<usize, EncodeError>;  // 引数順: (buf, value)
pub fn encode_into_vec(buf: &mut Vec<u8>, value: u64);  // 範囲外で panic
pub fn try_encode_into_vec(buf: &mut Vec<u8>, value: u64) -> Result<(), EncodeError>;
pub fn decode(buf: &[u8]) -> Result<(u64, usize), DecodeError>;
pub fn peek_len(buf: &[u8]) -> Option<usize>;
```

変更方針:

1. `VarInt` 導入後、`MAX_VALUE` は削除し `VarInt::MAX` に置き換える
2. `encoded_len` (フリー関数) は削除し、`VarInt::encoded_len(self)` メソッドを使用する
3. `try_encoded_len` は不要になる (値が VarInt に検証済みのため) — 削除
4. `encode` は引数型を `VarInt` に変更。`VarInt` 受取により `EncodeError::ValueTooLarge` が
   不要になるため、`EncodeError` を `BufferTooShort` のみの型に縮小する。
   引数順は `(buf, value)` を維持する (全呼び出し側の引数並び替えを避けるため)
5. `encode_into_vec` / `try_encode_into_vec` も引数型を `VarInt` に変更。
   `try_encode_into_vec` のエラー型は `BufferTooShort` を考慮しないため削除 (値検証済み)
6. `decode` は戻り値を `(VarInt, usize)` に変更。`from_validated_parts` で構築する。
   `DecodeError` は `BufferTooShort` のみの型として維持する
7. `peek_len` は値に依存しないため変更なし

変更後の公開 API:

```rust
// 変更後
pub fn encode(buf: &mut [u8], value: VarInt) -> Result<usize, EncodeError>;
pub fn encode_into_vec(buf: &mut Vec<u8>, value: VarInt);
pub fn decode(buf: &[u8]) -> Result<(VarInt, usize), DecodeError>;
pub fn peek_len(buf: &[u8]) -> Option<usize>;
```

削除する API:

```rust
pub const MAX_VALUE: u64;                             // → VarInt::MAX に置換
pub fn encoded_len(value: u64) -> usize;              // → VarInt::encoded_len(self)
pub fn try_encoded_len(value: u64) -> Result<usize, EncodeError>;  // VarInt により不要
pub fn try_encode_into_vec(buf: &mut Vec<u8>, value: u64) -> Result<(), EncodeError>;  // VarInt により不要
```

エラー型の変更:

```rust
// EncodeError: ValueTooLarge を削除、BufferTooShort のみ残す
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodeError {
    BufferTooShort,
}

// DecodeError: 変更なし
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    BufferTooShort,
}
```

## スコープ

本 issue で実施する:

- `src/varint.rs` に `VarInt` 構造体 / `VarIntError` 追加
- `src/varint.rs` の全公開 API (`encode`, `decode`, `encode_into_vec`, `peek_len`) の
  シグネチャを `VarInt` を取り扱う形に変更
- `src/varint.rs` の `MAX_VALUE` / `encoded_len` / `try_encoded_len` / `try_encode_into_vec` の
  削除 (VarInt 型により不要化)
- `src/varint.rs` の `EncodeError` から `ValueTooLarge` を削除
- `src/lib.rs` に `VarInt` / `VarIntError` の `pub use` 追加
- `src/frame/decoder.rs`: `varint::decode` の戻り値型変更に追従 (`.get()` で `u64` に戻す)
- `src/frame/encoder.rs`: `varint::encode` の引数が `VarInt` になるため、
  `VarInt::new()?` で変換 (範囲外の場合は `None` を返す)。
  `varint::encoded_len(u64)` → `VarInt::encoded_len()` に置換。
  `encode_frame_header` / `encoded_frame_len` / `encoded_settings_payload_len` /
  `encode_settings_frame` / `encode_goaway_frame` / `encode_max_push_id_frame` の
  全呼び出し箇所で `u64` → `VarInt` 変換が必要
- `src/connection/mod.rs`: `varint::decode` / `varint::encode_into_vec` /
  `varint::encoded_len` のシグネチャ変更に追従。`decode` の戻り値が `VarInt` になるため、
  `stream_type` / `session_id` 等の判定で `.get()` を追加 (`match stream_type { 0x00 => ... }`
  → `match stream_type.get() { 0x00 => ... }`、`session_id & 0x03` → `session_id.get() & 0x03`
  等)。
  `DecodeError::BufferTooShort` のパターンマッチは `DecodeError` が存続するため変更不要
- `src/webtransport/datagram.rs`: `varint::decode` / `varint::encode_into_vec` の
  シグネチャ変更に追従
- `src/webtransport/capsule.rs`: `varint::decode` / `varint::encode_into_vec` /
  `varint::encoded_len` のシグネチャ変更に追従
- `src/webtransport/stream.rs`: `varint::decode` / `varint::encode_into_vec` /
  `varint::encoded_len` のシグネチャ変更に追従。
  `ClassifiedUniStream` のフィールドは `u64` のままのため、`decode` の戻り値 `VarInt` を
  `.get()` で `u64` に変換する
- `pbt/tests/prop_varint.rs`: `VarInt` 型を使うよう strategy とプロパティを修正。
  PBT は別 crate なので `from_validated_parts` (pub(crate)) は使えず、
  `VarInt::new(v).unwrap()` で値を生成する
- `pbt/tests/prop_frame.rs`: `varint::encoded_len(u64)` → `VarInt::from_static(...).encoded_len()`
  に置換
- `fuzz/fuzz_targets/fuzz_varint.rs`: `varint::decode` の戻り値型変更に追従
- `examples/wt_server/src/webtransport.rs`: `varint::decode` /
  `varint::encoded_len` のシグネチャ変更に追従
- `examples/wt_server/src/main.rs`: `varint::encode` / `varint::encoded_len` の
  シグネチャ変更に追従
- `interop/wt/src/lib.rs`: `varint::decode` / `varint::encode_into_vec` の
  シグネチャ変更に追従
- `pbt/tests/prop_webtransport.rs` / `pbt/tests/prop_datagram.rs`: `varint::encode_into_vec`
  のシグネチャ変更に追従
- `tests/test_webtransport_draft_connect.rs`: `varint::encode_into_vec` の
  シグネチャ変更に追従

本 issue では実施しない (後続 issue に委ねる):

- `GoawayPayload.id` / `MaxPushId` 等の VarInt 型適用 → issue 0087
- `Settings` 系の VarInt 型適用 → issue 0086
- `FrameHeader.length` / `FrameHeader.frame_type` の VarInt 型適用 → issue 0087
- `trybuild` による `from_static` の compile-fail テスト → issue 0088

## 影響範囲

- `src/varint.rs`: `VarInt` / `VarIntError` 追加、全公開 API シグネチャ変更、
  `MAX_VALUE` / `encoded_len` (フリー関数) / `try_encoded_len` / `try_encode_into_vec` 削除、
  `EncodeError::ValueTooLarge` 削除、既存のテストを新 API に追従
- `src/lib.rs`: `pub use varint::{VarInt, VarIntError}` 追加
- `src/frame/decoder.rs`: `varint::decode` の戻り値が `(VarInt, usize)` になるため、
  `VarInt::get()` で `u64` に戻す (Frame 型の VarInt 化は 0087 で行う)
- `src/frame/encoder.rs`: `varint::encode` の引数が `VarInt` になるため、
  `VarInt::new()?` で変換 (範囲外の場合は `None` を返す)。
  `varint::encoded_len(u64)` → `VarInt::encoded_len()` に置換。
  `encode_frame_header` をはじめとする全内部関数 (`encoded_frame_len`、
  `encoded_settings_payload_len`、`encode_settings_frame`、`encode_goaway_frame`、
  `encode_max_push_id_frame`) で変換が必要
- `src/connection/mod.rs`: `crate::varint::decode` の戻り値型変更に追従
  (`stream_type` / `session_id` 等に `.get()` 追加)。
  `DecodeError::BufferTooShort` のパターンマッチは維持 (DecodeError は存続するため)。
  `varint::encode_into_vec` / `varint::encoded_len` の引数変更に追従
- `src/webtransport/datagram.rs` / `src/webtransport/capsule.rs` /
  `src/webtransport/stream.rs`: `varint::decode` / `varint::encode_into_vec` /
  `varint::encoded_len` のシグネチャ変更に追従
- `pbt/tests/prop_varint.rs`: PBT strategy を `u64` から `VarInt` 型ベースに変更。
  ラウンドトリップ等のプロパティは `VarInt` 経由で検証するよう修正。
  PBT は別 crate なので `pub(crate) from_validated_parts` は使えず、
  `VarInt::new(v).unwrap()` で値を生成する
- `pbt/tests/prop_frame.rs`: `varint::encoded_len(u64)` → `VarInt::from_static(...).encoded_len()`
  に置換
- `fuzz/fuzz_targets/fuzz_varint.rs`: `varint::decode` の戻り値型変更に追従
- `examples/wt_server/src/webtransport.rs` / `examples/wt_server/src/main.rs`:
  `varint::decode` / `varint::encode` / `varint::encoded_len` のシグネチャ変更に追従
- `interop/wt/src/lib.rs`: `varint::decode` / `varint::encode_into_vec` の
  シグネチャ変更に追従
- `pbt/tests/prop_webtransport.rs` / `pbt/tests/prop_datagram.rs` /
  `tests/test_webtransport_draft_connect.rs`: `varint::encode_into_vec` の
  シグネチャ変更に追従

## CHANGES.md エントリ

```
- [ADD] `VarInt` / `VarIntError` を新設し、RFC 9000 §16 の値域 (0..=2^62-1) を型で表現する
  - @担当者
- [ADD] `VarInt::from_static` を `const fn` で提供し、`2^62` 以上のリテラル定数を
  コンパイル時に検出可能にする
  - @担当者
- [CHANGE] `varint` モジュールの全公開 API (`encode`, `encode_into_vec`, `decode`) の
  シグネチャを `VarInt` を扱う形に変更する
  - @担当者
- [CHANGE] `varint::MAX_VALUE` / `varint::encoded_len(u64)` /
  `varint::try_encoded_len(u64)` / `varint::try_encode_into_vec` /
  `EncodeError::ValueTooLarge` を削除する (`VarInt` 型が値域を保証するため)
  - @担当者
```

## 受け入れ条件

- `VarInt` が `0..=2^62 - 1` の値域を持つ構造体として定義されている
- `VarInt::new(u64) -> Result<Self, VarIntError>` が範囲外で `Err` を返す
- `VarInt::from_static(u64) -> Self` が `const fn` で実装され、`#[track_caller]` が
  付与されている (コンパイル時エラー検証は issue 0088 で実施)
- `VarInt::encoded_len(self) -> usize` が wire 上のエンコード長 (1/2/4/8) を返す
- `VarInt` に `Display` 実装があり、保持する数値を 10 進文字列として出力する
- `VarInt::MAX` が `Self` 型で定義されている
- `varint::encode` の引数型が `VarInt`、`varint::decode` の戻り値型が `(VarInt, usize)`
  に変更されている (引数順 `(buf, value)` は維持)
- `varint::encode_into_vec` の引数型が `VarInt` に変更されている
- `From<u8>` / `From<u16>` / `From<u32>` / `TryFrom<u64>` / `TryFrom<usize>` が
  実装されている
- `pub(crate) from_validated_parts` が `debug_assert!` 込みで実装されている
- `VarIntError` が `Display` + `std::error::Error` を実装している
- `EncodeError::ValueTooLarge` が削除されている
- `MAX_VALUE` / `encoded_len` (フリー関数) / `try_encoded_len` / `try_encode_into_vec` が
  削除されている
- `peek_len` は変更なしで維持されている
- 既存の全テスト・PBT・fuzz が通る
- 単体テストに以下が含まれている:
  - `VarInt::new` の境界値検査 (0, MAX, MAX+1)
  - `VarInt::from_static` の正常系 (0, 境界値)
  - `VarInt::get` のラウンドトリップ
  - `VarInt::encoded_len` の各サイズ境界値検証
  - `VarInt::MAX` が正しい値であることの検証
  - `VarIntError` の `Display` 出力検証
  - `VarInt::ZERO` の値検証
  - `From<u8/u16/u32>` のラウンドトリップ
  - `TryFrom<u64/usize>` の正常系 / エラーパス
  - `from_validated_parts` の正常系
- PBT (`pbt/tests/prop_varint.rs`) の strategy が `u64` から `VarInt` 型ベースに変更され、
  ラウンドトリップ等のプロパティが `VarInt` 経由で検証される

## 依存

- なし (本 issue は土台)

## 関連

- [[0086-change-settings-construct-time-validation]] (Settings 値型に VarInt を適用)
- [[0087-change-frame-construct-time-validation]] (Frame ペイロードに VarInt を適用)
- [[0088-add-trybuild-and-pbt-construct-time-validation]] (`from_static` の compile_fail テスト)
