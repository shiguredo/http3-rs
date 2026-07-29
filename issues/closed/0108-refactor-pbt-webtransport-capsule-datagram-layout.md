# `prop_capsule.rs` と `prop_datagram.rs` を `prop_webtransport/` 配下に統合する

- Priority: High
- Created: 2026-06-15
- Completed: 2026-07-30
- Model: Opus 4.7
- Branch: feature/refactor-pbt-webtransport-capsule-datagram-layout
- Polished: 2026-07-21

## 目的

`pbt/tests/prop_capsule.rs` と `pbt/tests/prop_datagram.rs` はトップレベル配置だが、対応する src モジュールはどちらも `src/webtransport/{capsule,datagram}.rs` のサブモジュール。AGENTS.md「`src/<module>/` のようにディレクトリモジュールの場合は `pbt/tests/prop_<module>/main.rs` にサブモジュール対応で分割すること」に違反している。`prop_webtransport/capsule.rs` および `prop_webtransport/datagram.rs` に統合してファイル配置を正す。

## 優先度根拠

High。AGENTS.md の PBT 配置規約の明示的な違反。`pbt/tests/prop_webtransport/capsule.rs` が既に存在する一方、トップレベル `prop_capsule.rs` も存在し、capsule の PBT が 2 ファイルに分散している不整合状態にある。テスト戦略の整合性のため早期に整理が必要。

## 現状

ファイル一覧:

```
pbt/tests/
├── prop_capsule.rs                          ← src/webtransport/capsule.rs に対応する位置が誤り
├── prop_datagram.rs                         ← src/webtransport/datagram.rs に対応する位置が誤り
├── prop_webtransport/
│   ├── capsule.rs                           ← こちらが正しい配置
│   ├── connect.rs
│   ├── error.rs
│   ├── session.rs
│   ├── settings.rs
│   ├── stream.rs
│   └── main.rs
├── prop_frame.rs
├── prop_settings.rs
└── ...
```

AGENTS.md:

> PBT のファイル名は `pbt/tests/prop_<module>.rs` とし、`src/<module>.rs` に対応させること
> 特定のモジュールに対応しないテストには `test_` や `prop_` プレフィックスを付けないこと
> `src/<module>/` のようにディレクトリモジュールの場合は `pbt/tests/prop_<module>/main.rs` にサブモジュール対応で分割すること

`src/capsule.rs` および `src/datagram.rs` というファイルは存在せず、それぞれ `src/webtransport/` 配下のサブモジュール。したがって正しい PBT 配置は `pbt/tests/prop_webtransport/{capsule,datagram}.rs`。

## 設計方針

- `pbt/tests/prop_capsule.rs` の内容を `pbt/tests/prop_webtransport/capsule.rs` にマージし、トップレベルファイルを削除
- `pbt/tests/prop_datagram.rs` の内容を `pbt/tests/prop_webtransport/datagram.rs` (新規) に移動し、`pbt/tests/prop_webtransport/main.rs` の `mod datagram;` を追加
- マージ時に重複する PBT (`prop_capsule.rs` と `prop_webtransport/capsule.rs` の両方に存在するテスト) を排除
- PBT の cases 数や strategy の意図差分を確認し、必要な strategy のみ統合

## 完了条件

- トップレベル `pbt/tests/prop_capsule.rs` と `pbt/tests/prop_datagram.rs` が削除される
- `pbt/tests/prop_webtransport/capsule.rs` に capsule 関連 PBT が一本化される
- `pbt/tests/prop_webtransport/datagram.rs` が新設され datagram 関連 PBT を含む
- `pbt/tests/prop_webtransport/main.rs` に `mod datagram;` が追加される
- `cargo test --workspace` で全 PBT がパスする
- `make fmt && make clippy && make check` が全て通る

## 解決方法

1. `pbt/tests/prop_capsule.rs` の PBT を `pbt/tests/prop_webtransport/capsule.rs` に追記
2. 重複している PBT を統合 (capsule_type roundtrip 等)
3. `git rm pbt/tests/prop_capsule.rs`
4. `pbt/tests/prop_datagram.rs` を `pbt/tests/prop_webtransport/datagram.rs` に移動 (`git mv`)
5. `pbt/tests/prop_webtransport/main.rs` に `mod datagram;` を追加
6. `cargo test --workspace` で確認

### 関連ファイル

- 修正元: `pbt/tests/prop_capsule.rs`, `pbt/tests/prop_datagram.rs`
- マージ先: `pbt/tests/prop_webtransport/capsule.rs`, `pbt/tests/prop_webtransport/datagram.rs` (新規)
- 関連: `pbt/tests/prop_webtransport/main.rs`
- 規約: `AGENTS.md` (テスト配置)

## 解決方法

コミット f5b5260 で実装した。prop_capsule.rs と prop_datagram.rs を prop_webtransport/ 配下に統合し、src モジュールのディレクトリ構造と PBT 配置を一致させた。
