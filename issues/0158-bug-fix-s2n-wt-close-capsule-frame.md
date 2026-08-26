# tokio-s2n-quic の WtSession::close がカプセルを H3 DATA フレームで包まず送信する

- Created: 2026-08-08
- Completed: 2026-08-27
- Branch: feature/fix-s2n-wt-close-capsule-frame
- Polished: 2026-08-26

## 目的

セッションクローズカプセルを正しい H3 DATA フレーム形式で送信し、ピアに届くようにする。

## 現状

- `crates/tokio-s2n-quic/src/webtransport/session.rs` の `WtSession::close` は `Capsule::encode` の出力 (先頭 0x68 0x43 = カプセルタイプ 0x2843) をそのまま `connect_send.send()` で送信する
- CONNECT ストリーム上のデータは HTTP/3 フレーム形式でなければならず、カプセルは DATA フレームのペイロードとして包む必要がある (RFC 9297 Section 3)
- 0x2843 は H3 フレームタイプとして未知であり、RFC 9114 Section 9 の「未知フレームは無視」に従い、受信側のフレームデコード (request.rs の `Frame::Unknown`) で無視されカプセル処理に到達しない
- `docs/SAFARI_WT.md` は「各カプセルは個別の H3 DATA フレームで送る」ことを必須と明記している
- `crates/tokio-s2n-quic/examples/wt_echo_client.rs` がこの `close()` を呼ぶため、エコー動作の終了通知が実質無効

## 設計方針

- `WtSession::close` でカプセルを DATA フレーム (0x00 + varint 長 + ペイロード) で包んでから送信する。包み方は既存の `Capsule::encode_as_data_frame` (shiguredo_http3 の `src/webtransport/capsule.rs`) を利用し、再実装しない。受信側 (sans-I/O 層の `handle_wt_data_frame` → `process_wt_capsule_data`) は DATA フレーム剥離後のカプセル処理を既に持つため、送信側が包むことで対称性が取れる
- 現状 tokio-s2n-quic のカプセル送信は `close()` のみのため、本 issue の対象は `close()` に限定する (今後 tokio-s2n-quic でカプセル送信を追加する場合も同じヘルパーを使う)
- 0167 (死にコード削除) は完了済みで、`encode_as_data_frame` は本 issue での利用予定により維持されている (0167 の維持判断)。本 issue はそのまま `encode_as_data_frame` を利用する
- draft-16 Section 6 の MUST (WT_CLOSE_SESSION 送信後は CONNECT ストリームに即座に FIN を送る) に従い、`close()` はカプセル送信後に `connect_send.finish()` を呼ぶ
- Application Error Message の 1024 バイト制限 (draft-16 Section 6: 超過時は受信側が H3_MESSAGE_ERROR でリセット) の扱いは本 issue の範囲外とする (現状の `close()` は制限を検査しない)

## 完了条件

- ピアが `WtSession::close` のカプセルを受信し、`WebTransportEvent::SessionClosed` として処理できる (`close_error_code` / `close_message` が送信値と一致する)。この検証は 0156 の統合テスト (ループバック実接続でピアをクローズ) で行う
- テストが追加される (0156 の実装に依存しない): `encode_as_data_frame` の出力を `feed_stream` 経由で sans-I/O 層の受信処理に注入し、WT_CLOSE_SESSION が DATA フレームから取り出されること、および修正前の raw カプセルバイトでは取り出されないことを検証する回帰テスト。0156 は本 issue の修正後に `WtSession::close` でピアをクローズする統合テストを行う前提
- `close()` の `finish()` 呼び出し (FIN 送信) の検証は 0156 の統合テストで行う (実 QUIC 接続が必要なため。0156 の受信タスクは FIN 受信で終了する)
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 変更内容

- `crates/tokio-s2n-quic/src/webtransport/session.rs` の `WtSession::close`:
  - `Capsule::encode` の raw カプセルバイトを送っていた実装を `Capsule::encode_as_data_frame` に切り替え、WT_CLOSE_SESSION を H3 DATA フレーム (0x00 + varint 長 + ペイロード) として送出するようにした
  - カプセル送信直後に `SendStream::finish()` を呼び、CONNECT ストリームに FIN を送出するようにした (draft-16 Section 6 の MUST)
  - `connect_send` フィールドおよび `close()` メソッドの doc コメントを実態に合わせて更新した (旧名 `CLOSE_WEBTRANSPORT_SESSION` を `WT_CLOSE_SESSION` に、参照 draft を 15 から 16 に置き換え、FIN 送出用途を明記)
- 実装コメントに RFC 9297 Section 3.1 / RFC 9114 Section 7.2.1 の DATA フレーム根拠と draft-16 Section 6 の FIN MUST 根拠を記載した
- `tests/test_webtransport_draft_connect.rs` に `close_session_capsule_framing` モジュールを追加し、以下の 2 テストで回帰を防ぐようにした:
  - `wt_close_session_wrapped_in_data_frame_is_processed`: `encode_as_data_frame` の出力を `feed_stream` に注入すると `SessionClosed` イベントが発火し、`close_error_code` / `close_message` がカプセルの値と一致する
  - `wt_close_session_raw_capsule_bytes_are_not_processed`: `encode` の raw カプセルバイトを `feed_stream` に注入すると先頭 varint (0x2843) が未知 H3 フレームタイプとして無音破棄され `SessionClosed` が発火しない
- `CHANGES.md` の `## develop` セクションに `[FIX]` エントリを追加した

### 対象外

- リポジトリルートの `examples/wt_server` の `WtSession::close` も同一バグを持つが、本 issue の対象外 (呼び出し元が存在せず、影響が限定的)
- application error message の 1024 バイト制限 (draft-16 Section 6) の検証は本 issue の対象外
- `close()` の二重呼び出し保護 (`&mut self` のまま idempotent 化しない) は本 issue の対象外
- `crates/tokio-s2n-quic/src/webtransport/server.rs` の同クレート内フィールドコメントに残る旧名 `CLOSE_WEBTRANSPORT_SESSION` は本 issue の対象外 (`close()` / `connect_send` の doc に限定した設計方針のため)
- `close()` の実 QUIC 経由結合テスト (CONNECT ストリームの FIN 到達と `SessionClosed` の実接続経路) は 0156 の統合テストで実施する前提

### 一次資料

- `refs/webtrans/rfc9297.txt` Section 3.1 (HTTP Data Streams)
- `refs/h3/rfc9114.txt` Section 7.2.1 (DATA Frame)
- `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 6 (Session Termination)
- `docs/SAFARI_WT.md`
