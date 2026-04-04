# WebTransport Pending セッション数が接続単位で無制限

Created: 2026-04-07
Completed: 2026-04-07
Model: Opus 4.6

## 概要

サーバー側で未知の `session_id` に対してストリームまたはデータグラムが先着した場合、`Connection::associate_or_buffer_stream` および `Connection::feed_datagram` は無条件に新規 `WtSession` を `wt_sessions` に挿入する。`WT_MAX_BUFFERED_STREAMS` / `WT_MAX_BUFFERED_DATAGRAMS` は `WtSession` 内部のバッファ上限であり、Pending セッションの個数自体は接続単位で制限されていない。

その結果、攻撃者は一意な client-initiated bidi `session_id` を大量に投げ込むだけで、Pending セッションを無限に増殖させることができる (DoS 経路)。nghttp3 は QUIC の `max_client_streams_bidi` を参照して拒否しており、本実装にはこの境界がない。

## 該当箇所

- `src/connection/mod.rs` `feed_datagram` (現在 L833 付近、`else` ブランチで `WtSession::new`)
- `src/connection/mod.rs` `associate_or_buffer_stream` (現在 L1963 付近、`else` ブランチで `WtSession::new`)

## 根拠

- draft-ietf-webtrans-http3-15 Section 4.6: 先着ストリーム/データグラムのバッファリングは MAY であり、上限超過時の挙動は実装依存
- RFC 9297 Section 2.1: 作成不能な Quarter Stream ID は `H3_ID_ERROR` で接続を閉じてよい
- nghttp3 `lib/nghttp3_conn.c` L3649 付近: クライアント開始 bidi 上限を見て拒否

## 修正方針

1. `Connection` に接続単位の Pending セッション数上限定数 `WT_MAX_PENDING_SESSIONS` (例: 16) を導入する。
2. `feed_datagram` / `associate_or_buffer_stream` の「未知 session_id で新規 Pending を作成する」パスで現在の Pending セッション数を集計し、上限を超える場合は:
   - データグラム: 単純破棄
   - ストリーム: `BufferOverflow` 相当の戻り値で呼び出し側に拒否させる (`WT_BUFFERED_STREAM_REJECTED` で RESET)
3. 将来的に QUIC 層から `max_client_streams_bidi` を注入できる API を別 issue として検討する (本 issue のスコープ外)。
4. 単体テストで以下を追加:
   - `WT_MAX_PENDING_SESSIONS + 1` 個の異なる `session_id` でストリーム到着 → 最後の 1 個が拒否される
   - データグラム版でも同様の挙動になる

## 解決方法

- `src/connection/mod.rs` に定数 `WT_MAX_PENDING_SESSIONS = 16` を追加した。
- 補助関数 `Connection::count_pending_wt_sessions()` を追加し、`wt_sessions` の中で `Pending` 状態のものだけを数えるようにした。
- `Connection::feed_datagram()` の「未知 session_id で新規 Pending を作成する」パスで上限を超過する場合はデータグラムを破棄するようにした。
- `Connection::associate_or_buffer_stream()` でも同じ判定を入れ、上限超過時は `AssocOutcome::BufferOverflow` を返すことで呼び出し側に `WT_BUFFERED_STREAM_REJECTED` で RESET させるようにした。
- 単体テストで以下を追加:
  - `test_pending_wt_sessions_limit_for_streams`
  - `test_pending_wt_sessions_limit_for_datagrams`

## 残課題

- QUIC 層から `max_client_streams_bidi` を注入できる API の整備は別 issue で扱う (本 issue は接続単位の固定上限のみ)。
