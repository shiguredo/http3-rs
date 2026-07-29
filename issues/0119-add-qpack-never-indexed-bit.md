# QPACK の N (never-indexed) ビットを `Header` に保持して中継/再エンコードできるようにする

- Priority: Medium
- Created: 2026-06-15
- Completed: 2026-07-30
- Model: Opus 4.7
- Branch: feature/add-qpack-never-indexed-bit
- Polished: 2026-07-21

## 目的

`src/qpack/decoder.rs:185-204, 606-624` で Literal Field Line デコード時に N (never-indexed) ビットを読み捨てている。RFC 9204 Section 4.5.4 / 4.5.6 は「N=1 で受信したフィールドは literal で MUST forward」と規定するため、中継 / プロキシ実装では N ビットを保存する必要がある。`Header` 型に `never_indexed: bool` を追加し、エンコード時に N=1 を出力できる経路を提供する。

## 優先度根拠

Medium。本ライブラリを HTTP/3 中継 / プロキシ実装の基盤に使う場合に必須の機能。Sans I/O ライブラリとして「中継できない」という仕様違反を残すのは設計上の負債。

## 現状

`src/qpack/decoder.rs:185-204, 606-624`:

```rust
fn decode_literal_with_name_ref(buf: &[u8]) -> ... {
    // N ビット (bit 4 = 0x10) を読み取らず単に N=0 として扱う
}
```

`Header` 型 (`src/qpack/header.rs`) には `never_indexed` フィールドが存在しない。

RFC 9204 Section 4.5.4 (`refs/h3/rfc9204.txt`):

> When the 'N' bit is set, the encoded field line MUST always be encoded with a literal representation. In particular, when a peer sends a field line that it received represented as a literal field line with the 'N' bit set, it MUST use a literal representation to forward this field line.

## 設計方針

- `Header` 型に `never_indexed: bool` フィールドを追加 (デフォルト false)
- `decode_literal_with_name_ref` / `decode_literal_with_literal_name` で N ビットを読み取り `Header::with_never_indexed(...)` で保持
- `Encoder` で `never_indexed == true` の `Header` を出力する際は必ず Literal 表現を選択し、N=1 ビットを設定
- API 互換性のため `Header::new` の戻り値は `never_indexed: false` をデフォルトとし、`with_never_indexed(true)` ビルダーを追加
- `CHANGES.md` に `[ADD]` エントリ追加
- PBT で N ビットのラウンドトリップを検証

## 完了条件

- デコード時に N ビットが `Header::never_indexed` として保存される
- エンコード時に `never_indexed == true` の Header は Literal 表現 + N=1 で出力される
- PBT でラウンドトリップ等価性が検証される
- 既存テストがパスする
- `make fmt && make clippy && make check` が通る

## 解決方法

```rust
pub struct Header {
    // ...
    never_indexed: bool,
}

impl Header {
    pub fn with_never_indexed(mut self, never: bool) -> Self {
        self.never_indexed = never;
        self
    }
    pub fn is_never_indexed(&self) -> bool { self.never_indexed }
}
```

デコーダー内で N ビットを読み取り、エンコーダーで N ビット出力ロジックを追加。

### 関連ファイル

- 修正対象: `src/qpack/header.rs`, `src/qpack/decoder.rs:185-204, 606-624`, `src/qpack/encoder.rs` の Literal 出力
- PBT 追加: `pbt/tests/prop_qpack/main.rs`
- 一次資料: `refs/h3/rfc9204.txt` Section 4.5.4 / 4.5.6
- `CHANGES.md` 追記必要

## 解決方法

コミット f5b5260 で実装した。Header 型に never_indexed フィールドを追加し、Literal Field Line デコード時に N ビットを保持して中継時の literal 転送を実現した。
