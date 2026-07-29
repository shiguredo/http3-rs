# `webtransport::Datagram::decode` を Result 化して不正値とバッファ不足を区別する

- Priority: Medium
- Created: 2026-06-15
- Completed: 2026-07-30
- Model: Opus 4.7
- Branch: feature/change-datagram-decode-result
- Polished: 2026-07-21

## 目的

`src/webtransport/datagram.rs:108-128` の `Datagram::decode` は `Option<(Self, usize)>` 返却で「Quarter Stream ID 不正」と「バッファ短すぎ」を区別できない。RFC 9297 Section 2.1 は Quarter Stream ID > 2^60 - 1 を `H3_DATAGRAM_ERROR` で扱う MUST と規定するため、呼び出し側が H3_DATAGRAM_ERROR を発出できる API シグネチャに変更する。

## 優先度根拠

Medium。仕様上 MUST の動作 (H3_DATAGRAM_ERROR でクローズ) を実装するための前提。`StreamHeader::decode_*_checked` は既に `StreamHeaderDecodeError` で区別しており、API スタイルの一貫性も改善する。

## 現状

```rust
impl Datagram {
    pub fn decode(buf: &[u8]) -> Option<(Self, usize)> {
        // QSI > 2^60 - 1 でも None を返す
    }
}
```

呼び出し側は `None` が「バッファ不足」か「不正 QSI」かを判断できない。

RFC 9297 Section 2.1 (`refs/webtrans/rfc9297.txt` L185-189):

> The largest legal value of the Quarter Stream ID field is 2^60-1. Receipt of an HTTP/3 Datagram that includes a larger value MUST be treated as an HTTP/3 connection error of type H3_DATAGRAM_ERROR (0x33).

## 設計方針

- `Datagram::decode` の戻り値を `Result<(Self, usize), DatagramDecodeError>` に変更
- `DatagramDecodeError` enum に `BufferTooShort` / `InvalidQuarterStreamId` を導入
- `BufferTooShort` は呼び出し側が「待つ」を選択できる non-fatal、`InvalidQuarterStreamId` は H3_DATAGRAM_ERROR で接続クローズ
- `CHANGES.md` に `[CHANGE] webtransport::Datagram::decode の戻り値型を Result に変更する` を追加

## 完了条件

- `Datagram::decode` が `Result` 返却に変わる
- `DatagramDecodeError` enum が追加される
- `Connection` 側で `InvalidQuarterStreamId` を `H3_DATAGRAM_ERROR` で接続クローズする経路を実装
- 既存テスト / PBT / fuzz がパスする
- `make fmt && make clippy && make check` が通る

## 解決方法

```rust
#[derive(Debug)]
pub enum DatagramDecodeError {
    BufferTooShort,
    InvalidQuarterStreamId,
}

impl Datagram {
    pub fn decode(buf: &[u8]) -> Result<(Self, usize), DatagramDecodeError> {
        // ...
    }
}
```

呼び出し側 (`Connection::feed_datagram`) で `InvalidQuarterStreamId` を捕捉して H3_DATAGRAM_ERROR で接続クローズ。

### 関連ファイル

- 修正対象: `src/webtransport/datagram.rs:108-128`, `src/webtransport/mod.rs`, `src/connection/mod.rs::feed_datagram`
- 一次資料: `refs/webtrans/rfc9297.txt` Section 2.1
- `CHANGES.md` 追記必要

## 解決方法

コミット f5b5260 で実装した。webtransport::Datagram::decode を Result 化し、Quarter Stream ID 不正とバッファ不足を区別できるようにした。
