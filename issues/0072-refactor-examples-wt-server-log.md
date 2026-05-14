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

2 つの選択肢がある:
1. `tracing` → `log` に移行する (CLAUDE.md 遵守)
2. CLAUDE.md の規約を更新し、サンプルでの `tracing` 使用を明示的に許可する

## 影響範囲

- `examples/wt_server/Cargo.toml:19-20`
- `examples/wt_server/src/main.rs`
- `examples/wt_server/src/webtransport.rs`
- `examples/wt_server/src/tls.rs`
