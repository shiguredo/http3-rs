# フロー制御無効時の WebTransport セッション同時数制限が未実装

Created: 2026-04-07
Completed: 2026-04-07
Model: Opus 4.6

## 概要

draft-ietf-webtrans-http3-15 Section 5.1 は、WebTransport のフロー制御が両端で有効化されていない場合、クライアントは同時に複数の WebTransport セッションを確立してはならず (MUST NOT)、サーバーは超過分の CONNECT ストリームを `H3_REQUEST_REJECTED` で reset しなければならない (MUST) と規定している。

現在の実装はこの両方の MUST を満たしていない。

- クライアント側: `Connection::send_request` は WebTransport CONNECT に対する各種前提条件 (peer の `SETTINGS_WT_ENABLED` / `H3_DATAGRAM` / `reset_stream_at` 等) は検証するが、フロー制御無効時に既存の Pending または Established な WT セッションが存在していても無条件で `wt_sessions.insert` してしまう。
- サーバー側: `Connection::send_response` の Pending → Established 遷移 (`is_success_status_raw` ブランチ) も既存セッション数を確認せず、フロー制御無効時に複数セッションが同時に Established になりうる。

直近で導入した `WT_MAX_PENDING_SESSIONS` (issue #0049) は Pending セッションの DoS 上限であり、本件の「FC 未交渉時は同時 1 個まで」という規範とは独立した別の制約である。

## 該当箇所

- `src/connection/mod.rs` `Connection::send_request` (WebTransport CONNECT 検証ブロック、現在 L3128-3159 および L3216-3229)
- `src/connection/mod.rs` `Connection::send_response` (WebTransport セッション Established 遷移、現在 L3320-3338)
- `src/connection/mod.rs` 受信側 (`recv_headers` の WebTransport CONNECT 受理処理、現在 L2671-2694) — server がリクエストを受理する時点で `H3_REQUEST_REJECTED` を返す必要があるかは要検討

## 根拠

draft-ietf-webtrans-http3-15 Section 5.1 (L1016-1020):

> If flow control is not enabled, clients MUST NOT attempt to establish more than one simultaneous WebTransport session. A server that receives more than one session on an underlying transport connection when flow control is not enabled MUST reset the excessive CONNECT streams with a H3_REQUEST_REJECTED status (see Section 5.2).

加えて Section 5.2 (L1046-1047):

> An endpoint that does not support pooling and flow control MUST NOT accept more than one incoming WebTransport session at a time.

## 修正方針案

- フロー制御の有効/無効は `Connection::is_wt_flow_control_enabled` で既に判定できる。両端の SETTINGS が確定したあと (= peer SETTINGS 受信後) でなければクライアントは WT CONNECT を送らない既存ガードに乗る。
- 「同時セッション数」の定義として Pending と Established の合算を採用する。仕様文面の "establish" / "session" は CONNECT 送信時点から開始しているとも読めるため、安全側に倒す。
- クライアント: `send_request` で WebTransport CONNECT のとき、`is_wt_flow_control_enabled() == false` かつ `wt_sessions` に他の Pending/Established なエントリがあれば `Error::ConnectionError(ErrorCode::RequestRejected)` 相当で拒否する。
- サーバー: `recv_headers` で WebTransport CONNECT を受理する時点で同条件を検査し、超過時は対象ストリームを `H3_REQUEST_REJECTED` で reset する (Established への遷移ではなく受理段階で拒否する)。
- テスト: PBT で「FC 無効 SETTINGS の組合せ + 複数 WT CONNECT」を生成し、2 本目以降が拒否されることを確認する。FC 有効時は従来通り複数同時セッションが許容されることも確認する。

## 注意点

- 「同時」の解釈 (Pending を含めるか否か) は draft-15 が明示していない。Pending を含める安全側の実装を採用する旨を実装コメントに残す。
- 将来 draft 改訂で文面が変わる可能性があるため、判定箇所には draft-ietf-webtrans-http3-15 Section 5.1 への参照と「将来変更される可能性がある」旨のコメントを付ける。

## 解決方法

draft-ietf-webtrans-http3-15 Section 5.1 / 5.2 に従い、WebTransport フロー制御が両端で有効化されていない場合、active な WebTransport セッション (Pending + Established) を 1 個に制限する処理を追加した。

- `Connection::count_active_wt_sessions()` を新設し、Pending と Established の合計を返すようにした。「同時 (simultaneous)」の解釈はドラフトが明示していないため、CONNECT 送信/受信の時点でセッションは確立中とみなす安全側の解釈を採用している。
- `Connection::send_request()` の WebTransport CONNECT 検証ブロックで、`is_wt_flow_control_enabled() == false` かつ `count_active_wt_sessions() >= 1` の場合に `Error::ConnectionError(ErrorCode::RequestRejected)` を返すようにした。
- `Connection::recv_headers()` の WebTransport CONNECT 受理ブロックで、同条件かつ「対象 stream_id に対応する Pending セッションが既に先着ストリーム経由で存在する場合は除外」したうえで、超過分の CONNECT を `Error::StreamError(ErrorCode::RequestRejected)` で拒否するようにした。
- `tests/test_webtransport_draft_connect.rs` に `no_flow_control_single_session` モジュールを追加し、サーバー / クライアント双方で 2 本目の WT CONNECT が `H3_REQUEST_REJECTED` で拒否されることを検証する単体テストを追加した。
