# tokio-s2n-quic の WebTransport CONNECT 検証が欠落している

- Created: 2026-08-08
- Completed: 2026-08-27
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

### 変更内容

- `crates/tokio-s2n-quic/src/webtransport/client.rs` の `WtClient::connect`:
  - セッション確立判定を `Event::HeadersEnd` から `WebTransportEvent::SessionEstablished` に切り替え、sans-I/O 層の `handle_wt_connect_response` (2xx 時のみ SessionEstablished を発火する経路) に `:status` 検証を委譲する
  - `:status` ヘッダーを監視し、`HeadersEnd` の時点で 1xx なら次のレスポンスを待つためクリア、非 2xx / 非 1xx なら早期に `Error::ConnectionClosed` を返す
  - 200 OK + AlpnError で SessionClosed が発火するなど確立前にセッション終端イベントが届いたケースは早期エラー化する。ただし確立後 (SessionEstablished 発火後) に届いた `SessionClosed` / `SessionDraining` は `pending_wt_events` に転送し、`close_error_code` / `close_message` を保持したまま受信タスクへ引き継ぐ
  - ハンドシェイクループを `process_handshake_events` ヘルパーに抽出し、drain 側と process_stream_data 側で同じロジックを使う
- `crates/tokio-s2n-quic/src/webtransport/server.rs` の `WtSessionRequest`:
  - `from_connection` で全ヘッダーを `Vec<(Vec<u8>, Vec<u8>)>` に収集し、`ConnectRequest::from_headers` で `:method = CONNECT` / `:protocol = webtransport-h3 | webtransport` を検証する (draft-ietf-webtrans-http3-16 Section 3.2)
  - `InvalidMethod` / `InvalidProtocol` は 405 レスポンスをピアに返してから `Error::ConnectionClosed` を返す (draft-16 Section 3.2 の SHOULD)
  - `InvalidEncoding` は sans-I/O 側の `is_valid_field_value` が `obs-text` を許容するエッジケースで到達し得るため、`Error::Internal` として扱う
  - ハンドシェイクループを `collect_request_events` ヘルパーに抽出、CONNECT レスポンス送信を `send_reject_response` に共通化し `reject()` からも利用する
- `crates/tokio-s2n-quic/tests/webtransport_connect_validation_e2e.rs` を新規追加し、実 QUIC ループバックで `reject(405)` → クライアント側 `ConnectionClosed` を検証する統合テストを実装した
- `process_handshake_events` の単体テスト 9 件を追加した (1xx → 2xx、非 2xx、SessionEstablished 単発、確立前 SessionClosed / SessionDraining、確立後 SessionClosed 転送 (regression)、BufferedStreamRejected 転送等)
- `CHANGES.md` の `## develop` セクションに `[FIX]` エントリを追加した

### 対象外

- `:status` の詳細エラー variant (例: `RejectedByPeer(status)`) の追加は本 issue の設計方針で見送り (`ConnectionClosed` 単一で表現)
- 通常の GET リクエストが 405 で拒否されることの e2e 検証は sans-I/O 層の単体テスト (`test_connect_request_from_headers_invalid_method` / `test_connect_request_from_headers_invalid_protocol`) に委譲した
- FIN なし 405 レスポンスの受信検証、Err 復帰時の `send_stream.finish()` 呼び出し、`WtServer::bind` のバックグラウンドタスクリーク、`WtSessionRequest.recv_stream` の `stream_id` 絞り込み、`ConnectRequest::from_headers` のイテレータ化、`WtSessionRequest.path` / `authority` の `String` 化、e2e テストヘルパーの `tests/helpers` 共通化は別 issue で対応する

### 一次資料

- `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 3.2 (WebTransport CONNECT。2xx 受信で確立 / 非対応リソースへの 405 SHOULD)
- `refs/webtrans/rfc8441.txt` Section 4 (Extended CONNECT)
- `refs/h3/rfc9114.txt` Section 4.1 (1xx 中間レスポンスの扱い) / Section 4.5 (HTTP/3 で 101 を使わない)
