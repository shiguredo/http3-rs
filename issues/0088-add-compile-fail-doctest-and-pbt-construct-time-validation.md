# 0088: 構築時検査の compile_fail doctest と PBT 整合性検証を整備する

Created: 2026-05-23
Model: Opus 4.7

## 概要

[[0084-add-varint-constructor-type]] / [[0085-change-header-construct-time-validation]] /
[[0086-change-settings-construct-time-validation]] /
[[0087-change-frame-construct-time-validation]] で導入する構築時検査について、
**4 層の経路** (`new` / `const fn from_static` / decoder / `from_validated_parts`) の
整合性を CI で恒常的に検証する仕組みを整備する。

本 issue では以下を扱う:

1. **`compile_fail` doctest**: `*::from_static` の不正リテラルがコンパイルエラーになる
   ことを CI (`cargo test --doc`) で担保する
2. **PBT 整合性検証**: 完全性 / 健全性 / `from_static` ↔ `new` 一致 /
   `from_validated_parts` ↔ `new` 一致 を PBT (proptest) で検証する

外部 crate (trybuild) は採用しない。理由は「## 設計判断」を参照。

## 背景

構築時検査と decoder 側の検査を別々に実装すると、以下の不整合が起きやすい:

- 構築 API は受け入れるが decoder が拒否する値 → ネットワーク越しの相互運用で送信できない
- 構築 API は拒否するが decoder が受け入れる値 → リモートから受信した値を中継しようとして失敗
- `from_static` (`const fn`) と `new` の検査ロジックが微妙にズレる → ローカルでは通るが
  本番で違反値を許してしまう
- `const fn` の検査ロジックを `const fn` から普通の `fn` にうっかり戻すと、コンパイル時
  検出がサイレントに消える

## 根拠

- 「コンパイル時に弾ける」が本ライブラリの差別化要素であり、この性質を CI で守らないと
  サイレントに失われる
- 構築時検査と decoder の検査が同じ RFC ルールを実装していることは、PBT で
  「`new` が `Ok` を返す集合 ⇔ decoder が `Ok` を返す集合」を恒常的に検証することで
  始めて保証される

## 設計判断

### trybuild を採用しない理由

trybuild は `.stderr` を完全一致比較できる強みがあるが、本プロジェクトでは採用しない:

- 依存追加 (`proc-macro2 / syn / quote / glob / termcolor` 等) の代償が大きい
- `.stderr` が rustc バージョン変更に敏感で、toolchain 上げのたびに
  `TRYBUILD=overwrite` での再生成運用が必要
- 「`const fn` の検査が消える」リグレッションは PBT の `from_static ↔ new` 整合性で
  捕まえられる
- 「コンパイル失敗そのもの」は `compile_fail` doctest で十分カバーできる

`compile_fail` doctest の限界 (失敗理由が変わっても test が通る) は許容し、エラー文言の
回帰は PBT で担保する。

## 設計

### compile_fail doctest による検証

各 `const fn from_static` API の doc コメント内に `compile_fail` ブロックを書く。
`cargo test --doc` で自動実行され、依存追加ゼロ。

例 (`src/varint.rs`):

```rust
/// コンパイル時に値域外の値を弾く例:
///
/// ```compile_fail
/// use shiguredo_http3::VarInt;
/// const _BAD: VarInt = VarInt::from_static(1u64 << 62);
/// ```
pub const fn from_static(value: u64) -> Self { ... }
```

例 (`src/qpack/header.rs` の `Header::from_static` がある場合):

```rust
/// 大文字を含む name はコンパイル時に panic する:
///
/// ```compile_fail
/// use shiguredo_http3::Header;
/// const _BAD: Header = Header::from_static(b"Host", b"example.com");
/// ```
```

例 (`src/frame/mod.rs` の `GoawayPayload::from_static`):

```rust
/// VarInt 値域外の id はコンパイル時に panic する:
///
/// ```compile_fail
/// use shiguredo_http3::GoawayPayload;
/// const _BAD: GoawayPayload = GoawayPayload::from_static(1u64 << 62);
/// ```
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
異なる場合がある。実装時は実際のシグネチャに合わせること。

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
        let via_decoder = /* QPACK encode → QPACK decode の流れで Header を再構築 */;
        prop_assert_eq!(via_new, via_decoder);
    }
}
```

#### 2. 健全性 (ラウンドトリップ)

```rust
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

