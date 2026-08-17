# tokio-s2n-quic の WtSession::close がカプセルを H3 DATA フレームで包まず送信する

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-s2n-wt-close-capsule-frame
- Polished: 2026-08-16

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
- 0167 (死にコード削除) が `encode_as_data_frame` を削除候補に挙げているため、0167 より先に本 issue を実装して利用箇所を作る
- draft-16 Section 6 の MUST (WT_CLOSE_SESSION 送信後は CONNECT ストリームに即座に FIN を送る) に従い、`close()` はカプセル送信後に `connect_send.finish()` を呼ぶ
- Application Error Message の 1024 バイト制限 (draft-16 Section 6: 超過時は受信側が H3_MESSAGE_ERROR でリセット) の扱いは本 issue の範囲外とする (現状の `close()` は制限を検査しない)

## 完了条件

- ピアが `WtSession::close` のカプセルを受信し、`WebTransportEvent::SessionClosed` として処理できる (`close_error_code` / `close_message` が送信値と一致する)。この検証は 0156 の統合テスト (ループバック実接続でピアをクローズ) で行う
- テストが追加される (0156 の実装に依存しない。`encode_as_data_frame` の出力を `feed_stream` 経由で sans-I/O 層の受信処理に注入し、WT_CLOSE_SESSION が DATA フレームから取り出されること、および修正前の raw カプセルバイトでは取り出されないことを検証する回帰テスト。0156 は本 issue の修正後に `WtSession::close` でピアをクローズするテストを行う前提)。`close()` の `finish()` 呼び出しの検証も含める
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `crates/tokio-s2n-quic/src/webtransport/session.rs` (`WtSession::close` / `connect_send`。`Capsule::encode_as_data_frame` の利用と `finish()` の追加。ついでに `close()` / `connect_send` の doc コメントのカプセル名 (CLOSE_WEBTRANSPORT_SESSION → WT_CLOSE_SESSION) とドラフト版参照 (15 → 16) を修正する)
- リポジトリルートの `examples/wt_server` の `WtSession::close` も同一バグを持つが、本 issue の対象外とする (呼び出し元が存在せず、影響が限定的なため)
- 0156 と同一ファイルを変更するが、本 issue を先に実装する (0156 の完了条件テストが本 issue の修正を前提としているため)
- 一次資料: `refs/webtrans/rfc9297.txt` Section 3 (Capsule Protocol)、`refs/h3/rfc9114.txt` Section 9、`refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 6、`docs/SAFARI_WT.md`
