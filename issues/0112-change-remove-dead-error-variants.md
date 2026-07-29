# `Error` / `ErrorCode` の死に variant を削除する

- Priority: Medium
- Created: 2026-06-15
- Completed: 2026-07-30
- Model: Opus 4.7
- Branch: feature/change-remove-dead-error-variants
- Polished: 2026-07-21

## 目的

`src/error.rs` の `Error::InvalidStreamId(u64)`, `Error::VarintDecode`, `ErrorCode::ConnectError = 0x10f`, `ErrorCode::VersionFallback = 0x110` が src 全域で生成箇所ゼロの死に variant になっている。API 表面積を縮小する。

## 優先度根拠

Medium。死に variant は将来の利用者の混乱を招き、`match` のアームを冗長にする。削除は API 表面積の縮小に直接貢献する。

## 現状

`src/error.rs`:

- L43 `ConnectError = 0x10f` — 生成 0 件 (`from_code` のマップにのみ存在)
- L46 `VersionFallback = 0x110` — 生成 0 件
- L140 `InvalidStreamId(u64)` — 生成 0 件 (Display 実装と PartialEq の枝のみ)
- L146 `VarintDecode` — 生成 0 件

`grep` で `Error::InvalidStreamId\|Error::VarintDecode\|ErrorCode::ConnectError\|ErrorCode::VersionFallback` を `src/` 配下で実行すると、`error.rs` 内の定義と Display 実装以外のヒットが無い。

## 設計方針

- 上記 4 variant を削除
- `from_code` 等のマッピング関数も該当エントリを削除
- `CHANGES.md` に `[CHANGE] Error::InvalidStreamId / Error::VarintDecode / ErrorCode::ConnectError / ErrorCode::VersionFallback を削除する (未使用)` を追加
- 将来 ConnectError / VersionFallback を実装する際は別 issue で追加可

## 完了条件

- 4 つの variant が `src/error.rs` から削除される
- `cargo build --workspace` および `cargo test --workspace` が成功する
- `CHANGES.md` にエントリ追加
- `make fmt && make clippy && make check` が通る

## 解決方法

`src/error.rs` から 4 variant の定義と Display 実装の枝、`from_code` 等のマッピングを削除する。

### 関連ファイル

- 修正対象: `src/error.rs`, `CHANGES.md`

## 解決方法

コミット f5b5260 で実装した。Error / ErrorCode の死に variant (InvalidStreamId、VarintDecode、ConnectError、VersionFallback) を削除し、API 表面積を縮小した。
