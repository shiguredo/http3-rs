# Connection のストリーム / WT セッションが無制限に蓄積する

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-connection-resource-leak
- Polished: {YYYY-MM-DD}

## 目的

長時間接続のサーバーでメモリがリクエスト数・転送量・セッション数に比例して無制限に増加する問題を修正する。

## 現状

- `src/connection/mod.rs` の `Connection.streams` (`HashMap<u64, RequestStream>`) は `handle_bidirectional_stream` の `or_insert_with` と `send_request` の `insert` で追加されるのみで、完走 (StreamEnd) ・リセット (stream_reset) ・STOP_SENDING のいずれの経路でもエントリが除去されない (`Connection.streams` からの `remove` は存在しない)
- `src/stream/request.rs` の `process_raw` は DATA ペイロードを `recv_body` に無条件で `extend_from_slice` で累積するため、完走ストリームがボディ全体を保持し続ける (`RecvBuffer` はフレーム消費時に `consume` で drain されるため保持主体ではない)。`recv_body` は content-length 検証 (StreamEnd 時と QPACK ブロック解除時の 2 経路) に使われるが、検証後も解放されない
- `src/connection/wt_session.rs` の `Connection::terminate_wt_session_with` は状態を Closed にするだけで `Connection.wt_sessions` からエントリを除去しない。`associated_streams` / `capsule_buf` / `buffered_stream_entries` も保持し続ける

再現: 長時間稼働するサーバーにリクエストを送り続けると `streams` がリクエスト数に比例して増加する。WebTransport セッションでは CONNECT ストリームの `recv_body` が転送量に比例して増加する。

## 設計方針

- **`streams` からの除去は「ストリームが Reset になった時点」または「`StreamState::Closed` かつ送信バッファ完全消費済み」を条件にする**。`close_local()` は `send_body` / `send_response` の fin 指定時点 (send_buf に積んだ時点) で呼ばれ、その時点では応答が未送信のため、Closed になっただけでは除去できない。送信バッファの完全消費 (0138 の FIN 交付が済み、`take_stream_data` が None を返す状態) を除去条件に含める。StreamEnd (受信側 FIN) 時点や STOP_SENDING 受信時点では除去しない (サーバーが応答を送る必要がある / 受信側が open のまま)
  - Reset 時点の除去は、送信バッファに未交付のローカル送信データがある場合にそれを破棄する (RFC 9000 Section 3.2: RESET_STREAM は送信側と独立であり、ピアの RESET 受信後にローカル送信を継続しない)。除去後の `send_response` / `send_body` は `StreamNotFound` を返す (現在の `StreamClosed` からエラー種別が変わる)
  - 除去チェックは `process_stream_frames` のループ後・`feed_stream` の出口・`take_stream_data` / `consume_stream_data` で行う。クライアントは送信完了後に `take_stream_data` を呼び直さないため、受信経路 (ループ後) のチェックが必須
  - `process_stream_frames` のループ内では除去しない (ループ内に `expect("stream must exist while processing frames")` があり、除去すると panic する)
- **「送信バッファ完全消費」の検知は 0138 (FIN 交付の修正) に依存する**。0138 の修正で `take_stream_data` が FIN 交付後に None を返すようになるため、その状態を除去条件に使う。実装順序は 0138 → 0139 を想定する
- **WT CONNECT ストリームの `recv_body` には DATA を累積しない**。WebTransport の Capsule データは `handle_wt_data_frame` が処理するため、`recv_body` へのコピーは不要
  - content-length の扱い: RFC 9297 Section 3.2 は Capsule Protocol を使用するメッセージへの Content-Length ヘッダー付与を MUST NOT とし、違反を malformed と定める。したがって WT CONNECT では content-length ヘッダーの存在自体を `H3_MESSAGE_ERROR` で拒否する (「検証を対象外にする」ではなく「存在自体を拒否する」)。拒否チェックは StreamEnd 時 (`process_stream_frames`) と QPACK ブロック解除時 (`retry_blocked_streams`) の両方の検証経路に適用する
  - plain CONNECT の `recv_body` 累積は本 issue のスコープ外とする
