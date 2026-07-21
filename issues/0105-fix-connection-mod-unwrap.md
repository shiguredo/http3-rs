# `connection/mod.rs` の本番コード `.unwrap()` を `.expect("MESSAGE")` に置換する

- Priority: High
- Created: 2026-06-15
- Completed:
- Model: Opus 4.7
- Branch: feature/fix-connection-mod-unwrap
- Polished:

## 目的

`src/connection/mod.rs:2641, 2656` の `self.streams.get(&stream_id).unwrap()` / `.get_mut(&stream_id).unwrap()` が AGENTS.md「`.unwrap()` ではなく `.expect("MESSAGE")` を使用する」規約に違反している。本番コードでメッセージ付き expect に置換し、不変条件を明示する。

## 優先度根拠

High。AGENTS.md の明示的な規約違反。本番経路の panic 発生時のデバッグ性に直結する。修正コストは軽微。

## 現状

`src/connection/mod.rs:2641-2660` 周辺:

```rust
self.streams
    .entry(stream_id)
    .or_insert_with(|| RequestStream::new(stream_id));
if self.streams.get(&stream_id).unwrap().is_qpack_blocked() {
    // ...
}
// ...
let stream = self.streams.get_mut(&stream_id).unwrap();
```

直前で `entry().or_insert_with(...)` を呼んでいるため `.get(&stream_id)` は確実に `Some` を返すが、コードを読む側にとって「なぜ unwrap が安全か」が明示されない。さらに `retry_blocked_streams` 等の別経路から呼ばれるケースもあり、フロー全体を読まないと安全性が確証できない。

AGENTS.md:

> `.unwrap()` ではなく `.expect("MESSAGE")` を使用する
> - `.unwrap()` では情報が少ない
> - `.expect("MESSAGE")` を使用して、最低限「このパニックが状況によっては発生する可能性がある」のか、それとも「絶対に発生しない想定（発生した場合は実装バグ）」なのかがメッセージから分かるようにすること

## 設計方針

- `.unwrap()` を `.expect("entry().or_insert_with でストリームが直前に挿入されたため必ず存在する")` のような日本語メッセージ付き expect に置換
- 直接 `entry().or_insert_with(...)` の戻り値 (`&mut RequestStream`) を変数バインドして二度引きを避ける形にもリファクタ可能 (副次的改善)
- 他の `.unwrap()` も本ファイル内で grep して同様に置換 (`src/connection/mod.rs` の `#[cfg(test)] mod tests` 内の unwrap は別 issue 0121 で扱う)

## 完了条件

- `src/connection/mod.rs` の `#[cfg(test)] mod tests` 外の `.unwrap()` がすべて `.expect("MESSAGE")` に置換される
- expect メッセージは日本語で、なぜ安全か (絶対に発生しない想定) を明示する
- `cargo test --tests -p shiguredo_http3` が全てパスする
- `make fmt && make clippy && make check` が全て通る

## 解決方法

例:

```rust
let stream = self
    .streams
    .get_mut(&stream_id)
    .expect("entry().or_insert_with で直前に挿入したため必ず存在する");
```

副次的改善案 (任意):

```rust
let stream = self
    .streams
    .entry(stream_id)
    .or_insert_with(|| RequestStream::new(stream_id));
if stream.is_qpack_blocked() {
    // ...
}
```

### 関連ファイル

- 修正対象: `src/connection/mod.rs:2641, 2656` および `#[cfg(test)] mod tests` 外の全 `.unwrap()`
- 規約: `AGENTS.md` (ルート)
- 関連 issue: 0121 (tests/pbt の `.unwrap()` 一括 expect 化)
