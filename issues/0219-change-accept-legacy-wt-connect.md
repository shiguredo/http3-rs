# h3-webtransport (draft-02) クライアントの WebTransport CONNECT を受理する

- Created: 2026-09-18
- Completed: {YYYY-MM-DD}
- Branch: feature/change-accept-legacy-wt-connect
- Polished: {YYYY-MM-DD}

## 目的

quinn の h3-webtransport (0.1.2、draft-02) のように `SETTINGS_WT_ENABLED` を送らないクライアントからも WebTransport セッションを確立できるようにする。現在はサーバーが CONNECT を H3_MESSAGE_ERROR で拒否するため、この組み合わせだけ相互運用できない。

## 現状

- `src/connection/wt_session.rs` の `validate_wt_connect_request_server` は、ローカル設定のドラフトが Draft15 の場合にピアが `is_webtransport_enabled()` でなければ H3_MESSAGE_ERROR を返す
- h3-webtransport 0.1.2 は extended CONNECT (`:protocol=webtransport`) を送るが WebTransport 固有の SETTINGS を送らない。そのため `interop/wt/tests/quinn_client_ngtcp2_server.rs` は CONNECT が拒否されることを期待するテストになっており、`interop/wt/README.md` の相互運用性マトリクスも quinn クライアント → ngtcp2 サーバーを NG としている
- `interop/wt/tests/ngtcp2_client_quinn_server.rs` (ngtcp2 クライアント → quinn サーバー) は draft-02 のネゴシエーションで成功している。サーバー側だけが拒否する非対称な状態になっている
- 旧実装 (nghttp3) にはこの検証が無く、セッションを確立できていた

## 設計方針

- draft 検証をローカル設定ではなく相互広告されたドラフトで行う。相互広告が無いピアは draft-02 相当として扱い、`SETTINGS_H3_DATAGRAM` と transport parameter の前提を満たせば CONNECT を受理する
- 緩和する場合、`src/connection/mod.rs` や `src/connection/wt_session.rs` のドラフト検証テスト (`test_server_wt_connect_rejected_when_peer_wt_disabled` 等) を交渉結果に基づく期待値へ更新する
- 緩和しない判断をする場合は、この組み合わせの相互運用性を失うことを README とマトリクスに明記する (現状どおり)

## 完了条件

- quinn (h3-webtransport) クライアントからの CONNECT が受理され、`interop/wt/tests/quinn_client_ngtcp2_server.rs` がセッション確立成功を検証する
- draft-15 の WebTransport 検証テストが引き続き通る (緩和内容に合わせて更新される場合はその理由がテストコメントに書かれている)
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/connection/wt_session.rs` (`validate_wt_connect_request_server` / `mutually_advertised_wt_drafts` / `negotiated_wt_draft_version`)
- `src/connection/mod.rs` (WebTransport CONNECT 検証のテスト)
- `interop/wt/tests/quinn_client_ngtcp2_server.rs`
- `interop/wt/README.md`
- 一次資料: `refs/webtrans/draft-ietf-webtrans-http3-15.txt` Section 3.1 (Establishing a Session)、Section 7.1 (Version Negotiation)
