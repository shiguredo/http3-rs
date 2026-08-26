# WebTransport 実装内の draft-15 参照コメントを最新 draft-16 に統一する

- Created: 2026-08-27
- Completed: {YYYY-MM-DD}
- Branch: feature/refactor-wt-align-draft-references-to-16
- Polished: {YYYY-MM-DD}

## 目的

`src/` および `crates/tokio-s2n-quic/` 配下のソースコードコメントに残る `draft-ietf-webtrans-http3-15` 参照を、リポジトリの主参照仕様である `draft-ietf-webtrans-http3-16` に統一する。参照の混在によりコードリーディング時に「どの draft を実装しているのか」が不明瞭になる問題を解消する。

## 現状

- リポジトリの一次資料は `refs/webtrans/draft-ietf-webtrans-http3-16.txt`
- 一方で `src/` 内の 20 以上のファイルに `draft-ietf-webtrans-http3-15` 参照が計 200 箇所以上残存している
  - 特に多い箇所: `src/webtransport/session/mod.rs` (31 箇所)、`src/connection/wt_stream.rs` (24 箇所)、`src/event.rs` (14 箇所)、`src/webtransport/connect/request.rs` (10 箇所)、`src/connection/wt_capsule.rs` (10 箇所)
- 本参照混在の直近の顕在化: `crates/tokio-s2n-quic/src/webtransport/session.rs` の `WtSession::close` の doc コメントは既に draft-16 に更新されたが、親クレート `src/webtransport/session/mod.rs` の `close_with_error`・`src/webtransport/capsule.rs` の `//!` doc は draft-15 のまま残っている
- draft-15 と draft-16 で節番号や本文が同じ節も多いが、Section 番号が変わっている節・新設された節もあるため、単純テキスト置換だけでは正確性を担保できない

## 設計方針

- `refs/webtrans/draft-ietf-webtrans-http3-16.txt` を主参照とし、各コメントの節番号と該当文言を実際に開いて確認する
- 参照節番号が draft-15 と draft-16 で **同一** の場合はテキストのみ `draft-15` → `draft-16` に置換
- 参照節番号が **変わっている** 場合は正しい節番号に置換
- draft-16 で **廃止された** 節・機能を参照している場合はコメント本文の書き直しが必要 (該当があれば個別に判断)
- コメント内の「将来変更される可能性がある」旨の注記は維持する (draft の仕様である以上、将来変更されうる)
- `refs/webtrans/draft-ietf-webtrans-http3-15.txt` の削除は本 issue の対象外 (別途 `update-refs` スキルの範囲)
- 対象範囲は `src/` と `crates/tokio-s2n-quic/src/` に限る。`refs/` 配下や `docs/` 配下のドキュメント引用は対象外

## 完了条件

- `grep -rn "draft-ietf-webtrans-http3-15" --include="*.rs" src/ crates/` が 0 件を返す
- 変更後も `cargo test --workspace --tests` / `cargo fmt --all -- --check` / `cargo clippy --all-targets --all-features -- -D warnings` が通る
- 少なくとも `polish-refs` スキル (refs 引用の正確性検証) が変更箇所について致命的な誤りを検出しない

## 解決方法

### 関連ファイル (draft-15 参照件数の多い順)

- `src/webtransport/session/mod.rs`
- `src/connection/wt_stream.rs`
- `src/event.rs`
- `src/webtransport/connect/request.rs`
- `src/connection/wt_capsule.rs`
- `src/webtransport/connect/mod.rs`
- `src/webtransport/session/flow_control.rs`
- `src/webtransport/stream.rs`
- `src/webtransport/connect/response.rs`
- `src/webtransport/capsule.rs`
- `src/webtransport/error.rs`
- `src/settings.rs`
- `src/webtransport/settings.rs`
- `src/webtransport/connect/draft.rs`
- `src/webtransport/mod.rs`
- `src/webtransport/connect/sf_parser.rs`
- `src/webtransport/datagram.rs`
- `src/webtransport/error.rs`
- `src/connection/client.rs`
- `src/connection/server.rs`

上記に加え、`crates/tokio-s2n-quic/src/` 配下の draft-15 参照も対象とする (grep で個別確認する)。

### 一次資料

- `refs/webtrans/draft-ietf-webtrans-http3-16.txt` (主参照)
- `refs/webtrans/draft-ietf-webtrans-http3-15.txt` (差分確認用。存在する場合)
