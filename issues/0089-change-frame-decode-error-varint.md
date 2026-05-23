# 0089: FrameDecodeError の Http2Frame / ServerPushNotSupported を VarInt 化する

Created: 2026-05-24
Model: Opus 4.7

## 概要

`FrameDecodeError::Http2Frame(u64)` と `FrameDecodeError::ServerPushNotSupported(u64)` の
フィールド型を `VarInt` に変更する。

[[0087-change-frame-construct-time-validation]] で `FrameHeader::frame_type` 等を
`VarInt` 化したが、エラー型のフィールドだけが `u64` のまま残っており、
値域不整合 (`u64` の上位ビットが立った値がエラーに乗ってしまう) を型で防げない状態にある。

## 背景

`FrameDecodeError` は decoder が拒否した frame type をエラーに乗せて呼び出し元に渡す。
渡される値は wire 上の VarInt そのもので、本来 `0..=2^62 - 1` の範囲しか取り得ない。
にもかかわらず `u64` で表現しているため、以下の不整合が起きうる:

- `FrameDecodeError::Http2Frame(1u64 << 63)` のような VarInt 範囲外の値を呼び出し元が
  構築できてしまう (ライブラリ利用側の誤用)
- decoder 内部のリファクタリングで誤って raw `u64` を渡してしまっても、型で検出できない
- 同じ「HTTP/3 frame type」概念が `VarInt` と `u64` の 2 形式でコードベース内に併存し、
  呼び出し側で `.get()` / `.into()` 変換が散らばる

[[0087-change-frame-construct-time-validation]] の 3 周目レビュー指摘 I5 で挙がっていた
が、0087 のスコープ (Frame ペイロード本体の構築時検査) 外として保留した経緯がある。

## 根拠

- 型統一: `FrameHeader::frame_type` が `VarInt` である以上、それ由来のエラー値も
  `VarInt` であるべき。`u64` のまま残すと「同じ概念が 2 型」状態が固定化する
- 不正値の混入防止: `FrameDecodeError` を `pub` で公開している以上、`u64` のフィールド
  はライブラリ利用者が任意の `u64` を詰める経路を許してしまう
- 後方互換性: エラー型のバリアントフィールド型変更は MSRV 関係なく後方互換性破壊なので、
  0087 と同じ release window で固めるのが妥当

## 設計

### `FrameDecodeError` の変更

```rust
// src/error.rs (Before)
pub enum FrameDecodeError {
    // ...
    Http2Frame(u64),
    // ...
    ServerPushNotSupported(u64),
    // ...
}

// src/error.rs (After)
pub enum FrameDecodeError {
    // ...
    Http2Frame(VarInt),
    // ...
    ServerPushNotSupported(VarInt),
    // ...
}
```

`Display` 実装の `{:#x}` フォーマットは `t.get()` 経由に変更する:

```rust
Self::Http2Frame(t) => write!(f, "http/2 frame not allowed: {:#x}", t.get()),
Self::ServerPushNotSupported(t) => write!(f, "server push not supported: {:#x}", t.get()),
```

### decoder 側の生成箇所

`src/frame/decoder.rs:97` および `:141` で `frame_type.get()` を渡している箇所を
`frame_type` (`VarInt`) を直接渡す形に変更する。

```rust
// Before
return Err(FrameDecodeError::Http2Frame(frame_type.get()));

// After
return Err(FrameDecodeError::Http2Frame(frame_type));
```

`ServerPushNotSupported` 側 (`frame_type_u64` を介している) も `VarInt` を直接渡すよう
修正する。

### 呼び出し元 (match arm)

以下の `match` パターンは `_` で受けているため挙動上の変更不要だが、コメント等で
`VarInt` であることを示しておく:

- `src/stream/request.rs:287` `Err(crate::error::FrameDecodeError::Http2Frame(_))`
- `src/stream/request.rs:322` `crate::error::FrameDecodeError::ServerPushNotSupported(_)`
- `src/stream/control.rs:216` 同上
- `src/stream/control.rs:241` 同上

### テスト

`src/frame/decoder.rs` 内の `#[cfg(test)]` の以下を `VarInt` 比較に更新する:

- `assert_eq!(result, Err(FrameDecodeError::Http2Frame(0x02)));`
  → `Err(FrameDecodeError::Http2Frame(VarInt::from_static(0x02)))`
- 他 3 箇所同様

## 影響範囲

- `src/error.rs`: `FrameDecodeError::Http2Frame` / `ServerPushNotSupported` の引数型を
  `u64` → `VarInt` に変更、`Display` 実装の調整
- `src/frame/decoder.rs`: 2 箇所の生成、4 箇所の `assert_eq!` テスト更新
- `src/stream/request.rs`: match arm の型注釈確認 (動作変更なし)
- `src/stream/control.rs`: match arm の型注釈確認 (動作変更なし)
- `pbt/tests/prop_frame.rs`: 該当エラーバリアントを使う PBT があれば追従

## CHANGES.md エントリ

```
- [CHANGE] `FrameDecodeError::Http2Frame` / `ServerPushNotSupported` のフィールド型を
  `u64` から `VarInt` に変更する
  - @担当者
```

## 受け入れ条件

- `FrameDecodeError::Http2Frame` / `ServerPushNotSupported` のフィールドが `VarInt`
  になっている
- decoder からの生成は `VarInt` を直接渡す
- `Display` 実装が `t.get()` 経由で `{:#x}` 表示する
- 既存テスト・PBT・fuzz がすべて通る
- `make fmt && make clippy && make check && cargo test --tests` がすべて通る

## 依存

- [[0087-change-frame-construct-time-validation]]

## 関連

- [[0087-change-frame-construct-time-validation]] の 3 周目レビュー指摘 I5 由来
