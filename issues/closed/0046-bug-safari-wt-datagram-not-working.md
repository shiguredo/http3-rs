# Safari 26.4 WebTransport Datagram が動作しない

Created: 2026-04-06
Completed: 2026-04-06
Model: Opus 4.6

## 概要

Issue #0045 の SETTINGS 互換対応後、Safari 26.4 から WebTransport セッションは確立できるようになったが、Datagram の送受信がまだ動作しない。

## 再現手順

1. #0045 の修正込みのサーバーを起動する。
2. Safari 26.4 から `new WebTransport(...)` で接続しセッションを確立する。
3. `writer.write(...)` で Datagram を送信する / サーバーから Datagram を送る。
4. いずれも相手側に届かない。

## 観察

未調査。想定される原因候補:

- H3_DATAGRAM / WT session_id の quarter stream ID マッピング周りの draft 差
- Safari が Datagram のコンテキスト ID を draft-07 流儀で扱っている可能性
- フロー制御カプセル (WT_MAX_DATA 等) の送出順序の影響

## 根拠資料

- draft-ietf-webtrans-http3-07 Section 4 (Datagrams)
- draft-ietf-webtrans-http3-14 Section 6 (Datagrams)
- RFC 9297 (HTTP Datagrams)

## 解決方法

http3-rs サーバー側の不具合ではなく、クライアント (ブラウザ) 側 WebTransport API の仕様差異が原因だった。

Safari 26.4 は W3C WebTransport API の最新仕様に追従しており、Datagram 送信用の writable 取得方法が旧仕様と異なる:

- 最新仕様 (Safari 26.4): `wt.datagrams.createWritable()` メソッドを呼び出して WritableStream を取得する
- 旧仕様 (Chrome 等): `wt.datagrams.writable` プロパティから取得する

従来の `writable` プロパティしか参照しない実装では Safari で writable が取れず、Datagram が送信できない状態になっていた。moqt-js の `devtools/src/webtransport-devtools/signals.ts` の `sendDatagram()` にて、`createWritable` メソッドの有無で分岐する実装を確認し、Safari でも Datagram 送受信が正常に動作することを確認した。

http3-rs 側 (サーバー) は RFC 9297 / draft-ietf-webtrans-http3 に準拠しており修正不要。
