# draft-17 の対応状況と接続共有時の制約を文書化する

- Created: 2026-11-03
- Completed: {YYYY-MM-DD}
- Branch: feature/update-wt-draft-17-documentation
- Polished: {YYYY-MM-DD}
- Category: doc

## 目的

draft-ietf-webtrans-http3-17 と実装の対応関係を、利用者が判断できる形で README と WebTransport の説明へ反映する。対応予定を実装済みとして記載せず、旧版互換とトランスポート依存の制約も区別する。

## 現状

- `README.md` の対応一覧は draft-16 までで、新 ID `0x20b8933b`、本文版の選択規則、WT SETTINGS のキャッシュ破棄が未記載
- `docs/WT_HTTP3.md` の draft-16 の差分表は、0-RTT 再開時の初期値減少を `H3_SETTINGS_ERROR` にする旧規則を記載する
- `README.md` と `docs/WT_HTTP3.md` には、フロー制御を無効にした WT セッションが、同じ接続上の HTTP 応答や QPACK の進行を止め得るという Section 8 の補足がない
- `crates/tokio-ngtcp2/src/webtransport.rs` の `wt_settings` は初期フロー制御値を送らず、WT のフロー制御を無効にする。`crates/tokio-s2n-quic/src/webtransport/client.rs` / `server.rs` は RESET_STREAM_AT の未実装を説明したうえで能力検証に `true` を渡している
- `examples/wt_server/README.md` は s2n-quic の制約を記載する。sans-I/O の対応と、この example / 各アダプターで実際に保証できることは同一ではない

## 設計方針

- 0225 / 0226 / 0227 / 0228 / 0230 の実装結果と、0229 の pending を確認してから対応表を更新する
- `0x2c7cf000` の draft-15 / 16 と `0x20b8933b` の本文版を区別し、本文版だけのクライアントと旧 draft も広告するクライアントの違いを説明する。まだ公開されていない RFC 番号を仮定しない
- 旧版比較の表では歴史的な規則を残し、現在選択する版に適用する動作を明記する
- Section 8 のデッドロック条件を具体的に説明する。WT の読み取りを別 HTTP 応答の完了まで待ち、その HTTP 応答が同じ接続のフロー制御で進めない場合がある。WT フロー制御の有効化、または接続の分離が回避策になる
- Section 4.6 の先行ストリーム / データグラムは拒否 / 破棄され得ること、ストリーム拒否はセッションを終了させず、必要なら新ストリームで再送できることを説明する
- 部分配送付きリセットは、アプリケーション指定オフセットを含む Reliable Size と、下位 QUIC API の実際の対応状況を説明する
- 既存の 0168 は draft-16 時点の説明との整合、0182 は旧参照の整理であり、その作業を本 issue へ重複して取り込まない。旧 draft 固有の根拠まで機械的に draft-17 に置き換えない

## draft-16 から draft-17 への全差分の確認結果

表紙から著者情報までを比較し、改行 / 改ページによる差分を除いて本文を照合した。規範的な変更、既存実装で満たす明確化、編集上の変更を以下に記録する。

