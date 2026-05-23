# 0088: 構築時検査の trybuild compile-fail テストと PBT 整合性検証を整備する

Created: 2026-05-23
Model: Opus 4.7

## 概要

[[0084-add-varint-constructor-type]] / [[0085-change-header-construct-time-validation]] /
[[0086-change-settings-construct-time-validation]] /
[[0087-change-frame-construct-time-validation]] で導入する構築時検査について、
**4 層の経路** (`new` / `const fn from_static` / decoder / `from_validated_parts`) の
整合性を CI で恒常的に検証する仕組みを整備する。

本 issue では以下を扱う:

1. **trybuild compile-fail テスト**: `*::from_static` の不正リテラルが
   コンパイルエラーになることを CI で担保する
2. **PBT 整合性検証**: 完全性 / 健全性 / `from_static` ↔ `new` 一致 /
   `from_validated_parts` ↔ `new` 一致 を PBT (proptest) で検証する

## 背景

構築時検査と decoder 側の検査を別々に実装すると、以下の不整合が起きやすい:

- 構築 API は受け入れるが decoder が拒否する値 → ネットワーク越しの相互運用で送信できない
- 構築 API は拒否するが decoder が受け入れる値 → リモートから受信した値を中継しようとして失敗
- `from_static` (`const fn`) と `new` の検査ロジックが微妙にズレる → ローカルでは通るが
  本番で違反値を許してしまう
- `const fn` の検査ロジックを `const fn` から普通の `fn` にうっかり戻すと、コンパイル時
  検出がサイレントに消える
- `panic!` / `assert!` の文言を変更すると、利用者向けエラーメッセージが劣化する

`trybuild` は意図的にコンパイル失敗するソースのコンパイル結果 (stderr) を比較する
テストランナーで、Serde や thiserror が同様のリグレッション防止に採用している。

## 根拠

- 「コンパイル時に弾ける」が本ライブラリの差別化要素であり、この性質を CI で守らないと
  サイレントに失われる
- `compile_fail` doctest でも同等の検証はできるが、エラーメッセージの厳密な比較ができない
- `trybuild` は `dev-dependencies` 限定で本体ビルドに影響しない
- 構築時検査と decoder の検査が同じ RFC ルールを実装していることは、PBT で
  「`new` が `Ok` を返す集合 ⇔ decoder が `Ok` を返す集合」を恒常的に検証することで
  始めて保証される

## 設計

### trybuild 依存追加

```toml
# Cargo.toml
[dev-dependencies]
trybuild = "1"
```

`trybuild` は `dev-dependencies` 限定でライブラリ本体ビルドに影響しない。

### trybuild テスト配置

```
tests/
  trybuild.rs                                # ランナー
  trybuild/
    # VarInt (0084)
    varint_out_of_range.rs
    varint_out_of_range.stderr

    # Header (0085)
    header_uppercase.rs
    header_uppercase.stderr
    header_crlf_in_value.rs
    header_crlf_in_value.stderr
    header_empty_name.rs
    header_empty_name.stderr
    header_unknown_pseudo.rs
    header_unknown_pseudo.stderr

    # Setting (0086)
    setting_max_field_section_size_overflow.rs
    setting_max_field_section_size_overflow.stderr

    # Frame (0087)
    goaway_id_overflow.rs
    goaway_id_overflow.stderr
```

### ランナー実装

```rust
// tests/trybuild.rs
#[test]
fn compile_fail_construct_time_validation() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/trybuild/*.rs");
}
```

### コンパイル失敗ソースの例

```rust
// tests/trybuild/varint_out_of_range.rs
use shiguredo_http3::VarInt;

const _BAD: VarInt = VarInt::from_static(1u64 << 62);

fn main() {}
```

```rust
// tests/trybuild/header_uppercase.rs
use shiguredo_http3::qpack::Header;

const _BAD: Header = Header::from_static(b"Host", b"example.com");

fn main() {}
```

```rust
// tests/trybuild/header_crlf_in_value.rs
use shiguredo_http3::qpack::Header;

const _BAD: Header = Header::from_static(b":path", b"/foo\r\nX-Inject: 1");

fn main() {}
```

```rust
// tests/trybuild/goaway_id_overflow.rs
use shiguredo_http3::frame::GoawayPayload;

const _BAD: GoawayPayload = GoawayPayload::from_static(1u64 << 62);

fn main() {}
```

### PBT 戦略集約

`pbt/src/lib.rs` (既存) に各構築時検査型の `valid_*` / `invalid_*` 戦略を追加する。

