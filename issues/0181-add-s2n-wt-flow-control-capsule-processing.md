# tokio-s2n-quic に WebTransport フロー制御カプセルの処理を追加する

- Created: 2026-08-16
- Completed: {YYYY-MM-DD}
- Branch: feature/add-s2n-wt-flow-control-capsule-processing
- Polished: {YYYY-MM-DD}

## 目的

sans-I/O 層がイベント化した WebTransport フロー制御カプセル (`WebTransportEvent::Capsule`) を `webtransport::Session::process_capsule` に渡し、送信クレジット (WT_MAX_DATA / WT_MAX_STREAMS) の更新と禁止カプセル (WT_MAX_STREAM_DATA / WT_STREAM_DATA_BLOCKED) のエラー処理を行う。

## 現状

- `src/event.rs` の `WebTransportEvent::Capsule` は「上位層は `webtransport::Session::process_capsule` に渡すこと」と定める (draft-ietf-webtrans-http3-16 Section 5.6)
- `crates/tokio-s2n-quic` には `WebTransportEvent::Capsule` の処理経路が存在しない (0156 で受信側保持を実装するとイベント化されるようになる)
- tokio-s2n-quic には per-session の `webtransport::Session` インスタンスが存在せず、`process_capsule` の呼び出し先がない

## 設計方針

- 0156 の受信タスクが `drain_events` で回収した `WebTransportEvent::Capsule` を `webtransport::Session::process_capsule` に渡す
- per-session の `webtransport::Session` インスタンスを tokio-s2n-quic のセッション (WtSession 相当) に持たせる
- フロー制御カプセル受信時の送信許可は `webtransport::Session` の既存 API (送信判定) を利用する

## 完了条件

- `WebTransportEvent::Capsule` が `process_capsule` に渡され、送信クレジットが更新される
- 禁止カプセル受信時にセッションエラーになる
- テストが追加される (実 QUIC 接続のループバック統合テスト。モック・スタブは使わない)
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `crates/tokio-s2n-quic/src/webtransport/` (セッション型への `webtransport::Session` 追加と `process_capsule` 配線)
- `src/webtransport/session/mod.rs` (`process_capsule`)
- 一次資料: `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 5.6
