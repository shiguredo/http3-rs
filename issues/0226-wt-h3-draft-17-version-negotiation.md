# draft-17 本文版と旧 draft の選択規則を実装する

- Created: 2026-11-03
- Completed: {YYYY-MM-DD}
- Branch: feature/change-wt-draft-17-version-negotiation
- Polished: 2026-11-03
- Category: change

## 目的

draft-ietf-webtrans-http3-17 Section 3.1 / 7.1 に従い、クライアントが `SETTINGS_WT_ENABLED` を送らない場合にも本文で定義された版を選べるようにする。旧 draft と本文版の両対応時も、広告の共通部分だけを求める従来の規則では処理しない。

## 現状

- `src/connection/wt_session.rs` の `mutually_advertised_wt_drafts` / `negotiated_wt_draft_version` は、ローカルとピアが共に広告した旧 draft の共通部分から選ぶ
- `is_wt_fully_negotiated` / `validate_wt_connect_request_server` は、ローカルが `Draft15` の場合にクライアントの WT 広告を要求する
- クライアント側の `peer_wt_draft_version` / `validate_wt_connect_request` は、ピア側の広告を単独で判定している
- `ConnectRequest::from_headers` は `:protocol = webtransport-h3` を `Draft15` と推定する。draft-17 も同じトークンを使うため、ヘッダーだけでは版を特定できない
- `crates/tokio-ngtcp2/src/webtransport.rs` の `ClientWebTransportSession::open_session` も、ピアの広告からリクエストの版を選んでいる

## 設計方針

0225 の新旧コードポイントの分離を前提として、接続単位の選択結果を CONNECT、先行ストリームの振り分け、能力検証で共通して使う。

- 本文版だけに対応するサーバーでは、クライアントからの `SETTINGS_WT_ENABLED` を必須にせず、Section 3.1 に従って無視する
- 旧 draft と本文版の両方に対応するサーバーでは、クライアントが Section 9.2 の新 ID を送れば本文版、そうでなければ対応する旧 draft の最上位、それもなければ本文版を選ぶ
- 旧 draft だけに対応する端点との通信では、従来の共通バージョン選択を維持する
- 両方に対応する端点は、対応する異なるコードポイントをそれぞれ 1 回広告する。draft-15 / 16 のように同一 ID を使う版の数だけ同じ ID を重複送信しない
- クライアントでは、ローカルで対応していない旧版をサーバーの広告だけから選ばない
- `webtransport-h3` のヘッダー解析結果を、接続のネゴシエーション結果と区別する
- 必須の `H3_DATAGRAM`、`ENABLE_CONNECT_PROTOCOL`、QUIC transport parameter と `https` の検証を維持する
- 旧 draft を広告するサーバーは、クライアント SETTINGS 受信前に CONNECT / WT ストリームを処理しない。未受信と、受信済みだが WT の広告がない場合を区別する

0219 は `:protocol = webtransport` を使う draft-02 クライアントの互換受理が目的であり、本 issue の本文版フォールバックとは別に扱う。広告のない旧トークンを新ルールだけを理由に受理しない。

## 完了条件

以下を、実際に生成した SETTINGS / CONNECT のバイト列を接続へ渡すテストで確認する。

| サーバーの対応 | クライアントの広告 | 選択結果 |
|---|---|---|
| 本文版のみ | WT 広告なし | 本文版 |
| 本文版と旧 draft | 新 ID と対応する旧 ID | 本文版 |
| 本文版と旧 draft | 対応する旧 ID のみ | 共通する旧 draft の最上位 |
| 本文版と旧 draft | WT 広告なし、または対応する旧 ID なし | 本文版 |
| 旧 draft のみ | 対応する旧 ID | 従来の共通版 |

- 対応するサーバー広告がないクライアントは CONNECT を開始しない
- SETTINGS 未着、SETTINGS に WT 広告なし、必須設定 / transport parameter 不足を混同しない
- 新旧両対応で既存の draft-02 / 07 / 14 / 15 / 16 の接続を維持する
- 公開 API を用いたバージョン選択と先行ストリームのテストが通る

## 関連ファイル

- `src/connection/wt_session.rs`: `peer_wt_draft_version`, `mutually_advertised_wt_drafts`, `negotiated_wt_draft_version`, `is_wt_fully_negotiated`, `validate_wt_connect_request`, `validate_wt_connect_request_server`
- `src/connection/mod.rs`: `process_deferred_wt_connects`, `process_pending_wt_bidi_pre_negotiation` の呼び出し経路
- `src/webtransport/connect/request.rs`: `ConnectRequest::from_headers`, `draft_version`
- `crates/tokio-ngtcp2/src/webtransport.rs`: `ClientWebTransportSession::open_session`, `wt_settings`
- `tests/test_webtransport_draft_connect.rs`, `pbt/tests/prop_webtransport/connect.rs`

## 一次資料

- `refs/webtrans/draft-ietf-webtrans-http3-17.txt` Section 3.1 / 7.1
- <https://www.ietf.org/archive/id/draft-ietf-webtrans-http3-17.txt>
