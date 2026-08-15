# 楽観的カプセル送信のサーバー側バッファリングを実装する

- Created: 2026-07-31
- Completed: {YYYY-MM-DD}
- Branch: feature/add-optimistic-capsule-buffering
- Polished: 2026-08-15

## 目的

draft-ietf-webtrans-http3-16 Section 3.2 の楽観的カプセル送信要件に対応する。0135 から分割。

## 現状

draft-16 追加要件:

> To reduce latency at the start of a WebTransport session, a client MAY optimistically send capsules on the CONNECT stream before receiving the server's response. A server MUST NOT process these bytes as capsules until it sends a 2xx response accepting the session. Bytes received before the server sends the response are processed once the session is accepted or discarded if the session is rejected.

現在、`src/connection/wt_capsule.rs` の `handle_wt_data_frame` は `WtSessionState::Pending` 時に draft-07/14/15 で `H3_MESSAGE_ERROR` を返している。

## 設計方針

- **サーバー側のみ** `handle_wt_data_frame` の Pending 分岐でカプセルデータをバッファリングし、`src/connection/wt_session.rs` の `establish_wt_session_server` (2xx 送信時) でバッファを処理する
- バッファリングの対象は peer draft が draft-02 以外のケース (現行で `H3_MESSAGE_ERROR` を返していた draft-07/14/15)。draft-02 の「黙って破棄」は両ロールで現行のまま維持する
- クライアント側は現行の `H3_MESSAGE_ERROR` を維持する (楽観的送信は client → server 方向のみ)
- セッション拒否の検出は `src/connection/mod.rs` の `send_response` で最終レスポンス (1xx 中間レスポンスを除く) が非 2xx の場合に行い、その時点で Pending セッションを終了してバッファを破棄する。現行コードには非 2xx 送信時に Pending セッションを破棄する経路がないため、破棄処理の追加が必要
- 拒否時の破棄は `capsule_buf` のクリアだけでは不十分 (拒否後に到着する DATA が再びバッファリングされ、拒否後の FIN でスプリアスな `H3_MESSAGE_ERROR` が発生する)。既存のセッション終了処理 (`terminate_wt_session_with`) と同様に Pending セッション自体を終了する
- Pending 状態でバッファ済みデータが残ったまま CONNECT ストリームの FIN を受信した場合は、`H3_MESSAGE_ERROR` にせずバッファを破棄してセッションを終了する (CONNECT ストリームのクローズはセッション終了を意味する。draft-16 Section 6)
- バッファは `WtSession.capsule_buf` を再利用する。DoS 対策としてバッファ上限を設け、超過時は `H3_MESSAGE_ERROR` でストリームをリセットする
- 既存のサーバー側テスト `draft07_pending_data_rejected` / `draft14_pending_data_rejected` / `draft15_pending_data_rejected` は挙動変更に合わせて更新する

## 完了条件

- サーバーが 2xx 前に受信したカプセルデータをバッファリングし、2xx 送信後に処理する
- セッション拒否時にバッファが破棄される
- クライアント側の挙動が変更されない
- テストが追加される
- CHANGES.md の `## develop` セクションに変更履歴が記載される
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/connection/wt_capsule.rs` (`Connection::handle_wt_data_frame`, `handle_wt_stream_end`)
- `src/connection/wt_session.rs` (`establish_wt_session_server`)
- `src/connection/mod.rs` (`send_response`)
- `tests/test_webtransport_draft_connect.rs` (draft07/14/15_pending_data_rejected の更新)
- 一次資料: `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 3.2
