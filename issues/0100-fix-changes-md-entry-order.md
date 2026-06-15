# `CHANGES.md` の `## develop` セクションのエントリ種別順序違反を是正する

- Priority: High
- Created: 2026-06-15
- Completed:
- Model: Opus 4.7
- Branch: feature/fix-changes-md-entry-order
- Polished:

## 目的

`CHANGES.md` の `## develop` セクションが規約 (CHANGE → ADD → UPDATE → FIX の順) に違反した順序で並んでいる。さらに `### misc` セクションに機能影響のある `[FIX]` エントリが置かれているなど分類違反もある。リリース時の変更履歴の信頼性を保つため整理する。

## 優先度根拠

High。CLAUDE.md の変更履歴規約 (「エントリは種別の順番を守って記載すること (CHANGE → ADD → UPDATE → FIX の順)」) の明示的な違反であり、リリースノート公開時に混乱を招く。リリース前の整理は必須。

## 現状

`CHANGES.md` `## develop` セクションは次のような順序になっている:

- L14-27: `[CHANGE]` 7 件
- L28-78: `[ADD]` 多数
- L80-166: `[CHANGE]` 多数 (本来は最初の `[CHANGE]` ブロックと統合されるべき)
- L167-184: `[FIX]` 多数
- `[UPDATE]` が `## develop` 本体には存在せず `### misc` のみ

`### misc` (L186-238) には次の機能影響 FIX が誤って配置されている:

- L236-237 `[FIX] STOP_SENDING 受信時のクリティカルストリーム判定...`
- L238 `[FIX] payload が欠落した GOAWAY フレームのデコードエラー...`

CLAUDE.md「機能に直接影響しない変更（ドキュメント追加、リファクタリング等）は `### misc` サブセクションに記載すること」より、これらは `## develop` 本体に置くべき。

## 設計方針

- `## develop` 内のエントリを CHANGE → ADD → UPDATE → FIX の順に並べ替える
- `### misc` から機能影響のある FIX を `## develop` 本体に移す
- `### misc` には機能に影響しないリファクタ / ドキュメント / CI 関連の変更のみを残す
- 各エントリの担当者行 (`- @ユーザー名`) のインデント (変更内容より 2 文字下げ) が崩れていないか確認する
- エントリの内容は変更しない (順序とセクション分類のみ修正)
- 重複エントリがないか確認し、あれば統合する

## 完了条件

- `## develop` の最初から最後まで CHANGE → ADD → UPDATE → FIX の順で並んでいる
- `### misc` に機能影響のある FIX が存在しない
- `make fmt` 相当のフォーマッタ / markdownlint で警告が出ない
- エントリの本文・担当者行は手を加えず、順序とセクション分類のみが変わる

## 解決方法

1. `## develop` 内のエントリを抜き出して種別ごとに分類
2. CHANGE → ADD → UPDATE → FIX の順で再配置
3. `### misc` から機能影響 FIX (STOP_SENDING / GOAWAY decode) を本体 FIX セクションへ移動
4. `markdownlint` (prek.toml 設定参照) でリンティング
5. 目視確認

### 関連ファイル

- 修正対象: `CHANGES.md`
- 規約: `CLAUDE.md` (リポジトリルート)
