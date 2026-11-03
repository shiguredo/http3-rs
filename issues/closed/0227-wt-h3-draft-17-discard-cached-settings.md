# draft-17 では前回接続の WT SETTINGS を破棄して現在値だけを使う

- Created: 2026-11-03
- Completed: 2026-11-03
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

## 解決方法

- `src/connection/wt_session.rs` に `Connection::select_wt_draft_version` を追加し、`negotiated_wt_draft_version` と SETTINGS 受信処理が同じ選択規則を使うようにした。受信処理では `peer_settings` を更新する前に受信済み SETTINGS を渡して版を判定する
- `src/connection/mod.rs` の 0-RTT 再開時の減少検出を、選択した版が draft-14 / 15 / 16 の場合に行うようにした。draft-14 では最大 WebTransport セッション数の減少も検出する。draft-02 は 0-RTT に対応せず、draft-07 の 0-RTT 再開 (最大セッション数の減少検出) は未実装のため対象外、draft-17 は前回接続の WT SETTINGS を引き継がないため対象外とした。版を決定できない場合は、ローカルとピアのどちらも draft-17 に対応しない端点に限り従来どおり検出し、どちらかが draft-17 に対応する端点 (コードポイントを広告しない本文版だけの端点を含む) は前回値を破棄する
- `set_previous_wt_settings` の rustdoc を、draft-14 / 15 / 16 の 0-RTT 再開用であり draft-17 では使われないこと、初期フロー制御値のいずれかが前回値より小さければ接続を閉じることを明記した。`ClientConnection` 側は `Connection` の rustdoc を参照する形にした
- `Session::initialize_remote_limits` の rustdoc に、0-RTT 再開時も前回接続のキャッシュ値ではなく現在接続で受信した SETTINGS の値を渡すこと、前回値は減少検出にだけ使うことを追記した
- テストを追加した: 単体テストで draft-14 の初期上限と最大セッション数の減少検出、旧コードポイントのみの端点での省略検出、本文版だけのクライアントでの非検出、draft-02 / 07 を選択した場合の非検出、ピアが draft-17 だけを広告する場合の非検出、旧コードポイントの前回値を持つ draft-17 接続での非検出、版選択の各組合せ。統合テストで draft-17 の減少受理 (セッション確立まで) と、前回値が WT 有効化に流用されないこと (公開 API の CONNECT 開始で確認)。フロー制御テストで現在の SETTINGS が Session の比較基準になることを Connection から確認する。PBT で前回版と現在版を独立に生成し、旧コードポイントの版だけが減少検出の対象になることを検証する
- サーバーが draft-17 クライアントの SETTINGS_WT_ENABLED 省略を拒否しないことは、既存の `server_selects_document_version_without_client_advertisement` で確認している (サーバー側の処理は本変更で変わらない)
