# 0066: feed_stream がエラー状態で InternalError に差し替え、本来のエラーが伝播しない

Created: 2026-05-14
Model: deepseek-v4-pro

## 概要

`src/connection/mod.rs:1092-1094` の `feed_stream` において、既に接続エラー状態
(`self.error.is_some()`) の場合に `ConnectionError(InternalError)` を返している。
一方 `feed_datagram` (`mod.rs:790-792`) では `self.error.clone()` で本来のエラーを返しており、
一貫性がない。

## 修正方針

`feed_datagram` と同様に `self.error.clone()` を返すように統一する。
エラー状態でさらに入力を与えられた場合、本来のエラーを伝播する方がデバッグに有用。

```rust
// 修正前 (mod.rs:1092-1094)
if self.error.is_some() {
    return Err(Error::ConnectionError(ErrorCode::InternalError));
}

// 修正後
if let Some(ref err) = self.error {
    return Err(err.clone());
}
```

## 影響範囲

- `src/connection/mod.rs:1092-1094`
- エラー種別が `InternalError` から本来のエラーに変わるため、上位層のエラーマッチングに影響する可能性がある
