# canary.py が非 canary バージョンから 4 セグメントの不正バージョンを生成する

- Created: 2026-08-08
- Completed: 2026-08-23
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

### 検証結果: 現状分析は誤りであり、実装は既に正しい

本 issue の現状分析は実装 (git 履歴上 `canary.py` は初回コミットから変更なし) と食い違っていた。実際の動作を検証した結果を記録する。

- `update_version` の正規表現は `(version\s*=\s*")(\d+)\.(\d+)\.(\d+)` であり、キャプチャグループは group(1) = `version = "`、group(2) = MAJOR、group(3) = MINOR、group(4) = PATCH となる
- 非 canary 分岐の置換式 `f"{m.group(1)}{m.group(2)}.{int(m.group(3)) + 1}.0-canary.0"` は MAJOR をそのまま保ち、MINOR を +1 し、PATCH を 0 にした上で `-canary.0` を付与する
  - `2.0.3` → `2.1.0-canary.0` (3 セグメント) を正しく生成し、cargo/build が reject する 4 セグメント形式 `2.0.4.0-canary.0` にはならない
- コメント「次のマイナーバージョンにして -canary.0 を追加」と実装は乖離していない。group(2) / group(3) の対応を誤読みしたことが分析誤りの原因
- issue 執筆時 (2026-08-08) 時点のコードと現在のコードで差はなく、完了条件の「3 セグメント形式で生成される」「cargo build が通る」は既に満たされていた

### 修正内容

- コードの動作変更は不要だったため行わない。実在した問題のみ修正する
- `run_cargo_update` の doc コメント「shiguredo_http11」を「shiguredo_http3」に修正する (タイポ)
- 非 canary 分岐のコメントに「例: 2.0.3 -> 2.1.0-canary.0」を追記し、期待する変換を明示する
- `run_cargo_update` が行う `cargo update shiguredo_http3` は `[workspace.dependencies]` で `shiguredo_http3 = { path = "." }` と定義されており path 依存のため実質無効だが、これは本 issue の本題ではないため残す (スクリプトの簡素化は別途検討)
- テスト追加 (ドライラン検証) は行わない。実装の正しさは上記検証のとおりで、`--dry-run` 手前で対話プロンプトが挟まるためスクリプト自体の自動テスト化には新たな設計判断が必要であり、本 issue の修正内容からは逸脱するため
