# サーバー送信 GOAWAY 後の WebTransport セッションを先行ストリーム/datagram で受け入れてしまう

Created: 2026-04-06
Completed: 2026-04-07
Model: Opus 4.6

## 解決方法

`associate_or_buffer_stream()` および `feed_datagram()` の新規セッション生成パスで `last_sent_goaway_id` を参照し、`session_id >= last_sent_goaway_id` の場合は新規 Pending セッション生成を拒否するように変更した。stream 側は `Err(())` を返して `WT_SESSION_GONE` 相当でストリームをリセット、datagram 側は静かに破棄する。これにより graceful shutdown 中の境界以降の session_id に対する先行 stream / datagram で新規セッションが作られ続ける問題を防ぐ (nghttp3 と整合)。

## 優先度

P1

## 概要

サーバーが自身で送信した `GOAWAY` (`last_sent_goaway_id`) より大きい `session_id` に紐づく先行 WT stream / datagram を、`associate_or_buffer_stream()` と `feed_datagram()` がそのまま新規 Pending セッションとして受け入れてしまう。

draft-ietf-webtrans-http3-15 Section 4.7 では、`GOAWAY` 送信後にクライアントは新規 WebTransport セッションを開始できない。したがって、その将来 `session_id` に属する先行 stream / datagram も拒否しなければならない。

## 根拠

- draft-ietf-webtrans-http3-15 Section 4.7
- nghttp3 `lib/nghttp3_conn.c` L3654 付近: `tx.goaway_id <= session_id` を明示的に拒否
- `src/connection/mod.rs` L713-722 (`feed_datagram` else 分岐)
- `src/connection/mod.rs` L1764-1774 (`associate_or_buffer_stream` else 分岐)
- `src/connection/mod.rs` L395-396 (`last_sent_goaway_id` フィールド)

既存 issue 0036 では peer から受信した `GOAWAY` (`peer_goaway_received`) に基づく拒否を追加したが、サーバー自身が送った `GOAWAY` の境界 (`last_sent_goaway_id`) に対する拒否は実装されていない。コメント「サーバーが受信する GOAWAY は push ID を運ぶものであり、WebTransport セッションの新規拒否判定には使えない」は事実として正しいが、論点が逆向き (送信側 GOAWAY) のケースを見落としている。

## 影響

サーバーが graceful shutdown のために `GOAWAY` を送った後でも、その境界以降の `session_id` を含む先行 stream / datagram によって新規 Pending セッションが生成され続け、shutdown が完結しない。Sans I/O 状態機械としても破綻している。

## 対応方針

1. `associate_or_buffer_stream()` の else 分岐 (新規セッション生成パス) で `self.last_sent_goaway_id` を参照し、`session_id >= last_sent_goaway_id` の場合は `Err(())` を返して `WT_SESSION_GONE` 相当で拒否する
2. `feed_datagram()` の else 分岐でも同様に拒否し、datagram は静かに破棄する
3. 既存の peer GOAWAY 拒否 (issue 0036 で追加されたはずのもの) との整合性を取り、両方向 (rx / tx) の GOAWAY を一貫してチェックする
4. 既存コメントを差し替え、tx 側 GOAWAY が拒否根拠であることを明記する

## 参照

- draft-ietf-webtrans-http3-15 Section 4.7
- RFC 9114 Section 5.2
- nghttp3 `lib/nghttp3_conn.c` L3654
- `src/connection/mod.rs` L395, L713, L1764, L2987-3021
