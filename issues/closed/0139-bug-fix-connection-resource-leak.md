# Connection のストリーム / WT セッションが無制限に蓄積する

- Created: 2026-08-08
- Completed: 2026-08-08
- Branch: feature/fix-connection-resource-leak
- Polished: 2026-08-08

## 目的

長時間接続のサーバーでメモリがリクエスト数・転送量・セッション数に比例して無制限に増加する問題を修正する。

## 現状

- `src/connection/mod.rs` の `Connection.streams` (`HashMap<u64, RequestStream>`) は `handle_bidirectional_stream` の `or_insert_with` と `send_request` の `insert` で追加されるのみで、完走 (StreamEnd) ・リセット (stream_reset) ・STOP_SENDING のいずれの経路でもエントリが除去されない (`Connection.streams` からの `remove` は存在しない)
- `src/stream/request.rs` の `process_raw` は DATA ペイロードを `recv_body` に無条件で `extend_from_slice` で累積するため、完走ストリームがボディ全体を保持し続ける (`RecvBuffer` はフレーム消費時に `consume` で drain されるため保持主体ではない)。`recv_body` は content-length 検証 (StreamEnd 時と QPACK ブロック解除時の 2 経路) に使われるが、検証後も解放されない
- `src/connection/wt_session.rs` の `Connection::terminate_wt_session_with` は状態を Closed にするだけで `Connection.wt_sessions` からエントリを除去しない。`associated_streams` / `capsule_buf` / `buffered_stream_entries` も保持し続ける

再現: 長時間稼働するサーバーにリクエストを送り続けると `streams` がリクエスト数に比例して増加する。WebTransport セッションでは CONNECT ストリームの `recv_body` が転送量に比例して増加する。

## 設計方針

- **`streams` からの除去は「ストリームが Reset になった時点」または「`StreamState::Closed` かつ送信バッファ完全消費済み」を条件にする**。`close_local()` は `send_body` / `send_response` の fin 指定時点 (send_buf に積んだ時点) で呼ばれ、その時点では応答が未送信のため、Closed になっただけでは除去できない。送信バッファの完全消費 (「FIN 交付済み」) を除去条件に含める。StreamEnd (受信側 FIN) 時点や STOP_SENDING 受信時点では除去しない (サーバーが応答を送る必要がある / 受信側が open のまま)
  - Reset 時点の除去は `stream_reset` 内で行う。ピア RESET 後は QUIC 層が追加データを配達しないため、`feed_stream` の出口や `process_stream_frames` のループ後チェックは発火しない。送信バッファに未交付のローカル送信データがある場合はそれを破棄する (RFC 9114 Section 4.1.1 のリクエストキャンセル時の未クローズ方向の急停止 SHOULD、RFC 9000 Section 4.4 の RESET_STREAM は反対方向に影響しない)。除去後の `send_response` / `send_body` は `StreamNotFound` を返す (現在の `StreamClosed` からエラー種別が変わる)
  - 除去チェックは `process_stream_frames` のループ後・`feed_stream` の出口・`stream_reset`・`take_stream_data` / `consume_stream_data` で行う。クライアントは送信完了後に `take_stream_data` を呼び直さないため、受信経路 (ループ後) のチェックが必須
  - `process_stream_frames` のループ内では除去しない (ループ内に `expect("stream must exist while processing frames")` があり、除去すると panic する)
- **「送信バッファ完全消費」の検知は 0138 (FIN 交付の修正) に依存する**。0138 の修正で `take_stream_data` が FIN 交付後に None を返すようになるため、その状態を除去条件に使う。実装順序は 0138 → 0139 を想定する。除去条件を「データ全消費 (FIN 交付不要)」に緩める案は 0138 実装前 (0139 先行) の場合のみ成立し、0138 実装後は FIN が「データ消費後の追加呼び出しで `(空, fin=true)` として交付される」ため、データ全消費時点で除去すると FIN 交付の再呼び出しが None になり 0138 の完了条件「FIN が 1 回だけ交付され」を満たせなくなる。緩める案を採る場合は 0138 実装時に除去条件を「FIN 交付済み」へ移行する
  - **統合層 (tokio-s2n-quic) のドライブパターンとの整合を確認する**: `crates/tokio-s2n-quic/src/h3/server.rs` のレスポンス送信と `h3/client.rs` のリクエスト送信は `get_stream_data` を 1 回呼ぶだけで、FIN 交付 (追加呼び出し) をせず QUIC 側の `finish()` で代替する。このままではサーバーは応答送信後に `take_stream_data` を呼び直さず、FIN 交付による「送信バッファ完全消費」の除去条件が発火しない。統合層の送信ループを FIN 交付までドレインするよう更新するか、除去条件を「データ全消費 (FIN 交付不要)」に緩めるかを実装時に確定する (サーバーの `streams` リーク解消という本 issue の目的を達するためには、統合層の対応が必須)
