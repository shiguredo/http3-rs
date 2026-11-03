# draft-17 本文版と旧 draft の選択規則を実装する

- Created: 2026-11-03
- Completed: 2026-11-03
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

## 解決方法

- `src/webtransport/settings.rs` に `for_draft_versions` (指定した版の SETTINGS コードポイントを過不足なく広告する構築関数)、`advertised_draft_versions` (広告している版を優先順位順に返す関数)、`first_mutual_draft` (ピアと共通する最上位の版を確保なしで返す関数) を追加した。優先順位は draft-17 > draft-15 / 16 > draft-07 > draft-14 > draft-02 で、draft-07 と draft-14 の併送は Safari 互換のため draft-07 を優先する
- `src/connection/wt_session.rs` の `Connection::negotiated_wt_draft_version` を接続単位の版選択として実装した。サーバーは「クライアントが本文版のコードポイントを広告 → 本文版 / 対応する旧 ID のみ → 交差する旧 draft の最上位 / 広告なし・対応しない旧 ID のみ → 本文版対応なら本文版」、クライアントは「交差の最上位 / 版を広告しない端点はピアが本文版を広告していれば本文版」を選ぶ。SETTINGS 未着では `None` を返す
- `is_wt_fully_negotiated`、`validate_wt_connect_request`、`validate_wt_connect_request_server`、0x41 / 0x54 / datagram の受理判定、SETTINGS 受信後の保留再ディスパッチを選択結果で判断するようにした。SETTINGS 未着 (保留) と、受信済みで WT 広告がない場合 (選択結果に応じて受理 / 拒否) を区別する
- `src/webtransport/connect/mod.rs` に `ProtocolToken` を追加し、`:protocol` のヘッダー解析結果と接続の版を区別した。`ConnectRequest` の `draft_version` フィールドを `protocol_token` に置き換え (後方互換のない変更)、`protocol_value` の対応表を `ProtocolToken` に集約した
- `crates/tokio-ngtcp2` は新旧コードポイントと旧 draft を広告し、`open_session` が選択結果の版で CONNECT を組み立てるようにした。`examples/wt_server` は接続ごとに SETTINGS を構築し、Safari (draft-07 と draft-14 を併送) には draft-07 のコードポイントだけを、それ以外には対応する全コードポイントと初期フロー制御値を広告する
- `tests/test_webtransport_draft_connect.rs` に `version_selection` モジュールを追加し、完了条件の表 5 行、クライアントの開始条件、SETTINGS 未着と広告なしの区別、必須 SETTINGS / transport parameter 不足、先行ストリーム (0x41 / 0x54 / datagram) の保留・受理・拒否を、実際に生成した SETTINGS / CONNECT のバイト列で検証する
- `pbt/tests/prop_webtransport/settings.rs` に、SETTINGS のエントリ順序と分割位置を変えても選択結果が変わらない性質、広告集合が要求した版の優先順位どおりになる性質を追加した
- `examples/wt_server` の Safari 互換応答はブラウザ (Chromium / WebKit) との相互運用テストで確認した
