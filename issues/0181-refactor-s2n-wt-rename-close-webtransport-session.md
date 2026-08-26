# tokio-s2n-quic の webtransport/server.rs に残る旧名 CLOSE_WEBTRANSPORT_SESSION を WT_CLOSE_SESSION に統一する

- Created: 2026-08-27
- Completed: {YYYY-MM-DD}
- Branch: feature/refactor-s2n-wt-rename-close-webtransport-session
- Polished: {YYYY-MM-DD}

## 目的

`tokio-s2n-quic` クレート内の doc コメントに残る旧名 `CLOSE_WEBTRANSPORT_SESSION` を、現行仕様 (draft-ietf-webtrans-http3-16) の名称 `WT_CLOSE_SESSION` に統一し、同一クレート内の名称混在を解消する。

## 現状

- `crates/tokio-s2n-quic/src/webtransport/session.rs` の `WtSession::connect_send` フィールドおよび `WtSession::close` の doc コメントは既に `WT_CLOSE_SESSION` に統一されている
- 一方 `crates/tokio-s2n-quic/src/webtransport/server.rs` の `WtSessionRequest::send_stream` フィールドの doc コメントには旧名 `CLOSE_WEBTRANSPORT_SESSION` が残っており、同じ役割 (CONNECT ストリーム上でセッションクローズカプセルを送信する送信端) を指しているにもかかわらず名称が混在する
- カプセルタイプ 0x2843 の正式名称は RFC 9297 Section 3 / draft-ietf-webtrans-http3-16 Section 6 で `WT_CLOSE_SESSION`。旧名 `CLOSE_WEBTRANSPORT_SESSION` は draft-14 以前で使われていた表記で、現行仕様には存在しない
- 名称混在は grep やコードリーディング時に「同じものを指す 2 種類の名前」を追跡する負担を生む

## 設計方針

- `crates/tokio-s2n-quic/src/webtransport/` 配下で `CLOSE_WEBTRANSPORT_SESSION` を grep し、doc コメント・実装コメント内の記述をすべて `WT_CLOSE_SESSION` に置換する
- カプセル自体の実装 (`shiguredo_http3::webtransport::capsule::Capsule::CloseSession`) 側の識別子は変更しない (本 issue は doc コメントに絞る)
- 親クレート (`src/webtransport/capsule.rs` の `//!` doc、`src/webtransport/session/mod.rs` の `close_with_error` doc 等) に残る draft-15 / 旧名参照の統一は本 issue のスコープ外とする (別 issue で扱う)

## 完了条件

- `grep -r "CLOSE_WEBTRANSPORT_SESSION" crates/tokio-s2n-quic/` が 0 件を返す
- 変更後も `cargo test --workspace --tests` / `cargo fmt --all -- --check` / `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `crates/tokio-s2n-quic/src/webtransport/server.rs` (`WtSessionRequest::send_stream` フィールドの doc コメント)

### 一次資料

- `refs/webtrans/rfc9297.txt` Section 3 (Capsule Protocol)
- `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 6 (Session Termination) の `WT_CLOSE_SESSION` 定義