| 節 | 差分 | 実装との照合 / 対応先 |
|---|---|---|
| 表紙、目次、ページヘッダー、索引 | 公開日、有効期限、版番号、ページ位置の更新 | 取得した原文を保存。実装変更なし |
| 2.2 | サーバーの識別を authority / path の組から HTTPS URI の参照へ変更 | `ConnectRequest::validate` と接続の `https` 検証を確認。3.2 の必須疑似ヘッダーに変更なし |
| 3.1 | クライアントからの WT_ENABLED を通常は無視する規則と、draft 併用時の例外を明記 | 0226 |
| 3.1 | 対応する draft と本文版のコードポイントをクライアントが広告する規則を明確化 | 0225 / 0226 |
| 3.2 | 前回接続の WT SETTINGS を破棄し、減少 / 省略を再開時の接続エラーにしない | 0227。CONNECT の 0-RTT 禁止は継続 |
| 3.3 | 重複していた WT-Available-Protocols の導入文を削除 | 編集上の変更。ヘッダーフィールドの意味や解析規則に変更なし |
| 4.3 | WT_STREAM の許可位置を bidi 開始者の最初のバイト列と明記し、違反を H3_FRAME_UNEXPECTED に変更 | 0228。既存の `register_local_wt_stream` の逆方向にヘッダーがない契約も確認 |
| 4.4 | Reliable Size にアプリケーション指定オフセットを含める | 0229。下位 API の制約により pending |
| 4.4 | 変更せず配送するエラーを WT_APPLICATION_ERROR 範囲に限定し、範囲外の扱いに RESET_STREAM_AT を追加 | `ApplicationErrorCode::from_http3_code` が予約値 / 範囲外を `None` とし、`Error::from_http3_code` が未知値を保持することを確認。アダプターの配送保証は 0229 の完了時にも確認する |
| 4.4 | 中継対象に RESET_STREAM_AT を追加し、hop ごとに WT ヘッダー長を補正して同じペイロード範囲を保証 | 本リポジトリに WT 中継実装はない。端点のオフセット変換は 0229 |
| 4.6 | バッファ拒否がセッションに影響しないこと、両端で先行送信が拒否され得ること、新ストリームで再送できることを補足 | 拒否をセッション終了と分ける既存コードを確認。継続性を 0230 で検証し、本 issue で利用者向けに説明 |
| 5.6.1 | 中継が送るフロー制御カプセルも上限を増加させる必要があると補足 | WT 中継実装はないため、端点向けの新機能 issue は作成しない |
| 5.6.2 / 5.6.4 | 最初の WT_MAX_STREAMS / WT_MAX_DATA も SETTINGS 初期値より増加しなければならないと明記 | `Session::initialize_remote_limits` / `process_capsule`、`WtSendFlowControl::new` / `process_capsule`、既存の初期値境界テストで対応済み。重複起票しない |
| 7.1 | 異なるコードポイントごとの広告、本文版と draft の混在選択 / フォールバックを明記 | 0225 / 0226 |
| 8 | フロー制御無効時の HTTP / QPACK 停滞と、読み取り待ちによるデッドロックを補足 | 本 issue。フロー制御カプセル実装の修正は既存 0218 等へ委ねる |
| 9.2 | WT_ENABLED の登録 ID を 0x2c7cf000 から 0x20b8933b に変更 | 0225。ほかの初期値 SETTINGS の ID は不変 |
| 9.5 | WT_SESSION_GONE の登録記述に Reference、Change Controller、Contact を整備 | 登録書式の変更。エラーコードと説明の意味は不変 |
| 10 | OVERVIEW を 12 → 13、RESET-STREAM-AT を 09 → 11、HTTP/2 を 14 → 15 に更新し、RFC 3986 の直接参照を削除 | 原文を保持。ローカルの関連参照は最新版の overview-14 / reliable-stream-reset-11 / http2-16 に更新済み |

4.1 / 4.2 / 4.5 / 4.7 / 4.8、フロー制御の残りの本文、セッション終了、カプセルのコードポイント、アプリケーションプロトコルの解析規則には、上表以外の意味上の変更はない。

## 完了条件

- README と WebTransport の説明から、新旧 ID、バージョン選択、0-RTT、WT_STREAM のエラー、部分配送リセットの扱いを確認できる
- 未実装 / pending と実装済みを区別し、アダプターごとの制約を実際の能力と一致させる
- 接続共有時のデッドロック条件と回避策、先行ストリーム拒否後の扱いを説明する
- 上表の全差分に対応先、対応済みの根拠、または変更不要の理由が残る
- 旧版固有の説明を破壊せず、現在の挙動と混同させない
- `CODEBASE.md` に従い、CHANGES.md は変更しない

## 関連ファイル

- `README.md`: 対応 draft、設定 ID、フロー制御、参照規格
- `docs/WT_HTTP3.md`: バージョン差分、クライアントの必須設定、draft-16 の変更表
- `crates/tokio-ngtcp2/README.md`, `crates/tokio-s2n-quic/README.md`, `examples/wt_server/README.md`: 対応範囲と制約
- `src/webtransport/connect/request.rs`: `ConnectRequest::validate`
- `src/webtransport/session/mod.rs`: `initialize_remote_limits`, `process_capsule`
- `src/webtransport/error.rs`: `ApplicationErrorCode::from_http3_code`, `Error::from_http3_code`
- `crates/tokio-s2n-quic/src/webtransport/flow_control.rs`: `WtSendFlowControl`
- `tests/test_webtransport_flow_control.rs`: `test_peer_settings_initial_limits_are_monotonic_baseline`

## 一次資料

- `refs/webtrans/draft-ietf-webtrans-http3-16.txt`
- `refs/webtrans/draft-ietf-webtrans-http3-17.txt`
- <https://www.ietf.org/archive/id/draft-ietf-webtrans-http3-17.txt>
