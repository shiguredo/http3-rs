# draft-17 の SETTINGS_WT_ENABLED コードポイントを旧版と区別して扱う

- Created: 2026-11-03
- Completed: {YYYY-MM-DD}
- Branch: feature/add-wt-draft-17-settings-codepoint
- Polished: {YYYY-MM-DD}
- Category: add

## 目的

draft-ietf-webtrans-http3-17 Section 9.2 の `SETTINGS_WT_ENABLED = 0x20b8933b` を送受信できるようにする。draft-15 / 16 の `0x2c7cf000` と別々に保持し、旧版との互換性と新旧両方の広告を維持する。

## 現状

- `src/settings.rs` の `ID_WT_ENABLED` は `0x2c7cf000` 固定で、`Setting::WtEnabled` はこの ID だけを扱う
- `Setting::from_wire` は `0x20b8933b` を `Setting::Unknown` に変換し、`webtransport::Settings::from_payload` はこれを捨てる
- `src/webtransport/settings.rs` の `Settings` は `wt_enabled` を 1 つだけ持ち、`detect_draft_pattern` は最大で `DraftVersion::Draft15` を返す
- `src/webtransport/connect/draft.rs` の `DraftVersion` と SETTINGS ビルダーにも draft-17 を区別する表現がない
- `Connection::process_control_stream` の `SETTINGS_WT_ENABLED > 1` 検証も旧 ID のフィールドだけを参照する

新 ID だけを広告するサーバーを、現在のクライアントは WebTransport 非対応として扱う。定数を単純に置き換えると draft-15 / 16 が使えなくなる。

## 設計方針

- 旧 ID と新 ID の両方を型付き SETTINGS として表現し、それぞれの値を独立して保持する
- `Setting::from_wire` / `as_wire`、`Settings::from_payload` / `iter` と SETTINGS ビルダーに配線する
- 新 ID に対応するバージョン表現を用意し、draft-15 / 16 と同じ `webtransport-h3` トークンを返す
- `declares_flow_control` を含むバージョン依存の判定が、新 ID を旧版扱いしないようにする。draft-17 の宣言条件に draft-14 の `WT_MAX_SESSIONS > 1` を持ち込まない
- サーバーから新 ID の値 `2` 以上を受信したクライアントは `H3_SETTINGS_ERROR` にする。クライアントから受信した値の扱いは Section 3.1 / 7.1 の役割差を守り、クライアント向け検証をサーバーにも一律適用しない
- 同じ SETTINGS ID の重複は禁止したまま、新旧の異なる ID の併送を許可する

接続上でどの版を選ぶか、クライアントから新 ID が送られない場合にどうするかは 0226 で扱う。本 issue はワイヤー表現と設定値の保持に絞る。

## 完了条件

- 新旧 ID を独立してエンコード / デコードでき、片方だけの場合と両方を含む場合を公開 API 経由で検証できる
- 新 ID の値 `0` / `1` / `2` のクライアント側受信結果と、未知 ID を無視する既存動作を検証する
- 新 ID の設定で初期フロー制御値を保持でき、draft-14 の宣言条件を誤適用しない
- draft-02 / 07 / 14 / 15 / 16 の既存設定とテストを維持する
- 公開 API を追加する場合は使用箇所とテストを用意し、対象の PBT と境界値テストが通る

## 関連ファイル

- `src/settings.rs`: `Setting`, `Setting::from_wire`, `Setting::as_wire`
- `src/webtransport/settings.rs`: `Settings`, `from_payload`, `iter`, `detect_draft_pattern`, `declares_flow_control`
- `src/webtransport/connect/draft.rs`: `DraftVersion`, `build_client_settings`, `build_server_settings`
- `src/connection/mod.rs`: `Connection::process_control_stream`
- `pbt/tests/prop_settings.rs`, `pbt/tests/prop_webtransport/settings.rs`, `tests/test_webtransport_draft_connect.rs`

## 一次資料

- `refs/webtrans/draft-ietf-webtrans-http3-17.txt` Section 3.1 / 5.1 / 7.1 / 9.2
- `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 9.2: 旧 ID
- <https://www.ietf.org/archive/id/draft-ietf-webtrans-http3-17.txt>
