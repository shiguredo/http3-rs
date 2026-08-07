# フロー制御無効時に WebTransport 送信 API が常に false を返す

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-wt-flow-control-disabled-send
- Polished: {YYYY-MM-DD}

## 目的

draft-ietf-webtrans-http3-16 Section 5.1 の「フロー制御無効時は Section 5 の制限が適用されない」を送信 API にも適用する。

## 現状

- `src/webtransport/session/mod.rs` の `Session::try_send_data` / `Session::try_open_stream` (および `can_send_data` 系) は `flow_control_enabled` を参照せず、`remote_limits` (フロー制御無効時はカプセルが無視されるため 0 のまま) と比較する
- フロー制御無効時は `data_sent >= 0 >= bytes` が常に偽となり、1 バイトも送信できずストリームも開けない
- 受信側 (`check_received_data` / `check_received_stream`) は `flow_control_enabled` で無制限に分岐しており、同じフラグで送受信の意味論が非対称
- 根拠: draft-16 Section 5.1「If both endpoints take at least one of these actions, flow control is enabled, and the limits described in the entirety of Section 5 apply」

## 設計方針

- `!flow_control_enabled` のときは上限なしとして true を返す分岐を `try_send_data` / `try_open_stream` に追加する

## 完了条件

- フロー制御無効時に `try_send_data` / `try_open_stream` が常に true を返す
- テストが追加される
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/webtransport/session/mod.rs` (`Session::try_send_data` / `Session::try_open_stream` / `flow_control_enabled`)
- 一次資料: `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 5.1
