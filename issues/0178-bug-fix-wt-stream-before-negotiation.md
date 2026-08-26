# WT ネゴシエーション完了前に到着した先頭 0x41 の bidi ストリームがリクエストストリームとして誤処理される

- Created: 2026-08-14
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-wt-stream-before-negotiation
- Polished: 2026-08-26

## 目的

WT ネゴシエーション未完了時に到着した正当な WT bidi ストリームがリクエストストリームとして誤処理され、接続が壊れる問題を修正する。

## 現状

- サーバー側の新規クライアント開始 bidi ストリームの振り分けは `src/connection/mod.rs` の `Connection::dispatch_client_bidi_stream` が行い、`is_wt_fully_negotiated()` が true のときのみ先頭 varint を捕捉して WT 経路 (`handle_wt_bidi_stream`) に回す
- ネゴシエーション未完了 (クライアントの SETTINGS 未受信) の間に到着した先頭 0x41 のストリームは `handle_bidirectional_stream` 経由で `RequestStream::process_raw` に落ちる
- QUIC はストリーム間の到着順を保証しないため、ネゴシエーション未完了時の bidi ストリーム到着は起こり得る
- `RequestStream::process_raw` の `Frame::Unknown` 分岐は 0x41 を未知フレームとして無視する (サーバー先頭位置のみ) が、WT_STREAM は length を持たない (draft-ietf-webtrans-http3-16 Section 4.3: "WT_STREAM lacks length and is not a proper HTTP/3 frame") ため、ワイヤ上の 2 番目の varint (session_id) が HTTP/3 フレームの length として解釈される
- session_id が 0 でなければ実ペイロードの先頭が length 分巻き込まれ、以降の解析がずれて接続エラー (H3_FRAME_ERROR 等) に至り得る

## 設計方針

- **未ネゴシエーション時 (`is_wt_fully_negotiated()` が false、主にクライアントの SETTINGS 未受信) に先頭 0x41 の bidi ストリームを受信した場合は、ストリームデータを保留 (バッファリング) し、ネゴシエーション完了後に WT 経路 (`handle_wt_bidi_stream`) に回す方式を採用する**
  - 根拠: draft-16 Section 4.6「WebTransport endpoints SHOULD buffer streams and datagrams until they can be associated with an established session」
  - 破棄は採らない。理由は次の 2 つ: 正当な WT bidi ストリームのデータを失うこと、および bidi ストリームは RFC 9114 上デフォルトでリクエストストリームであり、0143 の 0x54 (uni) で採用した RFC 9114 Section 6.2 の abort 方式 (unknown stream type の MUST が定める 2 択) が適用できないため (Section 6.2 の対象は unknown stream type のみ)
  - 0147 の CONNECT 保留 (`deferred_wt_connects` を SETTINGS 受信時に処理) と同様の方式・同タイミングで処理する
