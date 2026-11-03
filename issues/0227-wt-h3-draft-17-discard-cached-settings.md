# draft-17 では前回接続の WT SETTINGS を破棄して現在値だけを使う

- Created: 2026-11-03
- Completed: {YYYY-MM-DD}
- Branch: feature/change-wt-draft-17-discard-cached-settings
- Polished: 2026-11-03
- Category: change

## 目的

draft-ietf-webtrans-http3-17 Section 3.2 の変更に従い、再開接続で前回より小さい WT SETTINGS を受信しても、それだけを理由に接続を閉じないようにする。WebTransport CONNECT を 0-RTT で送れないことから、前回接続の WT 設定を利用する旧要件が廃止された。

## 現状

- `src/connection/mod.rs` の `Connection::previous_wt_settings` が、前回接続の WT SETTINGS を保持する
- `src/connection/wt_session.rs` の `Connection::set_previous_wt_settings` と `src/connection/client.rs` の同名公開 API で、前回値を注入できる
- `Connection::process_control_stream` は、現在の uni / bidi / data の初期上限が前回値より小さい場合に `H3_SETTINGS_ERROR` を返す。省略を `0` とみなすため、初期上限の省略も接続エラーになる
- `test_previous_wt_settings_decrease_uni_returns_settings_error` などの既存テストが、この draft-16 の動作を固定している

再現は、クライアントに前回値 `uni = 100` を注入し、今回 `uni = 50` または省略された SETTINGS を渡すこと。現在は `Error::ConnectionError(ErrorCode::SettingsError)` になる。

## 設計方針

- 0225 / 0226 で選択した draft-17 本文版では、Section 9.2 の WT SETTINGS 全体を前回接続から引き継がず、現在接続で受信した値と省略時のデフォルト値だけを使う
- 初期フロー制御値の比較だけを外すのではなく、WT 有効化の判定にも前回値を流用しないことを確認する
- draft-15 / 16 の互換経路では、前回値が注入されたクライアント接続の 3 項目の減少検出を維持し、その版が選択された接続だけに適用する。旧 API が注入されたことだけを理由に draft-17 を旧規則で検証しない
- `set_previous_wt_settings` の公開契約と保持状態を、旧版向けであることが分かる形に整理する。使用箇所のない新 API は追加しない
- QPACK など、RFC 9114 の通常の SETTINGS に対する 0-RTT 規則には変更を加えない
- QUIC の `reset_stream_at` transport parameter に関する再開時の記憶義務は別の規則なので、WT SETTINGS のキャッシュ廃止と混同しない
- 現在接続のサーバー SETTINGS を受信するまで、WT CONNECT を送れない制約を維持する

## 完了条件

- draft-17 では前回値より小さい初期上限、同値、大きい値、省略のいずれでも、前回との差だけを理由とした `H3_SETTINGS_ERROR` を返さない
- 現在の初期上限が送信許可と最初のフロー制御カプセルの比較基準になる
- クライアントが現在接続のサーバーから必要な WT 広告を受信できない場合、または必須設定 / transport parameter が不足する場合は、前回値があってもセッションを確立できない。サーバーは draft-17 クライアントの WT 広告省略を拒否理由にしない
- draft-15 / 16 が選択され、前回値が注入されたクライアント接続では、初期上限の減少や省略を従来どおり検出することを独立したテストで確認する
- 既存の減少検出テストを新旧の選択結果に応じて更新し、対象テストが通る

## 関連ファイル

- `src/connection/mod.rs`: `Connection::previous_wt_settings`, `Connection::process_control_stream`, `test_previous_wt_settings_*`
- `src/connection/wt_session.rs`: `Connection::set_previous_wt_settings`
- `src/connection/client.rs`: `ClientConnection::set_previous_wt_settings`
- `src/webtransport/session/mod.rs`: `Session::initialize_remote_limits`
- `tests/test_webtransport_flow_control.rs`

## 一次資料

- `refs/webtrans/draft-ietf-webtrans-http3-17.txt` Section 3.2 / 9.2
- `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 3.2: 廃止された比較規則
- `refs/quic/draft-ietf-quic-reliable-stream-reset-11.txt` Section 3: QUIC transport parameter の記憶義務
- <https://www.ietf.org/archive/id/draft-ietf-webtrans-http3-17.txt>