- **セッション終了時の `wt_sessions.remove` は、終了済みセッション ID の記録 (tombstone) を接続終了まで保持したまま行う**。tombstone はセッション ID (u64) のみの軽量な記録であり、元の WtSession (associated_streams / capsule_buf / buffered_stream_entries) と比べて実質的なメモリ増加はない。tombstone により終了後に届く DATA / FIN / RESET / 新規ストリーム / データグラムを拒否し、`associate_or_buffer_stream` や `feed_datagram` の Pending セッション再生成 (zombie) と汎用イベント発行を防ぐ (draft-ietf-webtrans-http3-16 Section 6 の「WT_CLOSE_SESSION 後の追加データは H3_MESSAGE_ERROR」の MUST を維持する)
- **セッション終了時 (受信側の FIN / WT_CLOSE_SESSION / RESET / STOP_SENDING) に CONNECT ストリームも `streams` から除去する**。CONNECT は FIN 禁止のため両方向クローズに到達しないケースがあり、セッション終了を除去トリガに含める。除去は `process_stream_frames` のループ外 (遅延除去) で行う。ローカル側 FIN によるセッション終了経路は 0148 (ローカル側 CONNECT FIN でセッション終了) のスコープであり、0139 は受信側の終了経路を対象とする
- **除去済み stream_id への遅延データは破棄し、`streams` に再生成しない**。記録対象は終了済みセッションの ID (tombstone) のみ。通常ストリームは QUIC 層が FIN / RESET 後のデータを配達しないため、再生成防止の記録は不要
- 制御ストリーム・QPACK ストリームは `streams` に含まれないため対象外。`ignored_uni_streams` 等の他の無制限成長マップ、STOP_SENDING 受信後にピアが FIN / RESET を送らない残留は本 issue のスコープ外とする
- 0146 (バッファリング中 WT ストリームの stale エントリ) も `terminate_wt_session_with` を変更する。変更箇所は異なる (0146: buffered マッピング、0139: セッションエントリ自体) が、同一関数への同時変更となるため実装順序を調整する。0139 の tombstone 導入後に 0146 を実装する順序を想定する

## 完了条件

- ストリームが Reset になった時点、または `StreamState::Closed` かつ送信バッファ完全消費済み (FIN 交付済み) の時点で `Connection.streams` からエントリが除去される
- セッション終了後に `wt_sessions` からエントリが除去され、終了後に届いた DATA / FIN / RESET / 新規ストリーム / データグラムが現在と同じ挙動 (破棄 / 拒否) で処理される (zombie Pending セッションの再生成がない)
- セッション終了時に CONNECT ストリームが `streams` から除去される (ループ外の遅延除去で panic しない)
- tombstone はセッション ID のみの軽量な記録であり、接続終了まで保持する (元の WtSession エントリは解放される)
- 除去済み stream_id への遅延データが `streams` を再生成しない
- WebTransport セッション中に CONNECT ストリームの `recv_body` が増加しない
- content-length ヘッダー付きの WT CONNECT が `H3_MESSAGE_ERROR` で拒否される
- テストが追加・更新される: `src/connection/mod.rs` の `#[cfg(test)]` モジュールで `streams.len()` / `wt_sessions.len()` を構造的不変量として検証する (リクエスト完走 → 応答送信 → 除去、セッション終了 → 除去、終了後の到着データの拒否、除去済み ID への遅延データの破棄)。`wt_sessions[&session_id].state == Closed` を検証する既存テストは、除去後の「存在しないこと」の検証に更新する。`streams` / `wt_sessions` は private フィールドのため統合テスト (tests/) からは検証できない
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/connection/mod.rs` (`Connection::streams` / `stream_reset` / `stop_sending` / StreamEnd 処理 / `process_stream_frames` / `retry_blocked_streams` / `send_response` / `send_body` / `consume_stream_data` / `take_stream_data`)
- `src/connection/wt_session.rs` (`terminate_wt_session_with` / `associate_or_buffer_stream` / `handle_wt_stream_reset` / `handle_wt_stop_sending`)
- `src/connection/wt_capsule.rs` (`handle_wt_data_frame` / `handle_wt_stream_end` / `process_wt_capsule_data` の while ループ)
- `src/connection/wt_stream.rs` (`feed_datagram` の Pending セッション生成分岐)
- `src/stream/request.rs` (`process_raw` の `recv_body` 累積)
- 一次資料: `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 6 (セッション終了後のストリーム処理)、`refs/webtrans/rfc9297.txt` Section 3.2 (Capsule Protocol と Content-Length)
