# tokio-ngtcp2 サーバーが単一パケットでプロセス全体停止する

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-ngtcp2-server-packet-dos
- Polished: 2026-08-08

## 目的

不正パケット 1 個でサーバー全体が終了するリモート DoS 経路を修正する。

## 現状

`crates/tokio-ngtcp2/src/server.rs` の `Server::run` は、接続単位で処理すべき ngtcp2 / nghttp3 のエラーを `?` で `run()` 全体に伝播させるため、単一パケット・単一イベントでサーバーループが終了する。停止経路は次の 3 つ:

1. **新規接続パスの `read_pkt` エラー** (server.rs:306、webtransport.rs:1075 も同構造): 任意のアドレスから送られた不正な Initial 1 個で `NGTCP2_ERR_DROP_CONN` 等の負エラーが返り、`run()` が Err 終了する。ngtcp2 の API 契約上、`NGTCP2_ERR_RETRY` / `NGTCP2_ERR_DROP_CONN` / `NGTCP2_ERR_DRAINING` / `NGTCP2_ERR_CLOSING` は接続単位の非致命的エラーであり、サーバー全体を停止させてはならない (bindings.rs の `ngtcp2_conn_read_pkt` の doc)
2. **既存接続パスの `read_pkt` エラー** (server.rs:177): 同一 `SocketAddr` からの新規 Initial は既存接続の `read_pkt` に渡される。CID 不一致は ngtcp2 内部で `NGTCP2_ERR_DISCARD_PKT` として握りつぶされ 0 が返るため、この経路で実際に停止するのはピアの CONNECTION_CLOSE 受信後の `NGTCP2_ERR_DRAINING` 等である。いずれも `?` で `run()` が Err 終了する
3. **アイドルタイムアウト** (server.rs:329 の `handle_expiry`): `NGTCP2_ERR_IDLE_CLOSE` が返ると `run()` が Err 終了する。デフォルトの `max_idle_timeout` は 30 秒 (`crates/ngtcp2-rs/src/config.rs` の `TransportParams::new()`) のため、ハンドシェイク後のクライアント切断・放置でサーバーが停止する

また、ハンドシェイク完了後のストリーム処理 (server.rs:186 の `h3_conn.read_stream` / server.rs:194 の `extend_max_stream_offset` / server.rs:181 の `bind_server_control_streams` / server.rs:215・218 の `submit_response`) も同じ `?` 伝播でサーバーを停止させる。ハンドシェイクを完了させた攻撃者が不正な HTTP/3 フレーム 1 個を送るだけでプロセスが終了する。

## 設計方針

- **パケット処理・ストリーム処理のエラーは接続単位で処理し、サーバーループは継続する**。`handle_recv` 内の `read_pkt` / `read_stream` / `extend_max_stream_offset` / `bind_server_control_streams` / `submit_response` のエラーは `run()` に伝播させず、接続単位で処理する。送信経路 (`flush_all` の `write_pkt` / `write_and_send_h3_streams`) のエラーも同様に接続単位で処理する
- **エラー種別を弁別する**:
  - `NGTCP2_ERR_DROP_CONN` / `NGTCP2_ERR_IDLE_CLOSE`: 接続を黙って破棄し、マップから除去する (closing / draining 状態にならないため `remove_closed_connections` では除去されない。明示的に除去しないと IDLE_CLOSE 後に `compute_timer_duration` が 1ms ビジーループになる)
  - `NGTCP2_ERR_DRAINING` / `NGTCP2_ERR_CLOSING`: 終了状態に移行した接続であり、`remove_closed_connections` の対象。破棄・継続する
  - `NGTCP2_ERR_RETRY`: アドレス検証の要求であり、本実装では Retry パケット送信を行わないため、`DROP_CONN` と同様に接続を黙って破棄・除去する (将来 Retry パケット送信を実装する場合は `ngtcp2_crypto_write_retry` を使う)。なお Retry はサーバーがトークン検証を設定していない場合 (本実装は tokenlen 未設定のため常に該当)、SERVER_INITIAL 状態で CRYPTO 未処理かつ再順序バッファにデータがある場合に `read_pkt` が返し得るため、DoS 経路として弁別対象に含める
  - それ以外の致命的エラー (`NGTCP2_ERR_CRYPTO` 等): `Connection::write_connection_close` (conn.rs に実装済み) で CONNECTION_CLOSE を送り、接続を closing 状態にして除去する。エラーコードは `ngtcp2_err_infer_quic_transport_error_code` 相当で導出する。`write_connection_close` 自体が `NGTCP2_ERR_NOBUF` を返した場合は接続を黙って破棄する
  - `Error::Nghttp3` (H3 層のエラー): `Connection::write_connection_close_app` (conn.rs に実装済み) でアプリケーションエラーコード (`nghttp3_err_infer_quic_app_error_code` 相当) を付けて CONNECTION_CLOSE を送る