- **WT CONNECT ストリームの `recv_body` には DATA を累積しない**。WebTransport の Capsule データは `handle_wt_data_frame` が処理するため、`recv_body` へのコピーは不要
  - 実現方法: `src/stream/request.rs` の `RequestStream` は `is_connect` フラグのみで WT CONNECT と plain CONNECT を区別できないため、`process_raw` 単独では判別できない。**サーバー側・クライアント側の両方で** WT CONNECT と判別できた時点で `RequestStream` に WT CONNECT フラグを追加する。判定は `is_webtransport_connect` 関数 (mod.rs。draft-02 互換の `:protocol = webtransport` も含む) を使う。クライアント側は `send_request` の戻り値 (`wt_sessions` 登録済みの CONNECT ストリーム ID) から、サーバー側は受信ヘッダーから判別する。`process_stream_frames` 側での `wt_sessions` 登録済み判定は `process_raw` の `recv_body` 累積が先行する構造上、非累積化を実現できないため使わない
  - **非累積化後の content-length 検証の扱い**: クライアント側の WT CONNECT レスポンスにも content-length が付き得るが、`recv_body` 非累積化で `body_size` が常に 0 になるため、StreamEnd 時 / QPACK ブロック解除時の `validate_content_length` (mod.rs) は誤った検証になる。WT CONNECT (リクエスト / レスポンス両方) では content-length 検証をスキップするか、サーバー側と同様にヘッダー受信時点で拒否する
  - content-length の扱い: RFC 9297 Section 3.2 は Capsule Protocol を使用するメッセージへの Content-Length / Content-Type / Transfer-Encoding ヘッダー付与を MUST NOT とし、違反を malformed と定める。したがって WT CONNECT では content-length ヘッダーの存在自体を `H3_MESSAGE_ERROR` で拒否する (「検証を対象外にする」ではなく「存在自体を拒否する」)。拒否チェックは **ヘッダー受信時点 (`validate_wt_connect_request_server`) で行う**。StreamEnd 時や QPACK ブロック解除時に合わせると、WT CONNECT のリクエスト側はセッション存続中 FIN されないため実質発火しない。Content-Type / Transfer-Encoding は本 issue の拒否対象外とする (Transfer-Encoding は接続固有ヘッダーとして全リクエストで既に拒否されるため。Content-Type は RFC 9297 の MUST NOT 対象だが、ヘッダー名の存在確認のみで追加できるため実装時に content-length と同様に拒否するか判断する。拒否対象を content-length のみに絞る場合もスコープ外であることを明示する)
  - plain CONNECT の `recv_body` 累積は本 issue のスコープ外とする (WT CONNECT と同じフラグ機構で回避可能だが、plain CONNECT の content-length 検証は `recv_body` を参照するため、非累積化は別途検証が必要)
- **セッション終了時の `wt_sessions.remove` は、終了済みセッション ID の記録 (tombstone) を接続終了まで保持したまま行う**。tombstone はセッション ID (u64) 集合のみの軽量な記録であり、元の WtSession (associated_streams / capsule_buf / buffered_stream_entries) と比べてメモリは遥かに小さいが、**セッション数に比例して接続終了まで成長し続ける** (目的の「無制限に増加する問題を修正する」はセッションあたりのメモリの話であり、tombstone 自体の無制限成長は本 issue のスコープで許容する)。tombstone により終了後に届く DATA / FIN / RESET / 新規ストリーム / データグラムを拒否し、`associate_or_buffer_stream` / `feed_datagram` / `feed_stream` の Pending セッション再生成 (zombie) と汎用イベント発行を防ぐ (draft-ietf-webtrans-http3-16 Section 6 の「WT_CLOSE_SESSION 後の追加データは H3_MESSAGE_ERROR」の MUST を維持する)
  - **WT CONNECT ストリームの遅延 DATA は「破棄」ではなく `H3_MESSAGE_ERROR` で拒否する**。`wt_sessions.remove` 後に同一 `feed_stream` バッファ内の後続 DATA が `process_stream_frames` のループで処理されると、`handle_wt_data_frame` はセッション不在で `Ok(false)` を返し (wt_capsule.rs)、`mod.rs` の `process_stream_frames` が汎用 `Event::Data` を発行してしまう。tombstone チェックは `feed_stream` のディスパッチ前だけでなく `process_stream_frames` 内の `handle_wt_data_frame` / `handle_wt_stream_end` 呼び出しにも照合し、終了済み CONNECT ストリームの DATA には `Error::StreamError(ErrorCode::MessageError)` を返す
  - **終了済み CONNECT ストリームの FIN (StreamEnd) は受理し、汎用 `Event::StreamEnd` の発行のみ抑止する**。draft-16 Section 6「An endpoint that sends a WT_CLOSE_SESSION capsule MUST immediately send a FIN on the CONNECT Stream」のとおり、WT_CLOSE_SESSION を含む DATA と FIN が同一バッファに連続するのは正常な終了手順であり、FIN を `H3_MESSAGE_ERROR` にしてはならない。`handle_wt_stream_end` への tombstone 照合は「FIN は受理して何もしない (汎用 StreamEnd イベントを発行しない)」ことを意味する
  - **終了後に届く RESET は汎用 `Event::StreamReset` を発行せず静かに無視する** (RESET に `H3_MESSAGE_ERROR` で応答すると RESET のループになるため)
  - 通常ストリーム (非 CONNECT) の遅延データは「破棄」でよい (QUIC 層が FIN / RESET 後のデータを配達しないため。配達された場合は破棄する)
