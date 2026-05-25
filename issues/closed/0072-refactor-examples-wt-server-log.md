# 0072: examples/wt_server が tracing を使用している (CLAUDE.md 違反)

Created: 2026-05-14
Model: deepseek-v4-pro

## 概要

CLAUDE.md L35-L36: 「サンプルはお手本なので性能と堅牢性を両立させること」
CLAUDE.md L151: 「ログはできるだけださないが、使う場合は log を使うこと」

しかし `examples/wt_server` は `tracing` と `tracing-subscriber` に依存し、
全ソースファイルで `tracing::info!` / `tracing::debug!` / `tracing::error!` / `tracing::warn!`
マクロを使用している。

## 修正方針

CLAUDE.md L151 の「使う場合は log を使うこと」に従い、`tracing` → `log` に移行する。

- `tracing::info!` → `log::info!`
- `tracing::debug!` → `log::debug!`
- `tracing::error!` → `log::error!`
- `tracing::warn!` → `log::warn!`
- `tracing-subscriber` を削除し、`env_logger` 等の log 互換バックエンドに置き換える

## 影響範囲

- `examples/wt_server/Cargo.toml:19-20`
- `examples/wt_server/src/main.rs`
- `examples/wt_server/src/webtransport.rs`
- `examples/wt_server/src/tls.rs`

## 解決方法

polish-issue 時に issue の前提が誤っていることを確認したためクローズする。

AGENTS.md (148-149行) は「ログは tracing を使うこと / ログのフィルタリングは tracing-subscriber を使うこと」と規定しており、`examples/wt_server` が tracing を使用しているのは AGENTS.md の規定に**準拠**している。issue は旧 CLAUDE.md の文言を誤って引用したものと推測される。

Completed: 2026-05-26
