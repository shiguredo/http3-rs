# GOAWAY の方向別セマンティクスを正しく扱う

Created: 2026-04-06
Completed: 2026-04-06
Model: Opus 4.6

## 優先度

P1

## 概要

`connection/mod.rs` の GOAWAY 受信処理がロール非依存になっており、サーバーがクライアントから受信した `GOAWAY` でも WebTransport セッションの draining 伝播と新規 WT 拒否が発火する。HTTP/3 においてクライアントから送信される `GOAWAY` は push ID を運ぶもので、WT セッション ID (CONNECT 要求 stream ID) と比較する意味はない。結果として、サーバー側で client `GOAWAY` 後の先行到着 WT stream / datagram を不当に拒否する。

## 根拠

- RFC 9114 Section 5.2 / 7.2.6: クライアントが送る `GOAWAY` は push ID、サーバーが送る `GOAWAY` は request stream ID
- draft-ietf-webtrans-http3-15 Section 4.7: 「新規 WebTransport セッションを開始できなくなる」のは `GOAWAY` を受けたクライアント側
- `connection/mod.rs:2054` では既に `role == Client` 受信時のみ stream ID バリデーション (`% 4 == 0`) をしており、方向非対称を認識しているにもかかわらず、2068 行以降の `goaway_received` セットと draining 伝播、および `connection/mod.rs:688` / `connection/mod.rs:1718` の新規 WT 拒否がロール非依存で走っている

## 修正方針

現状の `goaway_received: bool` を以下の 2 フィールドに分離する:

- `peer_goaway_received: bool` — 単に GOAWAY を受信した事実（イベント発火・複数受信時の単調減少チェック用、両ロール共通）
- `peer_goaway_request_boundary: Option<u64>` — クライアント受信時のみ設定。WT 新規拒否 / draining 伝播の判定に使う

GOAWAY 受信処理:

- `role == Client`: 現行の stream ID バリデーション、`peer_goaway_request_boundary` 設定、既存 WT セッションへの draining 伝播を維持
- `role == Server`: push ID として受信事実のみ記録。WT draining 伝播も新規 WT 拒否もしない（push 未実装なら実質 no-op + `Event::GoawayReceived` 発火のみ）

`connection/mod.rs:688` と `connection/mod.rs:1718` の「WT 新規拒否」判定は `peer_goaway_request_boundary` ベースに変更し、クライアント時のみ効く。

## 影響

- 破壊的変更: `goaway_received` を参照している公開 API があれば変更
- テスト: サーバー受信 `GOAWAY` 後に WT stream / datagram が拒否されないこと、クライアント受信時は従来通り draining されることを検証

## 解決方法

`src/connection/mod.rs` で GOAWAY 受信処理をロール別に分岐した。

- `goaway_received: bool` → `peer_goaway_received: bool` に改名 (方向を問わない受信事実のみ)
- `goaway_id: Option<u64>` → `peer_goaway_last_id: Option<u64>` に改名
  (単調減少チェックは両ロールで実施するが、値の意味はロール依存)
- `peer_goaway_request_boundary(&self) -> Option<u64>` ヘルパーを追加
  (クライアント受信時のみ `peer_goaway_last_id` を返し、サーバー受信時は `None`)
- GOAWAY 受信ハンドラで WT draining 伝播を `role == Client` 時のみ実施するよう変更
- `queue_request_stream` の新規 request stream 境界チェックを
  `peer_goaway_request_boundary()` 経由に変更
- データグラム受信 (`feed_datagram`) と WT bidi stream 受信の新規 Pending セッション作成時に
  行っていた `goaway_received` による拒否ロジックを削除
  (サーバー受信の push ID は WT セッション ID とは比較する意味がないため)

既存テスト 418 件は全て通過。CHANGES.md に `[FIX]` エントリを追加した。
