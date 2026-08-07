# tokio-s2n-quic の WtSession::close がカプセルを H3 DATA フレームで包まず送信する

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-s2n-wt-close-capsule-frame
- Polished: {YYYY-MM-DD}

## 目的

セッションクローズカプセルを正しい H3 DATA フレーム形式で送信し、ピアに届くようにする。

## 現状

- `crates/tokio-s2n-quic/src/webtransport/session.rs` の `WtSession::close` は `Capsule::encode` の出力 (先頭 0x68 0x43 = カプセルタイプ 0x2843) をそのまま `connect_send.send()` で送信する
- CONNECT ストリーム上のデータは HTTP/3 フレーム形式でなければならず、カプセルは DATA フレームのペイロードとして包む必要がある (RFC 9297)
- 0x2843 は H3 フレームタイプとして未知であり、RFC 9114 の「未知フレームは無視」に従いピアに永遠に届かない
- `docs/SAFARI_WT.md` は「各カプセルは個別の H3 DATA フレームで送る」ことを必須と明記している
- `examples/wt_echo_client.rs` がこの `close()` を呼ぶため、エコー動作の終了通知が実質無効

## 設計方針

- `close()` (および他のカプセル送信) でカプセルを DATA フレームヘッダー (0x00 + varint 長) で包んでから送信する

## 完了条件

- ピアが `WtSession::close` のカプセルを受信・処理できる
- テストが追加される
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `crates/tokio-s2n-quic/src/webtransport/session.rs` (`WtSession::close` / `connect_send`)
- 一次資料: `refs/webtrans/rfc9297.txt` Section 3 (Capsule Protocol)、`docs/SAFARI_WT.md`
