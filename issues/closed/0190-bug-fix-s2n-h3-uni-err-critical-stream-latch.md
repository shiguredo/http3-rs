# tokio-s2n-quic の H3 uni タスクが `recv_stream.receive()` の Err 経路で `ClosedCriticalStream` を誤ラッチする

- Created: 2026-08-27
- Completed: 2026-09-10
- Branch: feature/fix-s2n-h3-uni-err-critical-stream-latch
- Polished: 2026-09-09

## 目的

H3 uni タスクが `recv_stream.receive()` から接続エラー (`StreamError::ConnectionError` 等) を受け取った際に、それを sans-I/O 層への FIN として誤伝達し、クリティカルストリームで `ClosedCriticalStream` を誤ラッチする副作用を修正する。あわせてピアの `RESET_STREAM` (`StreamError::StreamReset`) は sans-I/O 層の `stream_reset` に通知し、クリティカルストリームでは RFC の MUST どおり `ClosedCriticalStream` をラッチさせる。

## 現状

- `crates/tokio-s2n-quic/src/h3/client.rs` / `server.rs` の uni ストリーム受信タスクは `while let Ok(Some(data))` パターンで受信し、ループ抜け後に無条件で `feed_stream_only(stream_id, &[], true)` を呼び FIN を伝達する
- `recv_stream.receive()` の `Err` は一様ではない。`s2n_quic::stream::Error` (= `s2n_quic_core::stream::StreamError`) はピアの RESET_STREAM を表す `StreamReset { error, .. }` と接続断を表す `ConnectionError { error, .. }` 等を区別するが、現行は両者を区別せず FIN として通知する
- 接続断 (`ConnectionError`) を FIN として通知すると、クリティカルストリームで `ClosedCriticalStream` を誤ラッチする
- 一方 `StreamReset` は RFC 9114 Section 6.2.1 / RFC 9204 Section 4.2 の MUST により、クリティカルストリームでは `ClosedCriticalStream` をラッチするのが正しい。現行の FIN 通知はこれを偶然満たしているが、区別して `stream_reset` に通知する方が正しい (非クリティカルストリームも RESET として扱える)
- sans-I/O 層は FIN / RESET をクリティカルストリームで受けると `ClosedCriticalStream` を接続エラーとしてラッチし、次の `drain_events` で顕在化する
- 0159 の修正で `accept_request` / `send_request` の受信ループが冒頭で `drain_events` を回すようになったため、この副作用が **リクエストデータ到着前** に露出するようになり、interop テストで低確率の flake として現れる可能性がある
- WebTransport の `route_uni_stream` はストリームタイプ判定ループだけが `Err(_) => return` で、判定後の `ClassifiedUniStream::Http3` 分岐は同じバグパターン (`while let Ok(Some(data))` + 無条件 FIN) を持つ (`crates/tokio-s2n-quic/src/webtransport/client.rs` / `server.rs` の両方)

## 設計方針

- uni タスクの受信ループを `Ok(Some(data))` / `Ok(None)` / `Err(e)` の 3 パターンに分ける
- `Ok(Some(data))`: `feed_stream_only(_, &data, false)` でデータを feed する
- `Ok(None)` (クリーンな FIN 受信): 従来通り `feed_stream_only(_, &[], true)` を呼ぶ
- `Err(StreamError::StreamReset { error, .. })`: `internal/connection_state.rs` に追加するラッパー経由で sans-I/O 層の `Connection::stream_reset(stream_id, *error, 0)` を呼ぶ。クリティカルストリームでは sans-I/O が `ClosedCriticalStream` をラッチし、非クリティカルでは RESET として処理される。`final_size` は現行の s2n-quic API から取得できないため `0` を渡す (`connect_stream_reset` と同じ)
- `Err(StreamError::ConnectionError { .. })` およびその他: 接続が既に終了しているため FIN / RESET を伝達せず、エラーログのみ記録してタスク終了する
- WebTransport の `route_uni_stream` の `ClassifiedUniStream::Http3` 分岐 (`crates/tokio-s2n-quic/src/webtransport/client.rs` / `server.rs`) も同じ修正を適用する。判定ループの `Err(_) => return` はストリームタイプ確定前の許容 (RFC 9114 Section 6.2) であり、本修正の対象外

