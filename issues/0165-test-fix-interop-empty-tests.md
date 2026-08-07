# interop テストの空振り (assert なし・全分岐パス) を修正する

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
- Branch: feature/test-fix-interop-empty-tests
- Polished: {YYYY-MM-DD}

## 目的

相互運用テストのうち、検証が空振りして「失敗し得ない」テストを実質的な検証に修正する。

## 現状

- `interop/wt/tests/quinn_client_ngtcp2_server.rs` は確立成功 / 失敗 / タイムアウトの全分岐でテストがパスする (draft バージョン不一致を「想定内」として assert なし)
- `interop/wt/tests/ngtcp2_client_quinn_server.rs` はセッション失敗 (`Ok(None)`) でも「想定内」としてパスする
- `interop/h3/tests/ngtcp2_client_quinn_server.rs` / `ngtcp2_client_tquic_server.rs` / `ngtcp2_client_s2n_server.rs` はレスポンス内容・ステータス・ヘッダーを一切 assert しない (空ボディ・status 0 でもパス)
- `quiche_client_*` 系は `contains("Hello") || !is_empty()` の or 条件で実質「非空」検証のみ
- `interop/wt` の uni ストリーム / datagram テストは「セッション確立の成功のみを確認する」とコメントで自認しており、主要機能が未検証
- `interop/wt` の組み合わせ行列で `quinn_client_s2n_server` が存在しない

## 設計方針

- 各テストの期待値を明確にし、レスポンス内容 (ボディ・ステータス・ヘッダー) を assert する
- 全分岐パスのテストは期待する分岐のみパスするよう assert を追加する
- `quinn_client_s2n_server` のテストを追加する

## 完了条件

- interop テストがレスポンス内容を検証する
- 失敗し得ないテストがなくなる
- `make interop-test` が通る

## 解決方法

### 関連ファイル

- `interop/wt/tests/` 配下 (quinn_client_ngtcp2_server.rs / ngtcp2_client_quinn_server.rs ほか)
- `interop/h3/tests/` 配下 (ngtcp2_client_quinn_server.rs / ngtcp2_client_tquic_server.rs / ngtcp2_client_s2n_server.rs ほか)
