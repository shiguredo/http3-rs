# 0081: ClientConnection / ServerConnection の薄いラッパーの責務強化

Created: 2026-05-14
Model: deepseek-v4-pro

## 概要

`ClientConnection` (`src/connection/client.rs`) と `ServerConnection` (`src/connection/server.rs`)
は `Connection` への単なる委譲ラッパーであり、追加ロジックはゼロ。
`Connection` が `send_request`/`send_response` 両方を露出しており、ラッパーの
カプセル化効果が不十分。

## 修正方針

2 つの選択肢:
1. ラッパーを削除し、`Connection` に統一する
2. `Connection` 側の `send_request`/`send_response` を `pub(crate)` に落とし、
   ラッパー経由でのみ呼べるようにする

選択肢 2 を推奨。型レベルで役割を区別する意図は妥当なため。

## 影響範囲

- `src/connection/client.rs`
- `src/connection/server.rs`
- `src/connection/mod.rs`