各構築時検査型 (`VarInt`, `Header`, `Setting` 系, `GoawayPayload`, `FrameHeader`) で
同様のテストを実装する。

### Makefile / CI への組み込み

`compile_fail` doctest は `cargo test --doc` で実行される。Makefile の既存 `test`
ターゲットが `cargo test --workspace` を呼んでいれば追加の作業は不要。doc test を明示
する `doc-test` ターゲットを `Makefile` に追加する:

```makefile
doc-test:
	cargo test --doc --workspace
```

## 影響範囲

- `src/varint.rs`: `from_static` の doc に `compile_fail` ブロック追加、
  `#[cfg(test)] mod tests` に `from_validated_parts` ↔ `new` PBT 追加
- `src/qpack/encoder.rs` (もしくは `Header::from_static` を持つファイル):
  `compile_fail` doctest を追加、`#[cfg(test)] mod tests` に PBT 追加
- `src/settings.rs`: 該当があれば同様に追加
- `src/frame/mod.rs`: `GoawayPayload::from_static` の doc に `compile_fail` ブロック追加、
  `#[cfg(test)] mod tests` に PBT 追加
- `pbt/src/lib.rs`: `strategies` モジュール追加、各構築時検査型の `valid_*` / `invalid_*`
  戦略を定義
- `pbt/tests/prop_varint.rs` (既存追記): VarInt の完全性・健全性・`from_static` 一貫性 PBT
- `pbt/tests/prop_header.rs` (新規): Header の完全性・健全性・`from_static` 一貫性 PBT
- `pbt/tests/prop_setting.rs` (既存追記): Setting のラウンドトリップ・`from_static`
  一貫性 PBT
- `pbt/tests/prop_frame.rs` (既存追記): 各フレーム型の PBT
- `Makefile`: `doc-test` ターゲットを追加

## CHANGES.md エントリ

```
- [ADD] 構築時検査 (`*::from_static`) の `compile_fail` doctest を追加し、`const fn`
  検査がリグレッションすることを CI で防止する
  - @担当者
- [ADD] 構築時検査の完全性・健全性・`from_static` 一貫性・`from_validated_parts` 整合性を
  検証する PBT を整備する
  - @担当者
```

## 受け入れ条件

- 全 `const fn from_static` API について少なくとも 1 件以上の `compile_fail` doctest が
  存在する:
  - `VarInt::from_static`: 2^62 以上
  - `Header::from_static`: 大文字 / CRLF / 空 / 不明な疑似ヘッダー (該当 API があれば)
  - `GoawayPayload::from_static`: VarInt 範囲外
  (注: 0086 で `MaxFieldSectionSize` 専用ラッパー型は削除された。
  値域外検査は `VarInt::from_static` でカバーする)
- `cargo test --doc --workspace` で全 `compile_fail` ブロックが期待通り fail (= テスト
  としては成功) する
- `pbt/src/lib.rs` に各構築時検査型の `valid_*` / `invalid_*` 戦略が定義されている
- 「`new` と decoder が同じ入力集合を受理する」プロパティが全構築時検査型で実装されている
- ラウンドトリップ (encoder → decoder → 同値) プロパティが全構築時検査型で実装されている
- `from_static` と `new` の一貫性プロパティが実装されている
- `from_validated_parts` と `new` の整合性プロパティが `#[cfg(test)]` 内で実装されている
- `Makefile` に `doc-test` ターゲットが追加されている
- 既存の全テスト・PBT・fuzz が通る
- 外部 crate (trybuild 等) を追加していない

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
