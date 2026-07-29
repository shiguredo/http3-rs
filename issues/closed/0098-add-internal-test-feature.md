# `internal-test` フィーチャーを Cargo.toml に追加して `from_validated_parts` を公開する

- Priority: High
- Created: 2026-06-15
- Completed: 2026-07-30
- Model: Opus 4.7
- Branch: feature/add-internal-test-feature
- Polished: 2026-07-21

## 目的

`CHANGES.md` の `## develop` セクションは `internal-test` フィーチャーの追加 (L56) と、`VarInt::from_validated_parts` / `qpack::Header::from_validated_parts` の `internal-test` 限定公開 (L34) を謳っているが、`Cargo.toml` の `[features]` セクションが空で `internal-test` が未定義であり、対象関数も `pub(crate) fn from_validated_parts_internal` のみで外部公開されていない。CHANGES.md と実装が乖離しているため、フィーチャー定義と公開 API 整備を行ってこの乖離を解消する。

## 優先度根拠

High。CHANGES.md の宣言と実装が乖離している状態は、リリース時の説明と実態の不整合を引き起こす。さらに pbt クレートから `from_validated_parts` ↔ `new` の整合性検証を行う設計 (CHANGES.md L34) が成立しておらず、構築時検査のリグレッション検知 PBT が機能していない。

## 現状

`Cargo.toml:39-40`:

```toml
[features]
# 空
```

`src/varint.rs:121` 周辺:

```rust
pub(crate) const fn from_validated_parts_internal(value: u64) -> Self {
    // ...
}
```

`src/qpack/header.rs:549` 周辺:

```rust
pub(crate) fn from_validated_parts_internal(
    name: Cow<'static, [u8]>,
    value: Cow<'static, [u8]>,
) -> Self {
    // ...
}
```

CHANGES.md L34:

> [ADD] `VarInt::from_validated_parts` を `internal-test` フィーチャー限定で公開し、PBT から `from_validated_parts` と `new` の整合性検証を可能にする (内部用は `from_validated_parts_internal` に改名)

CHANGES.md L56:

> [ADD] `internal-test` フィーチャーを追加し、PBT / fuzz / 統合テストから検査バイパス API (`Header::from_validated_parts`) を利用できるようにする (通常のアプリケーションでは有効化しない)

Makefile も `cargo test --doc --workspace --exclude nghttp3-sys --exclude ngtcp2-sys --features shiguredo_http3/internal-test` のように `internal-test` を前提にする `doc-test` ターゲット (CHANGES.md L32) を持つが、現在の Cargo.toml にはこのフィーチャーが存在しない。

## 設計方針

- `Cargo.toml` の `[features]` に `internal-test = []` を追加する
- `VarInt` / `qpack::Header` に `#[cfg(any(test, feature = "internal-test"))]` で公開ラッパー `from_validated_parts` を追加し、`from_validated_parts_internal` を呼ぶ
- `pbt/Cargo.toml` で `shiguredo_http3` 依存に `features = ["internal-test"]` を指定する
- pbt の既存 PBT (`prop_varint.rs`, `prop_qpack/main.rs` 等) が `from_validated_parts` を経由する形に揃っているか確認し、揃っていなければ追加する
- `Makefile` の `doc-test` ターゲットが現実に実行可能なことを `make doc-test` で確認する

## 完了条件

- `cargo build --features internal-test` が成功する
- `cargo test --features internal-test` が成功する
- `cargo test --doc --workspace --exclude nghttp3-sys --exclude ngtcp2-sys --features shiguredo_http3/internal-test` (Makefile の `doc-test`) が成功する
- pbt クレートの PBT から `VarInt::from_validated_parts` と `qpack::Header::from_validated_parts` が呼び出せる
- `internal-test` が無効なビルドで `from_validated_parts` がコンパイルに現れない (`pub(crate)` 内部関数は残るが外部公開されない)
- `make fmt && make clippy && make check` が全て通る

## 解決方法

1. `Cargo.toml [features]` に `internal-test = []` を追加
2. `src/varint.rs` に下記を追加:
   ```rust
   #[cfg(any(test, feature = "internal-test"))]
   pub const fn from_validated_parts(value: u64) -> Self {
       Self::from_validated_parts_internal(value)
   }
   ```
3. `src/qpack/header.rs` に下記を追加:
   ```rust
   #[cfg(any(test, feature = "internal-test"))]
   pub fn from_validated_parts(
       name: Cow<'static, [u8]>,
       value: Cow<'static, [u8]>,
   ) -> Self {
       Self::from_validated_parts_internal(name, value)
   }
   ```
4. `pbt/Cargo.toml` で `shiguredo_http3` 依存に `features = ["internal-test"]` を指定 (workspace 依存継承時の方法は `pbt/Cargo.toml` の既存記述に合わせる)
5. 既存 PBT が `from_validated_parts_internal` を呼んでいる場合は `from_validated_parts` への呼び出しに置換
6. `cargo test --features internal-test` でテスト成功を確認

### 関連ファイル

- 修正対象: `Cargo.toml`, `src/varint.rs`, `src/qpack/header.rs`, `pbt/Cargo.toml`
- 確認対象: `Makefile` (doc-test ターゲット), `pbt/tests/prop_varint.rs`, `pbt/tests/prop_qpack/main.rs`

## 解決方法

コミット f5b5260 で実装した。Cargo.toml に internal-test フィーチャーを追加し、from_validated_parts をフィーチャー gated で限定公開した。CHANGES.md と実装の乖離を解消した。
