# tokio-s2n-quic の WebTransport CONNECT 検証が欠落している

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-s2n-wt-connect-validation
- Polished: {YYYY-MM-DD}

## 目的

CONNECT レスポンスの `:status` と、CONNECT リクエストの `:method` / `:protocol` を検証する。

## 現状

- クライアント側: `crates/tokio-s2n-quic/src/webtransport/client.rs` の `WtClient::connect` は `Event::HeadersEnd` を受けた時点で `session_established = true` とし、レスポンスの `:status` を検証しない。サーバーが 404 等で拒否 (`WtSessionRequest::reject`) してもセッション確立成功として `WtSession` を返す。draft-16 Section 3.2 はセッション確立を 2xx 受信時とする
- サーバー側: `webtransport/server.rs` の `WtSessionRequest::from_connection` は最初の bidi ストリームを無条件に CONNECT として扱い、`:path` / `:authority` しか収集しない。`:method = CONNECT` / `:protocol = webtransport-h3` の検証がないため、通常の GET リクエストでも `accept()` で 200 を返して WebTransport セッションと誤認する
- `examples/wt_server` 側は `ConnectRequest::from_headers` で検証しており、クレート本体だけが未検証

## 設計方針

- クライアント: CONNECT レスポンスの `:status` が 2xx のときのみセッション確立とする
- サーバー: `from_connection` で `:method` / `:protocol` を検証し、不正ならセッションを拒否する

## 完了条件

- サーバーが 404 で拒否した場合、クライアントがセッション確立失敗として扱う
- 通常の GET リクエストが WebTransport セッションとして受理されない
- テストが追加される
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `crates/tokio-s2n-quic/src/webtransport/client.rs` (`WtClient::connect`)
- `crates/tokio-s2n-quic/src/webtransport/server.rs` (`WtSessionRequest::from_connection` / `accept` / `reject`)
- 一次資料: `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 3.2