- **セッション終了時 (受信側の FIN / WT_CLOSE_SESSION / RESET / STOP_SENDING) に CONNECT ストリームも `streams` から除去する**。CONNECT ストリームはセッション中 FIN を送らず受信側も open のままのため、両方向クローズに到達しないケースがあり、セッション終了を除去トリガに含める (RFC 9114 Section 4.4 の「The request stream remains open at the end of the request」、draft-16 Section 6 のセッション終了条件)。除去は `process_stream_frames` のループ外 (遅延除去。保留リストをループ後に drain する等) で行う。ループ外から呼ばれた `terminate_wt_session_with` (stream_reset / stop_sending 経由) は即時除去してよい。ローカル側 FIN によるセッション終了経路は 0148 (ローカル側 CONNECT FIN でセッション終了) のスコープであり、0139 は受信側の終了経路を対象とする
- **除去済み stream_id への遅延データは破棄し、`streams` に再生成しない (「破棄」は非 CONNECT の通常ストリーム限定。終了済み CONNECT ストリームへの遅延 DATA は前項のとおり `H3_MESSAGE_ERROR` で拒否する)**。tombstone チェックを `feed_stream` のディスパッチ前 (`dispatch_client_bidi_stream` / `handle_bidirectional_stream` の入口) に置く。CONNECT ストリームはセッション終了後も受信側が open のため QUIC 層が遅延データを配達し得る (`handle_bidirectional_stream` の `or_insert_with` で再生成され、DATA が汎用リクエストボディとして処理される)。通常ストリームは再生成防止の記録は不要 (QUIC 層が FIN / RESET 後のデータを配達しない前提。仮に配達された場合も破棄する。RFC 9000 Section 3.2 は RESET 後のデータ到着を許容している)
- 制御ストリーム・QPACK ストリームは `streams` に含まれないため対象外。`ignored_uni_streams` 等の他の無制限成長マップ、STOP_SENDING 受信後にピアが FIN / RESET を送らない残留は本 issue のスコープ外とする
- 0146 (バッファリング中 WT ストリームの stale エントリ) も `terminate_wt_session_with` を変更する。0139 が `wt_sessions` からエントリごと除去すれば、0146 の「セッション終了時にバッファリング中ストリームのマッピングを除去する」のうち `buffered_stream_entries` 分は 0139 に吸収される。ただし `wt_uni_streams` / `wt_bidi_streams` の stale マッピング (buffered ストリームのストリーム ID → セッション ID) は FIN 到着まで残るため、0146 の残作業 (reset / stop_sending 時の掃除、`deliver_buffered_streams` の中断保持、`wt_uni_streams` / `wt_bidi_streams` の掃除) は独立している。実装順序は 0139 の tombstone 導入後に 0146 を実装する順序を想定する

## 完了条件

- ストリームが Reset になった時点 (`stream_reset`)、または `StreamState::Closed` かつ送信バッファ完全消費済み (FIN 交付済み) の時点で `Connection.streams` からエントリが除去される
- セッション終了後に `wt_sessions` からエントリが除去され、終了後に届いた DATA / FIN / RESET / 新規ストリーム / データグラムが破棄・拒否される (zombie Pending セッションの再生成がない)
- セッション終了時に CONNECT ストリームが `streams` から除去される (ループ外の遅延除去で panic しない)
- tombstone はセッション ID のみの軽量な記録であり、接続終了まで保持する (元の WtSession エントリは解放される)
- 除去済み stream_id への遅延データが `streams` を再生成しない
- WebTransport セッション中に CONNECT ストリームの `recv_body` が増加しない
- content-length ヘッダー付きの WT CONNECT が `H3_MESSAGE_ERROR` で拒否される
- 統合層 (tokio-s2n-quic) の送信ループが FIN 交付までドレインする (または除去条件を緩める) 対応が入り、サーバーの `streams` リークが実際に解消される。統合層は sans-I/O 層が返す `StreamError` (終了済み CONNECT ストリームの DATA に対する `H3_MESSAGE_ERROR` 等) を QUIC 層の RESET_STREAM に変換してピアへ伝える必要がある
- テストが追加・更新される: `src/connection/mod.rs` の `#[cfg(test)]` モジュールで `streams.len()` / `wt_sessions.len()` を構造的不変量として検証する (リクエスト完走 → 応答送信 → 除去、stream_reset 経由の除去、セッション終了 → 除去、終了後の到着データの拒否、除去済み ID への遅延データの破棄)。`wt_sessions[&session_id].state == Closed` を検証する既存テストは、除去後の「存在しないこと」の検証に更新する。`streams` / `wt_sessions` は private フィールドのため統合テスト (tests/) からは検証できない
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 修正内容

