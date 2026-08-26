# tokio-s2n-quic の WebTransport CONNECT 検証が欠落している

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-s2n-wt-connect-validation
- Polished: 2026-08-26

## 目的

CONNECT レスポンスの `:status` と、CONNECT リクエストの `:method` / `:protocol` を検証する。

## 現状

- クライアント側: `crates/tokio-s2n-quic/src/webtransport/client.rs` の `WtClient::connect` は `Event::HeadersEnd` を受けた時点で `session_established = true` とし、レスポンスの `:status` を検証しない。サーバーが 405 等で拒否 (`WtSessionRequest::reject`) してもセッション確立成功として `WtSession` を返す。draft-16 Section 3.2 はセッション確立を 2xx 受信時とする
- サーバー側: `webtransport/server.rs` の `WtSessionRequest::from_connection` は最初の bidi ストリームを無条件に CONNECT として扱い、`:path` / `:authority` しか収集しない。`:method = CONNECT` / `:protocol = webtransport-h3` の検証がないため、通常の GET リクエストでも `accept()` で 200 を返して WebTransport セッションと誤認する
- `examples/wt_server` 側は `ConnectRequest::from_headers` で検証しており、クレート本体だけが未検証

## 設計方針

- クライアント: CONNECT レスポンスの `:status` が 2xx のときのみセッション確立とする。sans-I/O 層は 2xx 時のみ `Event::WebTransport(WebTransportEvent::SessionEstablished)` を発行済み (`handle_wt_connect_response`) のため、このイベントを確立判定に使う (`:status` を直接検証しない。目的の「`:status` の検証」はこの委譲で満たされる)。非 2xx の場合は `crate::Error::ConnectionClosed` を返す (ステータスコード付きの新エラー variant は追加しない)。ただし、サーバーが fin なしで非 2xx を返した場合に `SessionEstablished` 待ちでハングしないよう、`:status` ヘッダーを監視して最終レスポンスの非 2xx を検出したら早期に `Err` を返す。1xx 中間レスポンス (例: 103 Early Hints。RFC 9114 Section 4.1) は失敗扱いせずスキップする (sans-I/O 層の `is_informational_status` と同様の区別)
- サーバー: `from_connection` で `:method` / `:protocol` を検証する。検証は既存の `ConnectRequest::from_headers` (examples で使用実績あり) を利用する。検証失敗時の応答は 2 経路に分かれる:
  - GET 等の非 WebTransport リクエスト (from_headers の `:method` / `:protocol` 検証失敗): ピアに 405 を返してから `Err` を返す (draft-16 Section 3.2 は非対応リソースへの応答を 405 SHOULD とする。0135 で 404 → 405 に変更済み)。405 送信は `reject()` が `self` を消費するため、`from_connection` 内では送信ロジックを共通ヘルパーとして抽出して使う
  - `:scheme` 不正等の WebTransport 形式のリクエスト: sans-I/O 層の `validate_wt_connect_request_server` が `Err` を返し、既存の `reset_stream_on_stream_error` 経路で RESET_STREAM になる (本 issue での追加対応は不要)
- `:scheme = https` の検証は sans-I/O 層 (`validate_wt_connect_request_server`) が既に行うため不要

## 完了条件

- サーバーが 405 で拒否した場合、クライアントがセッション確立失敗として扱う
- 通常の GET リクエストが WebTransport セッションとして受理されない
- テストが追加される (ループバック QUIC 統合テスト。モック・スタブは使わない。GET リクエストの送信は sans-I/O 層の `ClientConnection` を直接使うか、生 QUIC ストリーム + QPACK でヘッダーを手動構築する)
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `crates/tokio-s2n-quic/src/webtransport/client.rs` (`WtClient::connect`。`SessionEstablished` イベントでの確立判定と `:status` 監視による非 2xx の早期検出)
- `crates/tokio-s2n-quic/src/webtransport/server.rs` (`WtSessionRequest::from_connection` / `accept` / `reject`。`ConnectRequest::from_headers` による検証と 405 送信ヘルパーの抽出)
- 0156 と同一ファイル (`client.rs` / `server.rs` の同一関数) を変更するため、0156 の実装を取り込んでから作業する
- 一次資料: `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 3.2、`refs/webtrans/rfc8441.txt` Section 4
