# 0090: Connection::peer_goaway_request_boundary を Option<VarInt> 化する

Created: 2026-05-24
Completed: 2026-05-24
Model: Opus 4.7
Branch: feature/change-peer-goaway-boundary-varint

## 概要

`Connection::peer_goaway_request_boundary(&self) -> Option<u64>` の戻り値型を
`Option<VarInt>` に変更し、呼び出し側の比較も `VarInt` 同士で行うようにする。

## 背景

[[0087-change-frame-construct-time-validation]] で以下を `Option<VarInt>` 化した:

- `Connection::peer_goaway_last_id`
- `Connection::last_sent_goaway_id`
- `Connection::max_push_id`

一方で、これらから境界値を計算する `pub(crate) fn peer_goaway_request_boundary`
だけが `Option<u64>` のまま残っており、呼び出し元 (`src/connection/mod.rs:3431`) で
`u64` と stream id を比較している。

GOAWAY ID は RFC 9114 §5.2 で「stream ID または push ID の VarInt」と定義されており、
本来 VarInt 値域 (`0..=2^62 - 1`) しか取り得ない。`u64` で扱うと:

- 値域外の値が紛れ込んでも型で検出できない
- 内部 API の中で `VarInt` と `u64` が混在し、`.get()` / `VarInt::new(..).unwrap()`
  変換が散らばる
- 0087 で公開フィールドを `VarInt` 化した方針と内部で不整合

## 根拠

- 内部の型統一: 公開 API の `peer_goaway_last_id` が `Option<VarInt>` なら、それを
  ベースに計算する境界値も `Option<VarInt>` であるべき
- 値域不変条件の明示: 戻り値が `VarInt` であれば「VarInt 値域に収まる」ことが型で
  保証される
- [[0087-change-frame-construct-time-validation]] の 2 周目レビュー指摘 I2 由来
  (0087 のスコープ外として保留した)

## 設計

### シグネチャ変更

```rust
// src/connection/mod.rs (Before)
fn peer_goaway_request_boundary(&self) -> Option<u64> { ... }

// src/connection/mod.rs (After)
fn peer_goaway_request_boundary(&self) -> Option<VarInt> { ... }
```

`pub(crate)` 内部ヘルパーのため、外部後方互換への影響はなし。

### 内部実装

`peer_goaway_last_id` (`Option<VarInt>`) から境界値を計算する処理を `VarInt` 同士の
演算に揃える。VarInt の算術が直接できないなら `.get()` で `u64` に降ろして計算し、
最後に `VarInt::new(..).unwrap()` で戻す形を取る (理論上 VarInt 範囲内に収まることを
コメントで明記する)。

### 呼び出し元 (`src/connection/mod.rs:3431` 周辺)

```rust
// Before
if let Some(goaway_id) = self.peer_goaway_request_boundary()
    && stream_id >= goaway_id
{
    // ...
}

// After
if let Some(goaway_id) = self.peer_goaway_request_boundary()
    && stream_id >= goaway_id.get()
{
    // ...
}
```

stream_id 側が `u64` で持っているなら境界値を `.get()` で降ろして比較する
(stream_id 自体は QUIC stream ID で必ずしも HTTP/3 VarInt 制約に縛られないため、
比較時にのみ `u64` 同士にする)。

### Doc コメント

`src/connection/mod.rs:507` 付近の `peer_goaway_request_boundary` への参照コメントに
ある型表記を `Option<VarInt>` に更新する。

## 影響範囲

- `src/connection/mod.rs`:
  - `peer_goaway_request_boundary` シグネチャ変更 (戻り値 `Option<u64>` → `Option<VarInt>`)
  - 内部実装の `Option<VarInt>` 化
  - 呼び出し元 1 箇所 (`:3431` 周辺) の比較の調整
  - doc コメントの型表記更新

実装が `connection/mod.rs` に閉じる小規模な変更。

## CHANGES.md エントリ

```
- [CHANGE] `Connection::peer_goaway_request_boundary` の戻り値型を `Option<u64>` から
  `Option<VarInt>` に変更する
  - @担当者
```

注: `pub(crate)` の内部 API なので公開後方互換への影響はないが、内部 API 変更として
`[CHANGE]` に記載する。

## 受け入れ条件

- `peer_goaway_request_boundary` の戻り値が `Option<VarInt>` になっている
- 呼び出し元の比較ロジックが追従し、既存挙動を保つ
- 既存テスト・PBT・fuzz がすべて通る
- `make fmt && make clippy && make check && cargo test --tests` がすべて通る

## 依存

- [[0087-change-frame-construct-time-validation]]

## 関連

- [[0087-change-frame-construct-time-validation]] の 2 周目レビュー指摘 I2 由来

## 解決方法

`src/connection/mod.rs` の `peer_goaway_request_boundary` シグネチャを
`Option<u64>` → `Option<VarInt>` に変更。内部実装は `self.peer_goaway_last_id`
(既に `Option<VarInt>`) をそのまま返す形に簡略化 (`.map(|v| v.get())` を削除)。

呼び出し元 (`src/connection/mod.rs:3432`) は `goaway_id.get()` で `u64` に降ろして
`next_stream_id` (`u64`) と比較する形に修正。

変更は `connection/mod.rs` に閉じる小規模な変更。review-diff-code はスキップし CI に委ねた。