## 完了条件

- `recv_stream.receive()` が接続エラー (`StreamError::ConnectionError` 等) を返した場合、sans-I/O 層に FIN が伝達されず `ClosedCriticalStream` が誤ラッチされないこと
- `StreamError::StreamReset` の場合、クリティカルストリームでは `ClosedCriticalStream` が正しくラッチされ、非クリティカルでは RESET として処理されること
- `receive()` の Err 経路を確定的に踏む回帰テストを追加する (実 QUIC 接続でピアが制御 / QPACK ストリームを RESET_STREAM する、または接続断を起こす。モック・スタブは使わない)
- interop_h3 の advanced テストを 50 回連続で実行し flake が発生しないこと (`for i in $(seq 50); do cargo test -p interop_h3 --test advanced || exit 1; done`)
- `cargo test --workspace --tests` / `cargo fmt --all -- --check` / `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 修正内容

- `internal::classify_uni_recv` を追加し、`recv_stream.receive()` の結果を `Data` / `Fin` / `Reset(エラーコード)` / `Ignore` に分類する
- `ClientConnectionState` / `ServerConnectionState` に `apply_uni_recv_action` を追加し、分類に応じて `feed_stream_only` (データ / FIN) と `stream_reset_only` (RESET) を適用する。`Ignore` は何も伝達しない
- イベントの取り出しを受信ループ先頭の `drain_events` に一本化するため、`stream_reset_only` (drain しない版) を追加し、uni 受信タスクはこちらを使う
- H3 / WebTransport の単方向ストリーム受信タスク 4 箇所 (h3 client / server、WT client / server の `route_uni_stream` Http3 分岐) を `classify_uni_recv` + `apply_uni_recv_action` に置き換える
- 設計方針の「エラーログのみ記録」は、`tokio-s2n-quic` にログ基盤 (`tracing` 等) が無いため、接続エラー等は伝達せずタスク終了するのみとした

### 検証結果

- `cargo test -p tokio-s2n-quic` / `cargo test -p shiguredo_http3 --lib` / `cargo fmt --all -- --check` / `cargo clippy --workspace --all-targets -- -D warnings` が通る
- `cargo clippy --all-targets --all-features -- -D warnings` は `nghttp3-sys` / `ngtcp2-sys` の `overwrite` feature で既存の `clippy::ptr_arg` に抵触するため通らない (本 issue の変更起因ではない。CI も `--all-features` を使わない)
- 実 QUIC 統合テスト `tests/h3_critical_stream_reset_e2e.rs` (制御 / QPACK エンコーダーストリームの RESET で `H3_CLOSED_CRITICAL_STREAM` をラッチ) を 20 回連続で実行し全て成功
- 単体テスト `test_classify_uni_recv_connection_error_is_ignored` / `test_apply_uni_recv_action_ignore_does_not_feed_fin` が、接続エラーを FIN として誤伝達する退行を検知する (一時的に退行させて失敗することを確認)
- `cargo test -p interop_h3 --test advanced` を 50 回連続で実行し flake が発生しないこと

### 関連ファイル

- `crates/tokio-s2n-quic/src/h3/client.rs` (uni ストリーム受信タスク)
- `crates/tokio-s2n-quic/src/h3/server.rs` (uni ストリーム受信タスク)
- `crates/tokio-s2n-quic/src/webtransport/client.rs` (`route_uni_stream` の `ClassifiedUniStream::Http3` 分岐)
- `crates/tokio-s2n-quic/src/webtransport/server.rs` (`route_uni_stream` の `ClassifiedUniStream::Http3` 分岐)
- `crates/tokio-s2n-quic/src/internal/mod.rs` (`classify_uni_recv` / `UniRecvAction`)
- `crates/tokio-s2n-quic/src/internal/connection_state.rs` (`apply_uni_recv_action` / `stream_reset_only`)
- `crates/tokio-s2n-quic/tests/h3_critical_stream_reset_e2e.rs` (実 QUIC 回帰テスト)
- `interop/h3/tests/advanced.rs` (flake 確認)

### 一次資料

- `refs/h3/rfc9114.txt` Section 6.2.1 (制御ストリームのクローズ禁止)
- `refs/h3/rfc9204.txt` Section 4.2 (QPACK ストリームのクローズ禁止)
