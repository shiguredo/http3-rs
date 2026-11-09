# 受信リセットイベント WebTransportEvent::StreamReset に Reliable Size を載せるか検討する

- Created: 2026-11-09
- Completed: {YYYY-MM-DD}
- Branch: feature/add-wt-stream-reset-reliable-size
- Polished: {YYYY-MM-DD}
- Category: design

## 目的

WebTransport データストリームの受信リセットを通知する `WebTransportEvent::StreamReset` は `final_size` のみを運び、`RESET_STREAM_AT` でピアが配送保証を宣言した先頭バイト数 (`Reliable Size`) を運べない。一方で送信側の終了経路である `WebTransportEvent::SessionClosed` の `reset_streams` (要素 `WtStreamReset`) は `reliable_size` を運んでおり、イベント API として非対称になっている。受信イベントへ `Reliable Size` を載せる価値と下位 QUIC API での取得可否を含めて検討し、載せる / 載せないの判断を記録する。

## 現状

- `src/event.rs` の `WebTransportEvent::StreamReset` は `session_id` と `stream_id` と `error_code` と `final_size` の 4 フィールドである。Reliable Size は運ばない
- イベントの発火点は `src/connection/wt_session.rs` の `Connection::handle_wt_stream_reset` である。セッション終了時のリセットは `Connection::terminate_wt_session_with` が組み立てる `SessionClosed.reset_streams` に分離されている
- 受信イベントの `final_size` はデータフロー制御の計上 (`Connection::account_wt_stream_reset`) に使用される。ピアが保証を宣言した区間を表す `Reliable Size` は、ストリームヘッダー長にピアの任意オフセットを加えた値であり、そのオフセットはピアしか知らない。したがってこの層では下位 QUIC 層が frame から取得して伝える以外に値を用意できない
- draft-17 Section 4.4 は送信者への MUST (`Reliable Size` = ヘッダー長 + アプリ指定オフセットで RESET_STREAM_AT を送る) と中継者への MUST (同一 frame 型の送出とヘッダー長差の補正) を定める。受信者へ `Reliable Size` を引き渡す要求は無いため、搭載は仕様義務ではなく統合層の要否で決まる判断である
- `refs/quic/draft-ietf-quic-reliable-stream-reset-11.txt` Section 4 は RESET_STREAM_AT frame に `Reliable Size` フィールドを定める。0 も有効値のため、値を取得できない環境で 0 を既定値にすると `Reliable Size = 0` の受信と区別が付かない
- 下位 QUIC API の現状は次のとおりであり、両経路での `Reliable Size` 取得可否は未確認である
  - `crates/tokio-s2n-quic/src/internal/connection_state.rs` の `stream_reset_only` は、s2n-quic の `stream::Error` が Final Size を公開しないため final_size を常に 0 で渡す
  - `crates/tokio-ngtcp2/src/h3.rs` の `stream_reset` は、下位層から `stream_id` と `error_code` と final_size を取得する

## 設計方針

- 判断の材料として次を扱う
  - 仕様は受信者への搭載を要求しない (draft-17 Section 4.4)。載せる動機は、受信アプリへの配送保証区間の明示と、`SessionClosed.reset_streams` との対称性にある
  - 下位 QUIC API で受信した `Reliable Size` を取得できない経路がある場合、載せたフィールドは値として意味を持たない。実装着手時に固定依存 `shiguredo_ngtcp2_tokio` と s2n-quic の公開 API を確認する
  - 載せる場合、取得不能の表現の候補は `reliable_size: Option<u64>` である (0 は有効値のため 0 既定の数値は不可)
- 載せると判断した場合、公開 API 変更として CODEBASE.md の「公開 API は必ず使用箇所とテストを用意すること」を満たす使用箇所とテストを用意する。下位 API が揃わない場合は pending issue 0229 と同様の形式で pending の理由を記録する
- 載せないと判断した場合、その理由を `src/event.rs` の `WebTransportEvent::StreamReset` のドキュメントに追記し、`reset_streams` との非対称が意図的であることを残す

## 完了条件

- 載せる / 載せないの判断と理由が本 issue に記録されている
- 載せる場合、下位 QUIC API での受信 `Reliable Size` の取得可否が確認され、取得不能経路の扱い (Option 化を含む表現) が設計として定まっている
- 載せる場合、公開 API としての使用箇所とテストが CODEBASE.md の要件どおり用意されている
- 載せない場合、意図的な非対称である理由が `src/event.rs` のドキュメントに記録される

## 関連ファイル

- `src/event.rs`: `WebTransportEvent::StreamReset`, `WtStreamReset`
- `src/connection/wt_session.rs`: `Connection::handle_wt_stream_reset`, `Connection::terminate_wt_session_with`, `Connection::account_wt_stream_reset`, `Connection::wt_stream_header_len`
- `crates/tokio-ngtcp2/src/h3.rs`: `stream_reset` の呼び出し元と下位コールバック契約
- `crates/tokio-s2n-quic/src/internal/connection_state.rs`: `stream_reset_only`

## 一次資料

- `refs/webtrans/draft-ietf-webtrans-http3-17.txt` Section 4.4: RESET_STREAM_AT 送信 MUST と中継者の `Reliable Size` 補正。受信者への要求の不在
- `refs/quic/draft-ietf-quic-reliable-stream-reset-11.txt` Section 4: RESET_STREAM_AT frame と `Reliable Size` (0 が有効値である点を含む)
- `issues/pending/0229-wt-h3-draft-17-partial-reset-offset.md`: 送信側の `Reliable Size` 発行 API (ヘッダー長 + アプリ指定オフセット)。本 issue とは目的を分ける