- 保留バッファは既存の `pending_bidi_dispatch` (先頭 varint が不完全な bidi ストリームの一時バッファ。`feed_stream` が後続データを再ルーティングする) とは別のフィールドとする。`pending_bidi_dispatch` は varint 確定までの一時バッファであり、本 issue の保留は 0x41 確定後のストリームデータ (データ + FIN フラグ) を保持する。`dispatch_client_bidi_stream` 内の遷移は「varint 未完 → `pending_bidi_dispatch` → varint 確定 (0x41) → 未ネゴシエーションなら保留バッファ」となる
- 0x41 を「length 前置の HTTP/3 フレーム」として解釈しないこと (WT_STREAM は length を持たず、2 番目の varint は session_id)
- 保留データは SETTINGS 受信時に再ディスパッチする。`is_wt_fully_negotiated()` が true なら WT 経路 (`handle_wt_bidi_stream`) へ回す。false のまま (ピアの SETTINGS が WT 非対応等、WT がネゴシエーションされないことが確定した場合) は保留したストリームを破棄する (0x41 を length 前置の HTTP/3 フレームとして解釈しないため。RFC 9114 Section 9 の未知要素の無視に相当し、リクエストストリームとして流し込まない)。再ディスパッチと破棄の判定は SETTINGS 受信時に限定する。`wt_transport_verified` は統合層が接続確立時に注入済みである前提 (sans-I/O の利用規約) のため、SETTINGS 受信時点で false のままなら以後 true にならない
- 保留には上限 (ストリーム数・データ量) を設け、超過時は `WT_BUFFERED_STREAM_REJECTED` エラーコードで RESET_STREAM / STOP_SENDING して破棄する (draft-16 Section 4.6「endpoints MUST limit the number of buffered streams and datagrams」「When the number of buffered streams is exceeded, a stream MUST be closed by sending a RESET_STREAM and/or STOP_SENDING with the WT_BUFFERED_STREAM_REJECTED error code」。既存の `WebTransportEvent::BufferedStreamRejected` の方式を利用する)
- **0142 との整合**: 0142 の「サーバー側先頭位置の 0x41 は接続エラーにしない (未ネゴシエーション時は無視)」という決定は維持する。本 issue は、0142 の実装が 0x41 を length 前置フレームとして解釈して消費する無視の実装を保留 (バッファリング) に置き換える。0142 は closed の記録としてそのまま残し、未ネゴシエーション時の扱いの変更は本 issue に記録する
- **0143 との整合**: 0143 は 0x54 (uni) について RFC 9114 Section 6.2 の abort 方式を採用しバッファリングしない。bidi (0x41) は unknown stream type ではないため同条文は適用されず、draft-16 Section 4.6 の SHOULD buffer が適用される。両者は矛盾しない

## 完了条件

- ネゴシエーション未完了時 (主にクライアントの SETTINGS 未受信) に先頭 0x41 の bidi ストリームが到着しても、接続エラーにならずボディが誤解析されない (ストリームデータが保留される)
- ネゴシエーション完了後に、保留されていた同ストリームが WT ストリームとして処理される (`BidiStreamOpen` / `BidiStreamData` イベントが発火する)
- ピアの SETTINGS が WT 非対応の場合は、保留されていたストリームが破棄され、接続は壊れない
- 保留の上限を超えたストリームは `WT_BUFFERED_STREAM_REJECTED` でリセットされ、接続は壊れない
- ネゴシエーション完了後に到着した同ストリームは従来どおり WT ストリームとして処理される
- テストが追加される (`src/connection/mod.rs` の `#[cfg(test)]` モジュールでネゴシエーション未完了時の到着順を再現する統合テスト)
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

- `src/connection/mod.rs` の `Connection::feed_stream` / `Connection::dispatch_client_bidi_stream` を変更し、サーバー側の新規クライアント開始 bidi ストリームを `is_wt_fully_negotiated()` の状態に関わらず先頭 varint で判定し、0x41 かつ未ネゴシエーションならストリームデータを保留バッファに格納する
- SETTINGS 受信時 (`Connection::process_control_stream` 内、`process_deferred_wt_connects` の後) に保留バッファを再ディスパッチし、`is_wt_fully_negotiated()` が true なら `handle_wt_bidi_stream` 経由で WT ストリームとして処理し、false のままなら保留したストリームを破棄する
- 保留には上限 (ストリーム数・データ量) を設け、超過時はストリームをリセットして破棄する (draft-16 Section 4.6)
- テスト: `src/connection/mod.rs` の `#[cfg(test)]` モジュールで、SETTINGS 受信前に先頭 0x41 の bidi ストリームを feed → 接続エラーにならず保留 → SETTINGS と CONNECT 受信後に WT イベント (`BidiStreamOpen` / `BidiStreamData`) が発火することを検証する。ピアの SETTINGS が WT 非対応の場合のストリーム破棄と、保留上限超過時の破棄も検証する
- 一次資料: `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 3.1 / 4.3 / 4.6、`refs/h3/rfc9114.txt` Section 9
