# `webtransport` のエラー型に `Display` と `std::error::Error` を実装する

- Priority: Medium
- Created: 2026-06-15
- Model: Opus 4.7
- Branch: feature/add-webtransport-error-display-and-std-error
- Polished: 2026-07-21

## 目的

`src/webtransport/error.rs` の `Error`, `ErrorCode`, `CapsuleProcessError`, `DatagramError`, `StreamHeaderDecodeError`, `CapsuleDecodeError`, `CapsuleValidationError` と `connect.rs` の `ConnectError`, `CapabilityError` に `Display` / `std::error::Error` 実装が無く、`?` で連鎖した先で `format!("{e}")` や `e.source()` が使えない。ライブラリ利用者のエラーハンドリングを改善するため実装する。

## 優先度根拠

Medium。Rust のエラー型は `Display + std::error::Error` を満たすのが標準。本ライブラリを利用したアプリケーションコードでエラーログ / バックトレースを取りづらい状態。

## 現状

`src/webtransport/error.rs:138-212`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    // ...
}
// Display 実装なし
// std::error::Error 実装なし
```

`Error::description` のような独自メソッドはあるが、標準トレイトの代替にはならない。

## 設計方針

- 上記すべてのエラー型に `Display` を `#[derive(thiserror::Error)]` または手書き実装する
- 内部に他エラーを保持する variant では `#[from]` (thiserror 利用時) または `source()` を実装し、エラーチェーンを辿れるようにする
- `thiserror` 依存追加は AGENTS.md ライブラリ規約と相談 (現状の方針は手書き Display が多いため要確認)
- `Display` メッセージは英語 (AGENTS.md「エラーメッセージは全て英語にすること」)

## 完了条件

- すべての列挙エラー型に `Display` と `std::error::Error` が実装される
- エラーチェーンが `source()` で辿れる
- `format!("{e}")` が意味のあるメッセージを返す
- 既存テストがパスする
- `make fmt && make clippy && make check` が通る

## 解決方法

例 (手書き Display 実装):

```rust
impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Application { code, message } => {
                write!(f, "WebTransport application error: code={}, message={:?}", code, message)
            }
            // ...
        }
    }
}

impl core::error::Error for Error {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        None
    }
}
```

各エラー型を順次対応する。

### 関連ファイル

- 修正対象: `src/webtransport/error.rs`, `src/webtransport/connect.rs:245, 279`, `src/webtransport/capsule.rs` (`CapsuleDecodeError`, `CapsuleValidationError`)