- **同一 `SocketAddr` からの新規 Initial は、接続 ID ベースのルーティングで新規接続として処理する**。現行の `connections: HashMap<SocketAddr, ServerConnection>` は 1 アドレス 1 接続しか保持できず、2 接続目が確立できないため、キー構造の変更が必要 (RFC 9000 Section 5.1)
  - QUIC では 1 接続が複数の CID を持つ (クライアント初回 Initial の DCID、サーバー発行 SCID、`NEW_CONNECTION_ID` で発行した CID)。ルーティングは「到着パケットの DCID → 接続」のマップ (接続ごとの CID 集合を管理する第 2 段マップ) が必要で、単一の DCID キーではハンドシェイク後のパケットをルーティングできない
  - 既存接続の再送 Initial (同一 DCID) が新規接続として誤処理されないよう、DCID 照合で新規/既存を判定する
  - **キー構造の変更はサーバー API 全体に波及する**: `flush_all` / `compute_timer_duration` / `remove_closed_connections` / `Server::send_response` (公開 API の `client_addr` 引数) に加え、webtransport 側の `ServerWebTransportSession` の公開 API (`open_bidi_stream_for` / `send_stream_data_for` / `send_datagram_for` / `recv_datagram_for` / `open_uni_stream_for` / `get_established_addrs`) も SocketAddr キーに依存しているため、同じく変更が必要 (または API シグネチャの変更)
- `handle_timeouts` の `handle_expiry` エラーも接続単位で処理し、`run()` を終了させない
- クライアント側 (`Client::run` / `ClientWebTransportSession::run`) は単一接続のため、エラーを `?` で返す現行のままとする (サーバーとの非対称は設計判断)
- 公開 API (`Server::send_response` の `client_addr` 引数、webtransport の `open_bidi_stream_for` 等) の置き換え形 (ConnectionId 等) は実装時に決定し、破壊的変更となる場合は新メソッド追加で既存 API を維持する (0140 と同様の方針)

## 完了条件

- 同一アドレスから 2 接続目を張っても、1 接続目が維持されたまま 2 接続目が確立し、サーバーが継続する
- 不正パケット (DCID 不一致・破損 Initial) を送ってもサーバーが継続する
- クライアント切断 (アイドルタイムアウト含む) でサーバーが継続する
- ハンドシェイク完了後に不正な HTTP/3 フレームを送ってもサーバーが継続する
- テストが追加される: 同一 SocketAddr から 2 接続を張るテスト (1 つの UDP ソケットで複数 QUIC 接続を張るテスト用クライアントを実装し、DCID でデマルチプレクスする。テストクライアントは高レベル API の `Connection::client_new` (コールバック・TLS 設定を内部で行い、ソケットを所有しない) を外部の 1 ソケットでドライブして実装する。低レベル API (`client_new_raw`) は TLS 結線も全て呼び出し側の仕事になり別 issue 規模の作業になるため用いない。クライアントのローカルポート固定は同一ポートへの 2 ソケット bind が EADDRINUSE で失敗するため用いない)、不正パケットを投げるテスト (`std::net::UdpSocket` で実現可能)、アイドルタイムアウトのテスト (`with_max_idle_timeout` で短縮。デフォルト 30 秒のままだとテストが遅い)
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `crates/tokio-ngtcp2/src/server.rs` (`Server::run` / `handle_recv` / `handle_timeouts` / `flush_all` / `remove_closed_connections` / `Server::send_response` / `connections` のキー構造)
- `crates/tokio-ngtcp2/src/webtransport.rs` (`ServerWebTransportSession::run` / `recv_once` / `connections` のキー構造 / 公開 API 群)
- `crates/ngtcp2-rs/src/error.rs` (`Error::Ngtcp2` のエラーコード判別)
- `crates/ngtcp2-rs/src/conn.rs` (`write_connection_close` の使用)
- 一次資料: `refs/quic/rfc9000.txt` Section 5.1 (接続 ID によるルーティング)、Section 8.1 (Retry / アドレス検証)、Section 11.1 (不正 Initial の破棄・CONNECTION_CLOSE)
