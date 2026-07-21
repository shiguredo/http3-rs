# `connection/mod.rs` の nghttp3 行番号参照と壊れた doc コメントを整理する

- Priority: Medium
- Created: 2026-06-15
- Model: Opus 4.7
- Branch: feature/fix-connection-mod-stale-references
- Polished: 2026-07-21

## 目的

`src/connection/mod.rs` の以下のコメント/参照が陳腐化または構造的に壊れている。整理して可読性を回復する。

- L1968, 2835: nghttp3 の `lib/nghttp3_conn.c L62-71` 等の行番号参照 (上流リベースで陳腐化)
- L4365: 関数本体のない doc コメント (`/// WebTransport のネゴシエーションが完了した状態のサーバーを作成するヘルパー`) が次のテスト関数に誤帰属
- L5092: ヘルパー関数 doc が他ヘルパーの説明と混線
- L567-585: フィールド宣言順とコメントが交互に錯綜

## 優先度根拠

Medium。可読性低下・自動生成ドキュメントの誤情報・将来の改修時の判断ミス誘発。

## 現状

L1968 / L2835:

```rust
// (nghttp3 lib/nghttp3_conn.c L62-71 の TODO コメント参照)。
```

L4365:

```rust
/// WebTransport のネゴシエーションが完了した状態のサーバーを作成するヘルパー

#[test]
fn test_wt_uni_stream_open_and_data() { /* ... */ }
```

L567-585: `deferred_encoder_set_capacity` の doc コメント途中で `wt_transport_verified` のコメント本文が始まる構造破綻 (詳細はレビューレポート 6-8)

## 設計方針

- nghttp3 行番号参照 → シンボル名参照 (`nghttp3_conn_handle_remote_settings` 等関数名で示す)
- L4365 の宙ぶらりん doc コメントを削除またはヘルパー本体の直前に移動
- L5092 の doc コメント重複を整理
- L567-585 のフィールド宣言とコメントの整列を修正 (フィールド単位で完結したコメントブロックを並べる)
- カテゴリで区切ったコメントブロック (「QPACK 遅延」「WebTransport 状態」「制御ストリーム」「リクエストストリーム」「GOAWAY」) を導入

## 完了条件

- nghttp3 行番号参照がシンボル名参照に変わる
- 宙ぶらりん doc コメントが削除または整理される
- フィールド宣言ブロックの doc が再構成される
- `cargo doc` でドキュメント警告が出ない
- `make fmt && make clippy && make check` が通る

## 解決方法

ファイル順に該当箇所を整理する。Rust doc コメントの順序を守りつつカテゴリ別グループ化を施す。

### 関連ファイル

- 修正対象: `src/connection/mod.rs:567-585, 1968, 2835, 4365, 5092`
- 関連 issue: 0077 (connection/mod.rs 分割) と並行で対応すると効率的