```rust
// pbt/src/lib.rs
pub mod strategies {
    use proptest::prelude::*;
    use shiguredo_http3::*;

    /// RFC 9000 §16: 0..=2^62 - 1
    pub fn valid_varint() -> impl Strategy<Value = VarInt> {
        (0u64..=VarInt::MAX).prop_map(|v| VarInt::new(v).unwrap())
    }

    /// RFC 9114 §4.2: lowercase + token-char
    pub fn valid_field_name() -> impl Strategy<Value = Vec<u8>> { ... }

    /// RFC 9114 §4.2: CR/LF/NUL を除く field-vchar、先頭/末尾に SP/HTAB なし
    pub fn valid_field_value() -> impl Strategy<Value = Vec<u8>> { ... }

    /// RFC 9114 §4.3 / RFC 9220: 既知の疑似ヘッダー名
    pub fn valid_pseudo_header_name() -> impl Strategy<Value = Vec<u8>> {
        prop_oneof![
            Just(b":method".to_vec()),
            Just(b":scheme".to_vec()),
            Just(b":authority".to_vec()),
            Just(b":path".to_vec()),
            Just(b":status".to_vec()),
            Just(b":protocol".to_vec()),
        ]
    }

    /// RFC 9114 §7.2.4: 既知の Setting
    pub fn valid_setting() -> impl Strategy<Value = Setting> { ... }
}
```

### 検証する不変性

注: 以下のコードは検証対象のプロパティを示す **疑似コード** であり、実際の API 名とは
異なる場合がある。実装時は実際の `Encoder::encode` / `Decoder::decode` /
`SettingsPayload` / `Frame` 等のシグネチャに合わせること。

#### 1. 完全性 (`new` と decoder が同じ入力集合を受理)

```rust
// pbt/tests/prop_header.rs
proptest! {
    #[test]
    fn header_new_accepts_iff_decoder_accepts(
        name in proptest::collection::vec(any::<u8>(), 0..64),
        value in proptest::collection::vec(any::<u8>(), 0..256),
    ) {
        let via_new = Header::new(&name, &value).is_ok();
        // 同じバイト列を QPACK encoder → decoder の往復に通して受理されるか確認
        let via_decoder = /* QPACK encode → QPACK decode の流れで Header を再構築 */;
        prop_assert_eq!(via_new, via_decoder);
    }
}
```

#### 2. 健全性 (ラウンドトリップ)

```rust
// pbt/tests/prop_setting.rs
proptest! {
    #[test]
    fn setting_roundtrip(setting in valid_setting()) {
        let (id, value) = setting.as_wire();
        let parsed = Setting::from_wire(id, value).unwrap();
        prop_assert_eq!(setting, parsed);
    }
}

// pbt/tests/prop_frame.rs
proptest! {
    #[test]
    fn goaway_frame_roundtrip(id in valid_varint()) {
        let frame = Frame::Goaway(GoawayPayload::new(id));
        let mut buf = Vec::new();
        encode_frame(&frame, &mut buf).unwrap();
        let (decoded, _) = decode_frame(&buf).unwrap();
        prop_assert_eq!(frame, decoded);
    }
}
```

#### 3. `from_static` と `new` の一貫性

`const fn` で書かれた `from_static` の検査ロジックと、ランタイム検査の `new` が
同じ判定をすることを担保する。

```rust
proptest! {
    #[test]
    fn varint_static_matches_new(value in 0u64..=VarInt::MAX) {
        let via_new = VarInt::new(value).unwrap();
        let via_static = VarInt::from_static(value);
        prop_assert_eq!(via_new.get(), via_static.get());
    }
}
```

注: `Header::from_static` のテストは `&'static [u8]` を要求するため `Box::leak` で
擬似的に静的化する必要がある。PBT は数千ケース実行されるためメモリリークに注意し、
テストケース数を制限する (`proptest_config!(cases = 64)` 等)。

#### 4. `from_validated_parts` と `new` の整合性

`from_validated_parts` は `pub(crate)` のため、この検証は `src/` 内の
`#[cfg(test)] mod tests` として実装する (integration test crate の `pbt/tests/` からは
呼べない)。

```rust
// src/varint.rs 内の #[cfg(test)] mod tests
#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn validated_parts_matches_new(value in 0u64..=VarInt::MAX) {
            let via_new = VarInt::new(value).unwrap();
            let via_validated = VarInt::from_validated_parts(value);
            prop_assert_eq!(via_new, via_validated);
        }
    }
}
```

