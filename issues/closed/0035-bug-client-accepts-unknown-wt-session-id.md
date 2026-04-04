# クライアントが未知の session_id に対して Pending セッションを作成してしまう

Created: 2026-04-06
Model: Opus 4.6

## 優先度

P1

## 概要

クライアント側で、自身が開始していない session_id の WebTransport ストリーム / datagram を受信した際に、`WT_SESSION_GONE` で拒否せず新規 Pending セッションを作成してバッファリングしてしまう。

## 根拠

draft-ietf-webtrans-http3-15 Section 4.6 では、クライアントが先行受信できるのは「自分が開始済みの CONNECT に紐づく」ケースに限定される。nghttp3 もクライアント側で session stream が見つからなければ `NGHTTP3_ERR_WT_SESSION_GONE` を返している。

現在の実装では `associate_or_buffer_stream()` (`src/connection/mod.rs` L1437 付近) と `feed_datagram()` (`src/connection/mod.rs` L504 付近) で、`wt_sessions` に存在しない session_id に対して無条件に Pending セッションを作成している。session_id の形式チェック (`session_id & 0x03 != 0x00`) はあるが、クライアントが実際にその CONNECT ストリームを開始したかの検証がない。

## 影響

サーバーが任意の client-initiated ID 形式の session_id を使った不正な WT traffic を送信した場合、クライアントがそれを正当な未確立セッションとして抱え込む。メモリ消費の増大やセキュリティ上の問題につながる。

## 再現手順

1. クライアントが WebTransport セッションを 1 つ開始する (session_id = 0)
2. サーバーが session_id = 4 (クライアントが開始していない) の WT uni stream を送信する
3. クライアント側で session_id = 4 の Pending セッションが作成されバッファリングされる

## 対応方針

1. クライアント側 (`self.role == Role::Client`) で未知の session_id を受信した場合、`wt_sessions` に既存エントリがなければ `WT_SESSION_GONE` で拒否する
2. サーバー側は draft Section 4.6 に従い、先行到着のバッファリングを維持する
3. `associate_or_buffer_stream()` と `feed_datagram()` の両方に role ベースの制限を追加する

## 解決方法

Completed: 2026-04-06

`associate_or_buffer_stream()` と `feed_datagram()` の `else` 分岐（セッション未登録時）に `self.role == Role::Client` チェックを追加した。クライアントは自身が開始していない session_id に対して Pending セッションを作成せず、`WT_SESSION_GONE` で拒否する。サーバー側のバッファリング動作は維持。

既存テストもクライアント側テストにセッション確立済み状態を事前登録するよう修正し、クライアントが未知 session_id を拒否するテストを追加した。

## 参照

- draft-ietf-webtrans-http3-15 Section 4.6
- nghttp3 `nghttp3_conn.c` L3641 付近
- `src/connection/mod.rs` L504, L1437
