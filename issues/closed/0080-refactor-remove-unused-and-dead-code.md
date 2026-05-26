# 0080: 未使用コードと不要な lint 抑制を削除する

- Priority: Low
- Created: 2026-05-14
- Completed: 2026-05-26
- Polished: 2026-05-26
- Model: deepseek-v4-pro
- Branch: feature/refactor-remove-unused-and-dead-code

## 目的

未使用コード・不要な lint 抑制・重複定数を整理し、コードベースの衛生状態を改善する。

## 優先度根拠

Low: コンパイルやテストに問題はないが、死にコードの放置は CLAUDE.md の「Don't live with broken windows」に反する。

## 依存関係

- issue 0077 (connection モジュール分割) で `connection/mod.rs` の WT 定数を `wt_types.rs` に移動する計画がある。 0077 が先に実施された場合、項目 3 の定数集約先が変わる。 0080 を先に実施する場合は `webtransport/session.rs` に集約し、0077 はその結果を前提にする

## 対象項目

### 項目 1: `ReceivedData` enum の削除

- **場所**: `src/stream/request.rs:488-495`
- **理由**: `lib.rs` で re-export されておらず、`src/` / `tests/` / `pbt/` / `examples/` のいずれでも使用されていない完全な死にコード
- **対応**: 削除
- **後方互換**: `pub enum` として定義されているため `shiguredo_http3::stream::request::ReceivedData` としてパスでアクセスする外部コードが理論上存在しうるが、`lib.rs` で re-export されていないため影響は極めて小さい。 `[CHANGE]` として記録する

### 項目 2: `#[allow(dead_code)]` の削除

- **場所**: `src/connection/mod.rs:356` (`disassociate_stream` メソッド)
- **理由**: `mod.rs:3843` で `session.disassociate_stream(stream_id)` として呼び出されており dead code ではない。 `#[allow(dead_code)]` は不要
- **対応**: `#[allow(dead_code)]` 属性を単純に削除する (`#[expect(dead_code)]` への変更は不適切 — dead code でないため unfulfilled_lint_expectations 警告が出る)

### 項目 3: 重複定数の集約

- **場所**:
  - `src/connection/mod.rs:75` — `WT_MAX_BUFFERED_STREAMS: usize = 100`
  - `src/connection/mod.rs:78` — `WT_MAX_BUFFERED_DATAGRAMS: usize = 100`
  - `src/webtransport/session.rs:12` — `MAX_BUFFERED_STREAMS: usize = 100`
  - `src/webtransport/session.rs:15` — `MAX_BUFFERED_DATAGRAMS: usize = 100`
- **理由**: 同じ値 (100) で同じ意味の定数が 2 箇所に存在する。変更時に片方を修正し忘れるリスクがある
- **対応**: `webtransport/session.rs` 側の定数を `pub(crate)` に変更し、`connection/mod.rs` 側の重複定数を削除して `webtransport::session::MAX_BUFFERED_STREAMS` を参照する。 `connection/mod.rs` 側の `WT_` プレフィックス付き名前は廃止し、`session.rs` 側の名前 (`MAX_BUFFERED_STREAMS` / `MAX_BUFFERED_DATAGRAMS`) に統一する
- **注意**: `connection/mod.rs` にのみ存在する `WT_MAX_PENDING_SESSIONS` (16) と `WT_MAX_BUFFERED_STREAM_BYTES` (64 * 1024) は重複していないため対象外

## スコープ外

以下の項目は当初の issue に含まれていたが、調査の結果スコープ外とする:

- **`EncoderStream` の `encode_insert_with_name_ref` / `encode_insert_with_literal_name` / `encode_duplicate`**: `pbt/tests/prop_qpack.rs` (別クレート) から多数使用されている。RFC 9204 Section 4.3.2-4.3.4 のエンコーダ命令を送信する公開 API であり、非公開化はテストの破壊と API の意図的な機能縮小になる。 `pub` のまま維持する
- **`DecoderStream` の `encode_insert_count_increment`**: 同様に `pbt/tests/prop_qpack.rs` から使用されている。 `pub` のまま維持する
- **`EncoderStreamReceiver::buffer()` / `DecoderStreamReceiver::buffer()`**: `EncoderStreamReceiver::buffer()` は `tests/test_qpack_encoder_stream.rs` (別クレート) から 5 箇所で使用されている。 `DecoderStreamReceiver::buffer()` は現在未使用だが、API の対称性から維持する。 `pub` のまま維持する

これらは「src/ 内部から呼ばれていない」だけであり、テストや外部利用者が使用する正当な公開 API である。 `internal-test` フィーチャーは排除済み (commit `ebb023c`) であり、フィーチャーゲート付き公開への変更も方針に反する。

## テスト戦略

- `cargo test --workspace` で全テスト (tests/ + pbt/) が pass すること
- `cargo clippy --workspace` で新たな警告がないこと
- `ReceivedData` 削除後に `grep -rn 'ReceivedData'` でゼロヒットを確認

## 完了条件

- `ReceivedData` enum が削除されていること
- `disassociate_stream` の `#[allow(dead_code)]` が削除されていること
- 重複定数が `webtransport/session.rs` に集約されていること
- `cargo test --workspace` が全て pass すること
- `cargo clippy --workspace` で新たな警告がないこと

## 影響範囲

- `src/stream/request.rs`: `ReceivedData` enum の削除
- `src/connection/mod.rs`: `#[allow(dead_code)]` 削除、重複定数の削除と参照変更
- `src/webtransport/session.rs`: 定数の可視性を `pub(crate)` に変更

## 解決方法

issue に記載された 3 項目を全て対応した。

### 項目 1: `ReceivedData` enum の削除
- `src/stream/request.rs` から `pub enum ReceivedData` を削除
- `grep -rn 'ReceivedData'` でゼロヒットを確認済み

### 項目 2: `#[allow(dead_code)]` の削除
- `src/connection/mod.rs` の `disassociate_stream` メソッドから `#[allow(dead_code)]` 属性を削除
- 実際に `mod.rs:3836` で呼び出されていることを確認済み

### 項目 3: 重複定数の集約
- `src/connection/mod.rs` の `WT_MAX_BUFFERED_STREAMS` / `WT_MAX_BUFFERED_DATAGRAMS` を削除
- `src/webtransport/session.rs` の `MAX_BUFFERED_STREAMS` / `MAX_BUFFERED_DATAGRAMS` を `pub(crate)` に変更
- `connection/mod.rs` 内の参照を `crate::webtransport::session::MAX_BUFFERED_STREAMS` / `MAX_BUFFERED_DATAGRAMS` に置換

## CHANGES.md エントリ案

```markdown
- [CHANGE] stream::request::ReceivedData enum を削除する (未使用の死にコード)
  - @voluntas

### misc

- [UPDATE] disassociate_stream の不要な #[allow(dead_code)] を削除する
  - @voluntas
- [UPDATE] connection/mod.rs の重複定数を webtransport/session.rs に集約する
  - @voluntas
```
