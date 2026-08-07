# canary.py が非 canary バージョンから 4 セグメントの不正バージョンを生成する

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-canary-version-bump
- Polished: {YYYY-MM-DD}

## 目的

canary リリース時に Cargo が reject するバージョンを生成するスクリプトのバグを修正する。

## 現状

- `canary.py` の `update_version` は `-canary.` を含まないバージョン (例: `2.0.3`) に対して、パッチ番号を +1 した上で `-canary.0` を付与する
- 置換結果は `2.0.4.0-canary.0` の 4 セグメント形式になり、Cargo / semver が要求する `MAJOR.MINOR.PATCH` (3 セグメント) から逸脱して reject される
- コメント「次のマイナーバージョンにして -canary.0 を追加」の意図 (例: `2.1.0-canary.0`) と実装が乖離している
- `run_cargo_update` の `cargo update shiguredo_http3` は path 依存のため無意味であり、コメントの「shiguredo_http11」はタイポ

## 設計方針

- 非 canary からの遷移を「マイナー +1、パッチ 0、-canary.0」にする (例: `2.0.3` → `2.1.0-canary.0`)

## 完了条件

- 非 canary バージョンから canary バージョンへの更新が 3 セグメント形式で生成される
- 生成されたバージョンで `cargo build` が通る
- テスト (スクリプトのドライラン検証) が追加される

## 解決方法

### 関連ファイル

- `canary.py` (`update_version` / `run_cargo_update`)
