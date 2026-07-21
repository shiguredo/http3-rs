# `interop/h3` と `interop/wt` の個別 `Cargo.lock` を削除する

- Priority: Low
- Created: 2026-06-15
- Model: Opus 4.7
- Branch: feature/fix-remove-interop-cargo-lock
- Polished: 2026-07-21

## 目的

`interop/h3/Cargo.lock` と `interop/wt/Cargo.lock` がリポジトリに残存している。両ディレクトリは workspace member であり、workspace ルートの `Cargo.lock` を使うべき。CHANGES.md L206 (`相互運用テスト用クレートの配置を interop_h3 / interop_wt から interop/h3 / interop/wt に移す`) で workspace 統合が行われたが、個別 lock ファイルが削除漏れになっている可能性。整理する。

## 優先度根拠

Low。動作には影響しないが、依存関係解決の不整合や CI ビルドキャッシュの混乱を招きうる。`examples/wt_server` 移行時 (CHANGES.md L210) は明示的に個別 Cargo.lock 削除をエントリとして残しており、interop も同様にすべき。

## 現状

```
interop/h3/Cargo.lock     ← 削除対象
interop/wt/Cargo.lock     ← 削除対象
Cargo.toml                ← workspace で interop/h3 / interop/wt を member 化
Cargo.lock                ← workspace 共通
```

## 設計方針

- `git rm interop/h3/Cargo.lock interop/wt/Cargo.lock`
- 削除後に workspace の `Cargo.lock` のみで `cargo build -p interop_h3 -p interop_wt` が成功することを確認
- `CHANGES.md` の `### misc` セクション (リファクタ扱い) に `[UPDATE] interop/h3 と interop/wt の個別 Cargo.lock を削除する` を追加

## 完了条件

- `interop/h3/Cargo.lock` と `interop/wt/Cargo.lock` が削除される
- `cargo build -p interop_h3 -p interop_wt` が workspace `Cargo.lock` のみで成功する
- `make fmt && make clippy && make check` が通る

## 解決方法

```bash
git rm interop/h3/Cargo.lock interop/wt/Cargo.lock
```

### 関連ファイル

- 修正対象: `interop/h3/Cargo.lock`, `interop/wt/Cargo.lock`
- `CHANGES.md` 追記必要
