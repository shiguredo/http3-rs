# WebTransport セッションに Draining 状態がなく送信抑止が Connection 層で表現されない

Created: 2026-04-07
Completed: 2026-04-07
Model: Opus 4.6

## 概要

`WtSessionState` は `Pending` / `Established` / `Closed` の 3 状態しか持たない。`WT_DRAIN_SESSION` カプセル受信時およびクライアントが GOAWAY を受信した時、`Event::WebTransportSessionDraining` を発行するが、内部 state は `Established` のままになる。

このため Connection 層の送信 API (`send_datagram` など) は draining 後も `Established` チェックだけを通し、新規データグラム送信を許可してしまう。draft-ietf-webtrans-http3-15 Section 6 が要求する「draining されたセッションでは新規ストリーム/データグラムの送信を停止する」という挙動は、現状ラッパー任せになっており、Sans I/O 層の責務分離が崩れている。

加えて、新規 WT bidi/uni stream を Connection 層から開始する経路 (`open_wt_uni_stream` 等が今後追加される場合) も同じ問題を抱える。

## 該当箇所

- `src/connection/mod.rs` `WtSessionState` 定義 (現在 L172 付近)
- `src/connection/mod.rs` `Connection::send_datagram` (現在 L866 付近)
- `src/connection/mod.rs` GOAWAY 受信時の WT draining 通知 (現在 L2417 付近)
- `src/webtransport/session.rs` `WT_DRAIN_SESSION` 処理経路 (capsule decode 後の event 発行)
- `src/event.rs` `Event::WebTransportSessionDraining` (現在 L142 付近)

## 根拠

- draft-ietf-webtrans-http3-15 Section 6: `WT_DRAIN_SESSION` を受けたエンドポイントは新規ストリーム/データグラムを開始してはならない。既存のストリームは継続できる。
- draft-ietf-webtrans-http3-15 Section 4.7 / RFC 9114 Section 5.2: クライアントは GOAWAY を受けると新規 WT セッションを開始できなくなり、既存セッションは draining 扱いとなる。
- `refs/webtrans/draft-ietf-webtrans-http3-15.txt`

## 修正方針

破壊的変更を許容する。

1. `WtSessionState` に `Draining` を追加し、`Pending` / `Established` / `Draining` / `Closed` の 4 状態とする。状態遷移は以下:
   - `Pending` → `Established` (200 OK)
   - `Pending` → `Closed` (CONNECT 拒否)
   - `Established` → `Draining` (`WT_DRAIN_SESSION` 受信、または GOAWAY 起因で session_id がしきい値以上)
   - `Established` / `Draining` → `Closed` (`WT_CLOSE_SESSION` / FIN / RESET_STREAM)
   - `Pending` → `Draining` も認める (GOAWAY が pending session_id をカバーする場合)
2. `send_datagram` の状態チェックを `state == Established` から `state == Established || state == Draining` の許可に変えるのではなく、「`Draining` 状態では新規データグラムを拒否する」明示的な分岐に変える。Section 6 は「既存ストリームは継続」と書いているが、データグラムは独立した送信単位なので新規送信扱いとして拒否する方が安全。エラーは `Error::ConnectionError(ErrorCode::GeneralProtocolError)` ではなく WebTransport 固有のエラー種別 (例: `Error::WtSessionDraining`) を新設する。
3. WT 新規 bidi/uni stream のオープン API (現在/将来) でも `Draining` を拒否する。
4. `WT_DRAIN_SESSION` 受信処理および GOAWAY 受信処理で、イベント発行の前に `session.state = Draining` を設定する。`Closed` セッションへの再 draining 適用は no-op とする。
5. `Event::WebTransportSessionDraining` のセマンティクスをドキュメントに明記する: 「Connection 層は内部状態を `Draining` に移し、以後の新規送信を拒否する。上位層もアプリレベルで新規ストリーム作成を停止すること」。
6. `CHANGES.md` に `[CHANGE]` として記載する。
7. テストで以下を追加する:
   - `Established` セッションで `WT_DRAIN_SESSION` を受けた後に `send_datagram` が拒否される
   - クライアントが GOAWAY を受けた後、対象 session_id 以上の `Established` セッションが `Draining` に遷移し `send_datagram` が拒否される
   - `Draining` 状態のセッションが `WT_CLOSE_SESSION` / FIN を受けた場合に正しく `Closed` へ遷移する
   - `Draining` 状態でも既存ストリームの `send_*` (将来の API) と Capsule 受信処理は許可される

## 補足

`send_datagram` のエラー型変更は API 影響があるため、`Error` の variant 追加と関連箇所の更新を含めて 1 issue で扱う。`Error::WtSessionDraining` を新設するか既存の variant を再利用するかは実装時に判断する (`Error` 設計と整合させること)。

## 解決方法

- `src/error.rs` に `Error::WtSessionDraining(u64)` バリアントを追加した。HTTP/3 接続/ストリームエラーではなく、WebTransport セッションがグレースフルシャットダウン中であることを示すローカル API レベルのエラーとして扱う。
- `src/connection/mod.rs` の `WtSessionState` に `Draining` を追加し、状態遷移経路を以下のように整理した:
  - `WT_DRAIN_SESSION` カプセル受信: `Established` / `Pending` から `Draining` に遷移し、`Event::WebTransportSessionDraining` を発行する。
  - クライアントが GOAWAY を受信: 該当 `session_id` 以上の `Established` / `Pending` セッションを `Draining` に遷移させてからイベントを発行する。
  - `WT_CLOSE_SESSION` / FIN / RESET_STREAM 受信: `Draining` 状態でも `terminate_wt_session_with` 経由で `Closed` に遷移する (既存ロジックは `Closed` のみを no-op として弾いていたためそのまま動作)。
- `Connection::send_datagram` に明示的な `Draining` 分岐を追加し、`Error::WtSessionDraining(session_id)` を返すようにした。
- `Connection::feed_datagram` のルーティングでは `Established | Draining` を許可し、Draining 中も既存ストリーム/データグラム受信を継続できるようにした (Section 6 の「既存ストリームは継続」を遵守)。
- `Connection::associate_or_buffer_stream` の `Draining` 分岐を追加し、新規 WT データストリームの関連付けを拒否するようにした。
- CONNECT ストリーム上の DATA フレーム (Capsule) 受信処理は `Established | Draining` を許可し、Draining 中も `WT_CLOSE_SESSION` 等のカプセルを受信できるようにした。
- `count_active_wt_sessions` に `Draining` を加え、フロー制御無効時の同時 1 セッション制限の slot を Draining 中も消費するようにした。
- 単体テストを 3 件追加した:
  - `test_wt_drain_session_transitions_to_draining_and_blocks_send_datagram`
  - `test_wt_drain_session_then_close_session_transitions_to_closed`
  - `test_client_goaway_transitions_wt_session_to_draining`
- `CHANGES.md` の `## develop` に `[CHANGE]` として記載した。