- `Connection::remove_stream_if_done` を追加し、ストリームを次の 3 条件で `streams` から除去するようにした: Reset になった時点 (`stream_reset`) / `StreamState::Closed` かつ送信バッファ完全消費済み (FIN 交付済み、`RequestStream::is_send_complete`) / セッション終了済み (tombstone) の CONNECT ストリーム。除去チェックは `feed_stream` の出口 (エラー経路を含む) / `process_stream_frames` のループ後 / `stream_reset` / `stop_sending` / `consume_stream_data` に配置した
- `terminate_wt_session_with` が `wt_sessions` からエントリを除去し、終了済みセッション ID を `closed_wt_sessions` (tombstone) に記録するようにした。終了後に届いた DATA (`handle_wt_data_frame` / `handle_bidirectional_stream` 入口) は `H3_MESSAGE_ERROR`、FIN は受理して汎用 StreamEnd を抑止、RESET / STOP_SENDING は静かに無視、新規ストリーム (`associate_or_buffer_stream`) とデータグラム (`feed_datagram`) は拒否・破棄し zombie Pending セッションの再生成を防ぐ
- `RequestStream` に WT CONNECT フラグ (`is_wt_connect`) を追加し、WT CONNECT ストリームの DATA を `recv_body` に累積しないようにした。content-length / content-type ヘッダー付きの WT CONNECT (リクエスト送信 / サーバー受信 / レスポンス送受信) と 204 / 205 / 206 レスポンスを `H3_MESSAGE_ERROR` で拒否する (RFC 9297 Section 3.2)
- `stop_sending` 受信時に送信バッファを破棄 (`SendBuffer::discard`) し、QPACK ブロック状態をクリアするようにした (リーク経路の解消)
- 統合層 (tokio-s2n-quic) の送信経路 4 箇所を FIN 交付までドレインするループに更新し、受信ループ 4 箇所で `StreamError` を RESET_STREAM に変換してピアへ伝えるようにした

### テスト

- `src/connection/mod.rs` の `#[cfg(test)]` に 20 本以上を追加した (ストリーム除去 3 条件 / セッション終了後の拒否・破棄 6 経路 / recv_body 非累積 2 方向 / content-length・content-type・204 拒否 / STOP_SENDING 経路 / 通常 204 の回帰)
- 既存テスト 4 本の `wt_sessions[&id].state == Closed` 検証を「除去後の存在しないこと」に更新した

### 関連ファイル

- `src/connection/mod.rs` (`Connection::streams` / `stream_reset` / `stop_sending` / StreamEnd 処理 / `process_stream_frames` / `retry_blocked_streams` / `feed_stream` / `dispatch_client_bidi_stream` / `handle_bidirectional_stream` / `send_response` / `send_body` / `consume_stream_data` / `take_stream_data`)
- `src/connection/wt_session.rs` (`terminate_wt_session_with` / `associate_or_buffer_stream` / `handle_wt_stream_reset` / `handle_wt_stop_sending` / `validate_wt_connect_request_server` / `wt_sessions.remove` と tombstone)
- `src/connection/wt_capsule.rs` (`handle_wt_data_frame` / `handle_wt_stream_end` / `process_wt_capsule_data` の while ループ)
- `src/connection/wt_stream.rs` (`feed_datagram` の Pending セッション生成分岐)
- `src/stream/request.rs` (`process_raw` の `recv_body` 累積 / WT CONNECT フラグ)
- 統合層: `crates/tokio-s2n-quic/src/h3/server.rs` / `h3/client.rs` / `webtransport/server.rs` (送信ループの FIN 交付ドレイン)
- 一次資料: `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 6 (セッション終了後のストリーム処理)、`refs/webtrans/rfc9297.txt` Section 3.2 (Capsule Protocol と Content-Length / Content-Type / Transfer-Encoding)、`refs/quic/rfc9000.txt` Section 3.2 / 4.4、`refs/h3/rfc9114.txt` Section 4.1.1 / 4.4
