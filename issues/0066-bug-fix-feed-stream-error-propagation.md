# 0066: feed_stream がエラー状態で InternalError に差し替え、本来のエラーが伝播しない

- Priority: Medium
- Created: 2026-05-14
- Model: deepseek-v4-pro
- Branch: feature/fix-feed-stream-error-propagation

## 目的

`src/connection/mod.rs` の `feed_stream` 関数において、接続エラー状態 (`self.error.is_some()`) の場合に `Error::ConnectionError(ErrorCode::InternalError)` を返している。一方 `feed_datagram` 関数では `self.error.clone()` で本来のエラーを返しており、同一ライブラリ内の API で一貫性がない。

`InternalError` に差し替えることで:
- 呼び出し元は本来のエラーコード（例: `ClosedCriticalStream`, `FrameError` 等）を知ることができない
- デバッグ時にエラーの原因追跡が困難になる
- `feed_datagram` と `feed_stream` で異なるエラーハンドリングが必要になる

## 優先度根拠

Medium: デバッグ容易性と API 一貫性の問題。機能が動作しなくなるわけではないが、エラー情報の損失は将来的なトラブルシュートに影響する。修正は 3 行の変更で低リスク。

## 現状

```rust
// feed_datagram (797行): 本来のエラーを返す
if let Some(ref err) = self.error {
    return Err(err.clone());
}

// feed_stream (1099行): InternalError に差し替え
if self.error.is_some() {
    return Err(Error::ConnectionError(ErrorCode::InternalError));
}
```

## 設計方針

`feed_datagram` と同様に `self.error.clone()` を返すように統一する。

```rust
// 修正後
if let Some(ref err) = self.error {
    return Err(err.clone());
}
```

## テスト戦略

単体テストで対応する。`tests/test_connection.rs`（既存ファイルまたは新規作成）に以下を追加:

- 接続エラー状態に遷移した後の `feed_stream` 呼び出しが、`InternalError` ではなく元のエラーを返すこと
- 元のエラーが `ConnectionError(FrameError)` や `ConnectionError(ClosedCriticalStream)` 等の複数パターンで正しく伝播されることを確認

## 完了条件

- `feed_stream` のエラー状態チェックが `feed_datagram` と同一パターンになっていること
- 単体テストが pass すること
- 既存テスト (`cargo test`) が全て pass すること
- CHANGES.md にエントリが追記されていること

## 後方互換性

エラー状態での `feed_stream` の戻り値が `ConnectionError(InternalError)` から本来のエラーに変わる。呼び出し元で `InternalError` にマッチしてハンドリングしているコードがある場合は影響を受ける。ただし、エラー状態で `feed_stream` を呼ぶこと自体が異常系パスであり、修正後の動作の方が正しいため `[FIX]` として記録する。

## 影響範囲

- `src/connection/mod.rs`: `feed_stream` 関数冒頭のエラーチェック（1099行付近）

## CHANGES.md エントリ案

```
- [FIX] feed_stream がエラー状態で本来のエラーではなく InternalError を返していた問題を修正する
  - @担当者
```