各構築時検査型 (`VarInt`, `Header`, `Setting` 系, `DataPayload`, `HeadersPayload`,
`GoawayPayload`, `FrameHeader`) で同様のテストを実装する。

### Makefile への組み込み

```makefile
# 既存 test ターゲットの一部として実行される (cargo test --workspace)
# 個別に実行したい場合のため compile-fail ターゲットを追加
compile-fail:
	cargo test --test trybuild
```

### エラーメッセージのバージョン依存性

`trybuild` の `.stderr` は rustc のバージョンに依存して微妙に変わる。
本リポジトリは `rust-toolchain.toml` で固定 (現在 1.88) しているため、CI で同じ
バージョンが使われる限り再現性がある。

複数のコンパイルバージョンでテストする必要が出た場合は `TRYBUILD=overwrite cargo test`
で `.stderr` を再生成し直す運用にする。

## 影響範囲

- `Cargo.toml`: `[dev-dependencies] trybuild = "1"` を追加
- `tests/trybuild.rs`: ランナー追加
- `tests/trybuild/*.{rs,stderr}`: 各構築時検査型のケースファイル群
- `pbt/src/lib.rs`: `strategies` モジュール追加、各構築時検査型の `valid_*` /
  `invalid_*` 戦略を定義
- `pbt/tests/prop_varint.rs` (新規 or 既存追記): VarInt の完全性・健全性・
  `from_static` 一貫性 PBT
- `pbt/tests/prop_header.rs` (新規): Header の完全性・健全性・`from_static` 一貫性 PBT
- `pbt/tests/prop_setting.rs` (新規 or 既存追記): Setting のラウンドトリップ・
  `from_static` 一貫性 PBT
- `pbt/tests/prop_frame.rs` (既存追記): 各フレーム型の PBT
- `src/varint.rs` / `src/qpack/encoder.rs` / `src/settings.rs` / `src/frame/mod.rs` 内の
  `#[cfg(test)] mod tests`: `from_validated_parts` ↔ `new` 整合性 PBT を追加
- `Makefile`: `compile-fail` ターゲットを追加

## CHANGES.md エントリ

```
- [ADD] `trybuild` による compile-fail テストを追加し、`const fn from_static` 系の
  リテラル違反検出のリグレッションを CI で防止する
  - @担当者
- [ADD] 構築時検査の完全性・健全性・from_static 一貫性・from_validated_parts 整合性を
  検証する PBT を整備する
  - @担当者
```

## 受け入れ条件

- `Cargo.toml` の `[dev-dependencies]` に `trybuild = "1"` が追加されている
- `tests/trybuild.rs` ランナーが存在する
- 全 `const fn from_static` API について少なくとも 1 件以上の compile-fail ケースが
  存在する:
  - `VarInt::from_static`: 2^62 以上
  - `Header::from_static`: 大文字 / CRLF / 空 / 不明な疑似ヘッダー
  - `GoawayPayload::from_static`: VarInt 範囲外
  (注: 0086 で `MaxFieldSectionSize` 専用ラッパー型は削除された。
  値域外検査は `VarInt::from_static` でカバーする)
- `cargo test --test trybuild` で全ケースが期待通り fail (= テストとしては成功) する
- `.stderr` の期待値が `rust-toolchain.toml` の固定バージョン (1.88) と一致している
- `pbt/src/lib.rs` に各構築時検査型の `valid_*` / `invalid_*` 戦略が定義されている
- 「`new` と decoder が同じ入力集合を受理する」プロパティが全構築時検査型で実装されている
- ラウンドトリップ (encoder → decoder → 同値) プロパティが全構築時検査型で実装されている
- `from_static` と `new` の一貫性プロパティが実装されている
- `from_validated_parts` と `new` の整合性プロパティが `#[cfg(test)]` 内で実装されている
- `Makefile` に `compile-fail` ターゲットが追加されている
- 既存の全テスト・PBT・fuzz が通る

## 依存

- [[0084-add-varint-constructor-type]]
- [[0085-change-header-construct-time-validation]]
- [[0086-change-settings-construct-time-validation]]
- [[0087-change-frame-construct-time-validation]]

## 関連

- [[0084-add-varint-constructor-type]] (VarInt の from_static)
- [[0085-change-header-construct-time-validation]] (Header の from_static)
- [[0086-change-settings-construct-time-validation]] (Setting の from_static)
- [[0087-change-frame-construct-time-validation]] (GoawayPayload の from_static)
