# tokio-ngtcp2 の WebTransport フロー制御カプセル連携を安定させる

- Created: 2026-09-18
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-wt-flow-control-capsules
- Polished: {YYYY-MM-DD}
- Refreshed: 2026-11-14

## 目的

WebTransport のフロー制御 (draft-ietf-webtrans-http3-17 Section 5.1 / 5.6) を有効にしたときに相互運用テストが不安定になる問題を解消する。現在は回避策として tokio-ngtcp2 側の `wt_settings` が初期フロー制御値を広告しないため、interop 接続ではフロー制御が無効になっており、フロー制御カプセルの経路がテストで検証できていない。

## 現状

- `crates/tokio-ngtcp2/src/webtransport.rs` の `wt_settings` は `wt_initial_max_streams_uni` / `wt_initial_max_streams_bidi` / `wt_initial_max_data` を送らず、WebTransport のフロー制御を無効にしている (QUIC のフロー制御のみ有効)
- `interop/wt/src/lib.rs` の `test_wt_settings` は s2n 側の初期上限を bidi / uni 各 100 ストリーム、データ量 1 MiB に設定している。フロー制御の有効化には両端の宣言が必要なため、ngtcp2 側が宣言しない現行設定では無効のままである
- 起票時は ngtcp2 側の初期フロー制御値を有効にすると `interop/wt/tests/s2n_client_ngtcp2_server.rs` の `test_large_data` (64 KiB 送信) が断続的に失敗した (数回に 1 回)。現行コードでの再現状況は未確認である
  - 受信量が 16〜63 KiB で停止し、カプセル未到達による送信クレジットの未回復が疑われた
  - または `Http3(ConnectionError(FrameError))` で接続が切れた。起票時は登録済み WT ストリームの stream header 再解決を原因としたが、現行の `src/connection/mod.rs` の `Connection::feed_stream` と `src/connection/wt_stream.rs` の `Connection::handle_wt_bidi_stream` は登録済みストリームをヘッダー解析せず処理する。このエラーの原因は未確認である
- 起票後の `90e2e7d` で s2n 側の送信フロー制御と受信カプセルの反映が実装された。現在は `crates/tokio-s2n-quic/src/webtransport/session.rs` の `apply_flow_control_capsule` と `crates/tokio-s2n-quic/src/webtransport/flow_control.rs` の `WtSendFlowControl::process_capsule` が WT_MAX_DATA / WT_MAX_STREAMS を反映するため、起票時の症状はこの変更を含む現行コードで確認し直す必要がある
- カプセルの取得と DATA フレーム化は `crates/tokio-ngtcp2/src/h3.rs` の `H3State::take_wt_capsules` が行う。クライアントは `ClientWebTransportSession::deliver_capsules` が `Client::take_wt_capsules` で取得し、`crates/tokio-ngtcp2/src/client.rs` の `Client::write_wt_stream` を経由して QUIC の CONNECT ストリームへ直接書き込む。サーバーは `drive_connection` が `H3State::take_wt_capsules` の結果を `state.conn.write_stream` で直接書き込む。いずれも HTTP/3 層の送信バッファ (`H3State::write_pending`) を経由しないため、HTTP/3 層が同じストリームに積んだ DATA フレームとの順序が保証されない
- `wt_data_consumed` は `ClientWebTransportSession::poll` と `drive_connection` でアプリケーションにイベントを渡す時点で呼んでいる

## 設計方針

- カプセルの DATA フレーム化と送信順序を HTTP/3 層に任せる経路を検討する (`send_body` による DATA フレーム送出など)
- `wt_data_consumed` の呼び出し量とタイミングを見直す (アプリケーションが実際に消費したバイト数と一致させる)
- フロー制御を有効にした設定で 64 KiB 転送を繰り返す再現テストを追加する
- 原因が `shiguredo_http3` 側にある場合は同ライブラリにテストを追加する

## 完了条件

- interop 用設定でフロー制御を有効にしても `interop/wt` のテストが安定して通る (64 KiB 転送が複数回の実行で失敗しない)
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `crates/tokio-ngtcp2/src/webtransport.rs` (`wt_settings` / `deliver_capsules` / `drive_connection`)
- `crates/tokio-ngtcp2/src/h3.rs` (`take_wt_capsules` / `wt_data_consumed`)
- `crates/tokio-ngtcp2/src/client.rs` (`Client::take_wt_capsules` / `Client::write_wt_stream`)
- `src/connection/client.rs` / `src/connection/server.rs` (`take_wt_pending_capsules` / `wt_data_consumed`)
- `src/connection/mod.rs` (`Connection::feed_stream`)
- `src/connection/wt_stream.rs` (`Connection::handle_wt_bidi_stream`)
- `crates/tokio-s2n-quic/src/webtransport/session.rs` (`apply_flow_control_capsule`)
- `crates/tokio-s2n-quic/src/webtransport/flow_control.rs` (`WtSendFlowControl::process_capsule`)
- `interop/wt/tests/s2n_client_ngtcp2_server.rs` (`test_large_data`)
- `interop/wt/src/lib.rs` (`test_wt_settings`)
- 一次資料: `refs/webtrans/draft-ietf-webtrans-http3-17.txt` Section 5.1 (Negotiating the Use of Flow Control)、Section 5.6 (Flow Control Capsules)
