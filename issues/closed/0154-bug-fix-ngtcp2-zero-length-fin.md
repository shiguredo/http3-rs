# tokio-ngtcp2 の 0 長データ + FIN が破棄され終端が送信されない

- Created: 2026-08-08
- Completed: 2026-08-16
- Branch: feature/fix-ngtcp2-zero-length-fin
- Polished: 2026-08-15

## 目的

nghttp3 の「0 長データ + FIN」(fin のみ) を送信できない問題を修正する。データ送信後の空データ + FIN 呼び出し (クライアントの `send_body(stream_id, &[], true)` / WebTransport クライアントの `send_stream_data(sid, &[], true)` / WebTransport サーバーの `send_stream_data_for` / `send_stream_data_by_conn_id`) でピアに終端が届かない。データ送信を完了した側はストリームの送信方向をクローズして終端を通知する必要がある (HTTP リクエスト/レスポンスは RFC 9114 Section 4.1。WebTransport データストリームも同様に終端を通知する)。

## 現状

- 送信ループ (クライアント: `crates/tokio-ngtcp2/src/client.rs` の `write_h3_streams`、サーバー: `server.rs` の `write_and_send_h3_streams`、WebTransport: `webtransport.rs` の `write_h3_streams` / `write_h3_streams_tracked` / `write_h3_streams_for_wt_connection`、テストヘルパー: `tests/helpers/multi_conn_client.rs` の `write_h3_streams`) は `count == 0` で break する
- `crates/nghttp3-sys/src/bindings.rs` の `nghttp3_conn_writev_stream` ドキュメントは「count が 0 で `*pfin` が非ゼロのケース (0 長データ + FIN) は QUIC スタックに渡し、`nghttp3_conn_add_write_offset` を 0 バイトで呼ぶこと」と規定しており、このケースで FIN が送信されない (データ送信後に `send_body(stream_id, &[], true)` / `send_stream_data(sid, &[], true)` で fin のみを送る経路、WT サーバーの `send_stream_data_for` / `send_stream_data_by_conn_id` で fin のみを送る経路が壊れる。サーバーの `send_response` による空ボディレスポンスは、ヘッダーが小さければ HEADERS と fin が同一の `write_stream` 呼び出しで返るため影響を受けない。ヘッダーが nghttp3 内部の `NGHTTP3_MIN_UNSENT_BYTES` (圧縮後のヘッダーサイズ基準。現在は 16382 バイトでバージョンにより変わり得る) 以上の場合のみ fin のみに分離して影響を受ける)
- fin のみはデータ送信完了後の追加呼び出しで発生する (例: `send_request_body(stream_id, &[], true)` の後、read コールバックが 0 バイト + EOF を返し、`write_stream` が (stream_id, fin=true, count=0) を返す)。この経路が `count == 0` の break で破棄される
- `data_written > 0` のガードにより、fin のみのケースの `add_write_offset(stream_id, 0)` もスキップされ、nghttp3 が FIN 送信完了を認識しない
- `webtransport.rs` の `write_h3_streams` / `write_h3_streams_tracked` と `multi_conn_client.rs` の `write_h3_streams` は `if h3_data.is_empty() { continue; }` と fin を考慮しておらず、`count == 0` の break を直してもこのガードが fin のみケースを破棄する (`client.rs` / `server.rs` / `write_h3_streams_for_wt_connection` は fin 考慮済み)
- テストは「データ + FIN 一体型」のみで、FIN のみの送信経路が検出されていない

## 設計方針

- `count == 0` でも `fin == true` の場合は処理を継続して QUIC に FIN を渡す
- `h3_data.is_empty() && !fin` の continue ガードを全関数で統一する (`webtransport.rs` の `write_h3_streams` / `write_h3_streams_tracked` と `multi_conn_client.rs` の `write_h3_streams` に fin 判定を追加する。count == 0 の break 修正は全関数に適用する)
- `data_written == Some(0)` でも fin が立っている場合は `add_write_offset(stream_id, 0)` を呼ぶ (`h3_conn.write_stream()` の戻り値の fin を使う。`data_written == None` の輻輳ブロックでは呼ばない)
- `write_h3_streams_tracked` の `else if pkt_written == 0` による block 判定は、fin のみ送信 (`data_written = Some(0)`) を輻輳ブロックと誤判定しないよう再構成する

## 完了条件

- 0 バイトデータ + FIN の送信 (クライアントの `send_body(stream_id, &[], true)` / WebTransport クライアントの `send_stream_data(sid, &[], true)` / WebTransport サーバーの `send_stream_data_for` / `send_stream_data_by_conn_id`) でピアに StreamEnd が届く
- テストが追加される (fin のみ送信の経路。HTTP/3 クライアント・WebTransport クライアント・WebTransport サーバー)。fin のみ経路 (count=0, fin=1) はストリームの outq が空の状態でのみ発生するため、テストは「先にデータ (fin=false) を送信してフラッシュし、その後空データ + FIN を別呼び出しで送る」2 段階シーケンスを基本とすること。新規ストリームへの初回の空 + FIN 送信は、未送信の HEADERS / WT_STREAM ヘッダーと fin が一体で送られ (count>=1, fin=1) 修正前でも通る場合があり、fin のみ経路の検証にならない
- CHANGES.md の `## develop` セクションに変更履歴が記載される
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `crates/tokio-ngtcp2/src/client.rs` (`write_h3_streams`)
- `crates/tokio-ngtcp2/src/server.rs` (`write_and_send_h3_streams`)
- `crates/tokio-ngtcp2/src/webtransport.rs` (`write_h3_streams` / `write_h3_streams_tracked` / `write_h3_streams_for_wt_connection`)
- `crates/tokio-ngtcp2/tests/helpers/multi_conn_client.rs` (同一バグパターンを持つテストヘルパー)
- 参照: `crates/nghttp3-sys/src/bindings.rs` (`nghttp3_conn_writev_stream` / `nghttp3_conn_add_write_offset` のドキュメント)
- スコープ外: `crates/ngtcp2-rs/tests/webtransport_sans_io.rs` の `write_h3_to_packets` (ngtcp2-rs 側のテストヘルパー。count == 0 の break と `data_written > 0` のガードで fin のみケースが破棄される点は同一だが、本 issue の対象は tokio-ngtcp2 のみ)
