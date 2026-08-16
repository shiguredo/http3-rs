# フロー制御無効時に WebTransport 送信 API が常に false を返す

- Created: 2026-08-08
- Completed: 2026-08-16
- Branch: feature/fix-wt-flow-control-disabled-send
- Polished: 2026-08-15

## 目的

draft-ietf-webtrans-http3-16 Section 5.1 の「フロー制御無効時は Section 5 の制限が適用されない」を送信 API にも適用する。

## 現状

- `src/webtransport/session/mod.rs` の `Session::try_send_data` / `Session::try_open_stream` と事前チェック API (`can_send_data` / `can_create_unidirectional_stream` / `can_create_bidirectional_stream`) は送信許可判定で `flow_control_enabled` を参照せず、`remote_limits` (フロー制御無効時はカプセルが無視されるため 0 のまま) と比較する
- フロー制御無効時は `0 (max_data - data_sent) >= bytes` が bytes >= 1 で常に偽となり、1 バイトも送信できずストリームも開けない
- 受信側 (`check_received_data` / `check_received_stream`) は `flow_control_enabled` で無制限に分岐しており、同じフラグで送受信の意味論が非対称
- 根拠: draft-16 Section 5.1「If both endpoints take at least one of these actions, flow control is enabled, and the limits described in the entirety of Section 5 apply」(フロー制御無効時は Section 5 の制限が適用されない帰結)

## 設計方針

- `!flow_control_enabled` のときは上限なしとして true を返す分岐を `try_send_data` / `try_open_stream` に追加する
- 事前チェック API (`can_send_data` / `can_create_unidirectional_stream` / `can_create_bidirectional_stream`) にも同じ分岐を追加する (try 系だけ true で can 系が false のままになる自己矛盾を防ぐ)
- `data_sent` / `streams_uni_opened` / `streams_bidi_opened` カウンタの加算はフロー制御有効時と同様に行う (公開 API `flow_state()` の観測値を維持する)
- セッション状態によるガード (`state.can_send()` / `state.can_create_stream()`) は維持する (Pending / Connecting / Closed では false のまま)

## 完了条件

- フロー制御無効時 (かつセッションが送信可能状態) に `try_send_data` / `try_open_stream` / `can_send_data` / `can_create_unidirectional_stream` / `can_create_bidirectional_stream` が上限超過以外の理由で false を返さない
- テストが追加される
- 既存テスト `test_flow_control_disabled_no_capsules` は挙動変更に合わせて更新する
- CHANGES.md の `## develop` セクションに変更履歴が記載される
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/webtransport/session/mod.rs` (`Session::try_send_data` / `try_open_stream` / `can_send_data` / `can_create_unidirectional_stream` / `can_create_bidirectional_stream` / `flow_control_enabled`)
- 一次資料: `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 5.1
