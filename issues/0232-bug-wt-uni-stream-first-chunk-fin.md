# 最初のチャンクに FIN が付いた WebTransport 単方向ストリームでストリーム終了が配送されない

- Created: 2026-11-03
- Completed: {YYYY-MM-DD}
- Branch: {BRANCH}
- Polished: {YYYY-MM-DD}
- Category: bug

## 目的

WebTransport 単方向ストリームの最初のチャンクに FIN が付いている場合に、ストリーム終了イベントが配送されず、フロー制御クレジットの回復と受信量の追跡も行われない不具合を修正する。

## 現状

- `src/connection/mod.rs` の `handle_new_unidirectional_stream` は、ストリームタイプ確定後の FIN 処理で `wt_uni_streams` からストリームを削除して `WebTransportEvent::UniStreamEnd` を即時発火する
- この経路は `src/connection/wt_stream.rs` の `handle_wt_uni_stream_fin` を経由しないため、`WtSession::on_remote_stream_closed(false)` と `WtSession::remove_stream_received_data(stream_id)` が呼ばれない
- 同じ最初のチャンクに FIN が付いた場合でも、セッションが Pending のときは `WtSession` のバッファエントリに FIN が記録されない (`mark_buffered_stream_fin` が呼ばれない)
- セッションが Established のときに最初のチャンクへ FIN が付くと、終了イベントは発火するが、フロー制御クレジットが回復せず受信量の追跡も残る

## 再現手順

1. draft-17 の SETTINGS を交換し、transport parameter の検証を完了する
2. CONNECT を処理して Pending セッションを作る
3. クライアント開始単方向ストリーム (stream 6) に `0x54` + session ID + ペイロードを `fin = true` で流し込む
4. 2xx を送ってセッションを確立する
5. `SessionEstablished` / `UniStreamOpen` / `UniStreamData` は発火するが `UniStreamEnd` が発火しない

## 影響

- Pending セッションでは、確立後に配送されるイベント列が `UniStreamOpen` → `UniStreamData` の順になり、`UniStreamEnd` が欠落する
- Established セッションでも、最初のチャンクに FIN が付いた単方向ストリームは `WT_MAX_STREAMS` のクレジットが回復せず、`stream_received_data` の追跡も残る
- クライアントが受信するサーバー開始単方向ストリームでも同じ

## 修正方針

- 最初のチャンクに FIN が付いている場合も `handle_wt_uni_stream_fin` と同じ後処理 (セッションへの終了通知、クレジット回復、受信量追跡の削除、Pending 時のバッファへの FIN 記録) を行う
- セッションが Pending のときは終了イベントを即時発火せず、ストリームのバッファエントリに FIN を記録して確立後に配送する
- 双方向ストリームの同等経路 (`handle_wt_bidi_stream` の初回チャンク FIN) も同じ扱いになっているか確認する
- 既存の `handle_new_unidirectional_stream` の FIN 処理と `handle_wt_uni_stream_fin` の重複を整理する

## 完了条件

- Pending セッションで最初のチャンクに FIN が付いた単方向ストリームが、確立後に `UniStreamOpen` → `UniStreamData` → `UniStreamEnd` の順で配送される
- Established セッションで最初のチャンクに FIN が付いた単方向ストリームの後、`WT_MAX_STREAMS` のクレジットが回復し、同じセッションで新しいストリームを開ける
- クライアント側 (サーバー開始単方向ストリーム) でも同じ挙動になる
- FIN が別チャンクで到着する既存の経路に退行がない
- 公開接続 API 経由のテストで上記を検証する

## 関連ファイル

- `src/connection/mod.rs`: `handle_new_unidirectional_stream`
- `src/connection/wt_stream.rs`: `handle_wt_uni_stream_fin`, `resolve_wt_uni_stream_session_id`
- `src/connection/wt_session.rs`: `deliver_buffered_streams`
- `src/connection/wt_types.rs`: `WtSession::on_remote_stream_closed`, `remove_stream_received_data`, `mark_buffered_stream_fin`
- `tests/test_connection.rs`, `tests/test_webtransport_draft_connect.rs`

## 一次資料

- `refs/webtrans/draft-ietf-webtrans-http3-17.txt` Section 4.2, 4.6
- `refs/webtrans/draft-ietf-webtrans-http3-17.txt` Section 5.6
