# WT-Protocol / WT-Available-Protocols のネゴシエーションが接続層で検証されない

Created: 2026-04-05
Model: Opus 4.6

## 概要

クライアント側で WebTransport CONNECT の 2xx レスポンスを受信した際、`WT-Protocol` ヘッダーの
存在や値を検証せずにセッションを `Established` に遷移させている。

`webtransport/connect.rs` にネゴシエーション検証ロジック (`validate_wt_protocol_negotiation` 等)
は実装済みだが、接続層から呼び出されていない。

## 根拠

- `draft-ietf-webtrans-http3-15 Section 3.3`: クライアントが `WT-Available-Protocols` を送信した場合、サーバーの 2xx レスポンスに `WT-Protocol` が含まれていなければ、または送信したリスト外の値が返された場合は `WT_ALPN_ERROR` でセッションをクローズしなければならない
- 現状では `WT-Available-Protocols` を送信しても、レスポンスの `WT-Protocol` を一切検証しない

## 補足

`WT-Available-Protocols` / `WT-Protocol` は OPTIONAL な機能であり、使用しない場合は検証不要。
問題はクライアントが `WT-Available-Protocols` を送信した場合に限定される。

## 必要な変更

1. セッション確立時（L1573-1592）で、リクエストに `WT-Available-Protocols` が含まれていたかを確認する
2. 含まれていた場合、レスポンスの `WT-Protocol` を `connect.rs` の検証ロジックで検証する
3. 検証失敗時は `WT_ALPN_ERROR` でセッションをクローズする

## 優先度

P1 — プロトコルネゴシエーション機能を利用する場合の仕様違反。

## 解決方法

Completed: 2026-04-05

1. `WtSession` に `available_protocols: Vec<String>` フィールドを追加
2. クライアントの `send_request` で WebTransport CONNECT 送信時に `wt-available-protocols` ヘッダーから値をパースして保存
3. 2xx レスポンス受信時に `available_protocols` が空でない場合:
   - レスポンスの `wt-protocol` ヘッダーを `ConnectResponse::parse_protocol` でパース
   - 値が存在しないか `available_protocols` に含まれていなければ `terminate_wt_session` で閉鎖
   - 検証成功時のみ `Established` に遷移

## 参考

- `src/connection/mod.rs:1573-1592`: 2xx レスポンス受信時のセッション確立処理
- `src/webtransport/connect.rs:548`: 未使用の検証ロジック
