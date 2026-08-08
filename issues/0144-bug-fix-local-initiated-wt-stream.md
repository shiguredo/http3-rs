# ローカル開始の WT ストリーム受信データがリクエストストリームとして誤処理される

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-local-initiated-wt-stream
- Polished: 2026-08-08

## 目的

アプリが自分で開いた WT ストリーム (クライアント開始 bidi 等) への受信データが HTTP/3 リクエストストリームとして誤解析される問題を修正する。

## 現状

- `src/connection/mod.rs` の `Connection.wt_uni_streams` / `wt_bidi_streams` は受信経路 (ピアが開いたストリーム) でのみ登録される
- ローカル側が開いた WT bidi ストリーム (クライアント開始 bidi のピアからのデータ、サーバー開始 bidi のピアからのデータ) を `Connection::feed_stream` に渡すと、`wt_bidi_streams` に無いため `Connection::handle_bidirectional_stream` でリクエストストリームとして処理され、WT ペイロードが HTTP/3 フレームとして誤解釈される。受信データにはヘッダー (0x41 + セッション ID) が含まれない (ストリームヘッダーは開始側が先頭で 1 回だけ送る。draft-16 Section 4.3 の「the rest of the stream is the application payload」)。ピア開始ストリームのみヘッダーが付くため、ストリーム ID ベースの登録と判定が必要になる
- ローカル開始ストリームを登録する API が存在しない (実装漏れ)
- WebTransport の基本利用 (アプリが開いたストリームへの応答データ受信) で発生し得る経路
- ローカル開始の WT uni ストリームへの受信データは仕様上存在しない (RFC 9000 Section 2.1 の「Unidirectional streams carry data in one direction: from the initiator of the stream to its peer」。単方向ストリームは開始者のみが送信する) ため、本 issue の対象は bidi のみとする

## 設計方針

- **ローカル開始 WT bidi ストリームの登録 API を追加する** (ストリーム ID とセッション ID の関連付け)。`Connection` の公開 API として追加し、`ClientConnection` / `ServerConnection` にもラッパーを追加する
  - 検証: ストリーム ID の下位 2 ビットが bidi を示すこと (RFC 9000 Section 2.1 Table 1: クライアント開始 bidi = 0x00 / サーバー開始 bidi = 0x01。uni の 0x02 / 0x03 は拒否する。下位 1 ビットが開始者を示す (RFC 9000 Section 2.1「Client-initiated streams have even-numbered stream IDs」) ため、クライアント側は 0x00 のみ、サーバー側は 0x01 のみを許可する)、セッション ID が `wt_sessions` に存在し Established 状態であること (Pending 状態のセッションへの登録は拒否する。Pending セッションでは `associated_streams` に追加しても受信データが `BufferedStreamRejected` になるか、確立時の flush でローカル開始ストリームを受信ストリームとして誤カウントするため)、同一ストリーム ID の二重登録をエラーにすること
  - 登録 API は `wt_bidi_streams` への insert と、`WtSession::associated_streams` への追加の両方を行う (セッション終了時の RESET 対象 (reliable size 計算を含む) に含めるため。`wt_stream_header_len` もローカル開始ストリームのヘッダー長を結果として正しく返すようになる)
  - 登録済みストリームは `feed_stream` の既存の `wt_bidi_streams.contains_key` 分岐で処理され、`BidiStreamOpen` イベントは発火しない (アプリは自分で開いたストリームを既知のため。`BidiStreamData` / `BidiStreamEnd` のみ発火する)
  - 競合への対処: 登録前に受信データが到着した場合は `handle_bidirectional_stream` が `streams` に `RequestStream` を作成し、WT ペイロードを HTTP/3 フレームとして誤解析して接続エラーになり得る (H3_FRAME_ERROR / QPACK エラー等)。`streams` のエントリ削除では誤解析の結果を巻き戻せないため、ストリームを開いたら即登録する運用とする。登録 API が既に `streams` に作成済みのエントリを検出した場合はエラーを返す (実装時に確定する)
- `feed_stream` で登録済みストリーム ID が来たら WT ストリームとして処理する (既存の `wt_bidi_streams.contains_key` 分岐 (mod.rs の `feed_stream`) がそのまま使える。実装本体は登録 API の追加とテスト)
  - 登録済みストリームの受信データはヘッダー無しのアプリペイロードであり、確定済み分岐 (ヘッダー解決をしない経路) を通す
- **ローカル開始ストリームの FIN では受信側フロー制御を更新しない**。既存の確定済み分岐の FIN 処理は `on_remote_stream_closed` を呼ぶが、これは「ピアが開いたストリーム」のクローズを通知するものであり (WT_MAX_STREAMS はピアが開くストリーム数の制限。draft-16 Section 5.3)、ローカル開始ストリームの FIN ではクレジットを返却してはならない。ローカル開始かどうかは `feed_stream` の分岐に到達する bidi ストリームの下位 2 ビット + ロールで判定する (クライアント側 0x00 / サーバー側 0x01 がローカル開始。`wt_bidi_streams` はローカル開始フラグを持たないため、追加状態は不要)。ただし `BidiStreamEnd` イベントの発火と `wt_bidi_streams` からの除去はローカル開始ストリームでも行う (ピアの送信方向終了の通知はアプリのストリームクローズ判断に必要)
- 0142 (WT_STREAM をリクエストストリームの先頭以外で受信しても H3_FRAME_ERROR にならない) は `handle_bidirectional_stream` の受信経路を変更する。本 issue の登録 API はこの経路を通るローカル開始ストリームを WT 処理へ振り分けるため、両者の実装は `feed_stream` の分岐で交錯する。実装順序に注意する
- ローカル開始 WT uni ストリームは本 issue のスコープ外とする (受信データが存在しないため。uni ストリームの RESET / FIN 伝播は 0170 で対応)

## 完了条件

- ローカル開始の WT bidi ストリーム (クライアント開始 bidi・サーバー開始 bidi) にピアからの受信データを feed すると WT ストリームとして処理される (リクエストストリームとして誤処理されない)
- 登録 API が `ClientConnection` / `ServerConnection` から公開 API として呼び出せる
- 登録 API の検証が機能する: uni ストリーム ID (下位 2 ビット 0x02 / 0x03)・非ローカル開始 ID・二重登録・セッション不存在・Pending セッションがエラーになる
- ローカル開始ストリームの FIN で WT_MAX_STREAMS クレジットが誤返却されず、`BidiStreamEnd` イベントは発火する
- テストが追加される: `src/connection/mod.rs` の `#[cfg(test)]` モジュールで、クライアント側 (クライアント開始 bidi) とサーバー側 (サーバー開始 bidi) それぞれについて「登録 → feed → イベント検証 (BidiStreamData 等)」、「FIN 時のフロー制御不変と BidiStreamEnd 発火」、「登録 API の検証エラー (uni ID / 非ローカル開始 ID / 二重登録 / セッション不存在 / Pending セッション)」を検証する
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/connection/mod.rs` (`Connection::feed_stream` / `Connection::handle_bidirectional_stream` / `wt_uni_streams` / `wt_bidi_streams` / 登録 API)
- `src/connection/client.rs` / `server.rs` (登録 API の公開ラッパー)
- `src/connection/wt_session.rs` (`WtSession::associated_streams` への追加 / セッション終了時の RESET 対象)
- `src/connection/wt_stream.rs` (ローカル開始ストリームの FIN 処理の分岐)
- 一次資料: `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 4.2 / 4.3 / 5.3、`refs/quic/rfc9000.txt` Section 2.1
