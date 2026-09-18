# tokio-ngtcp2 の WebTransport フロー制御カプセル連携を安定させる

- Created: 2026-09-18
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-wt-flow-control-capsules
- Polished: {YYYY-MM-DD}

## 目的

WebTransport のフロー制御 (draft-ietf-webtrans-http3-15 Section 5.1 / 5.6) を有効にしたときに相互運用テストが不安定になる問題を解消する。現在は回避策として interop 用の設定でフロー制御を無効化しており、フロー制御カプセルの経路がテストで検証できていない。

## 現状

- `crates/tokio-ngtcp2/src/webtransport.rs` の `wt_settings` は `wt_initial_max_streams_uni` / `wt_initial_max_streams_bidi` / `wt_initial_max_data` を送らず、WebTransport のフロー制御を無効にしている (QUIC のフロー制御のみ有効)
- これらを有効にすると `interop/wt/tests/s2n_client_ngtcp2_server.rs` の `test_large_data` (64 KiB 送信) が断続的に失敗する (数回に 1 回)
  - 受信量が 16〜63 KiB で停止する。カプセルが届かずピアの送信クレジットが回復しない
  - または `Http3(ConnectionError(FrameError))` で接続が切れる。WT ストリームとして登録済みのストリームで stream header の再解決が走っている
- カプセル配送は `crates/tokio-ngtcp2/src/webtransport.rs` の `deliver_capsules` が `take_wt_pending_capsules` で取り出したカプセルを `encode_as_data_frame` し、QUIC の CONNECT ストリームへ直接 `write_stream` している。HTTP/3 層の送信バッファ (`H3State::write_pending`) を経由しないため、HTTP/3 層が同じストリームに積んだ DATA フレームとの順序が保証されない
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
- `src/connection/client.rs` / `src/connection/server.rs` (`take_wt_pending_capsules` / `wt_data_consumed`)
- `interop/wt/tests/s2n_client_ngtcp2_server.rs` (`test_large_data`)
- 一次資料: `refs/webtrans/draft-ietf-webtrans-http3-15.txt` Section 5.1 (Flow Control)、Section 5.6 (Flow Control Capsules)
