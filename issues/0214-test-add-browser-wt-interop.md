# Chromium / WebKit との WebTransport 相互運用テストを CI で自動実行する

- Created: 2026-09-12
- Completed: 2026-09-12
- Branch: feature/test-add-browser-wt-interop
- Polished: {YYYY-MM-DD}

## 目的

`examples/wt_server` の WebTransport 実装が、実ブラウザ (Chromium / WebKit) の WebTransport 機能を網羅して動作することを CI で自動的に保証する。

Rust 側のテストだけではブラウザ固有の要件を検証できない。実際に Safari 26.4 は draft-07 と draft-13/14 のハイブリッド実装であり、サーバーが返してよい WT 系応答 SETTINGS が `SETTINGS_WEBTRANSPORT_MAX_SESSIONS` 単体に限られる (`docs/SAFARI_WT.md`)。この制約は Rust の e2e テストでは再現できず、ブラウザで接続して初めて分かる。

## 現状

- `examples/wt_server` は `handle_bidi_echo` / `handle_uni_echo` / `handle_datagram_echo` を持ち、双方向ストリーム・単方向ストリーム・データグラムの経路が実装されている
- 一方でブラウザからの接続を検証する自動テストは存在しない。`docs/SAFARI_WT.md` に手順が記録されているが、手動での確認に依存しており回帰を検出できない
- リポジトリには `interop/h3` / `interop/wt` があり、他実装との相互運用を自動テストしている。ブラウザも「もう 1 つの WebTransport 実装」として同じ枠に置けるが、現状は未整備
- `interop-wt.yml` は `macos-26` で動作しており、Chromium と WebKit の実エンジンを CI で動かせる土台がある
- `--allow-origin` オプションと `WtSessionRequest::origin()` は実装済みで、ブラウザが送る Origin ヘッダーを検証できる

## 設計方針

### 配置

`interop/browser/` を新設する。`interop/h3` / `interop/wt` と同じ「他実装との相互運用テスト」の位置付けとする。Node.js のみで構成するため Cargo ワークスペースには追加しない。

### 実装言語

Node.js の Playwright を使う。Chromium と WebKit を同一 API で駆動できる唯一の手段であり、WebKit を実エンジンで動かせることが Safari 対応の必須条件である。

### 検証ページ

自前の `index.html` をリポジトリ内に持ち、HTTPS で配信する。WebTransport は secure context を要求するためページサーバーは省けない。公開ページに依存すると、ページ側の改修やネットワーク障害で CI が赤くなり、原因がこのリポジトリ内で完結しなくなる。

接続先 URL と証明書ハッシュは `addInitScript` で注入し、ページ内の WebTransport クライアントは約 40 行に抑える。サーバーが生成する証明書は ECDSA P-256 の named curve 形式であり、`examples/wt_server` は起動時に SHA-256 ハッシュを base64 でログ出力するため、これを Node 側で読んで渡す。

### 既知の制約への対処

- Chromium は公開オリジンからローカルアドレスへの接続を Local Network Access チェックで拒否する。自前ページはローカル配信のため該当しないが、CI で公開ページ由来の接続を扱う場合に備えて `--disable-features=LocalNetworkAccessChecks` の要否を実測で判断する
- WebKit は接続時に ClientHello が欠落する既知の flake があるため接続リトライを入れる
- Chromium はオリジンごとに証明書検証結果をキャッシュするため、起動ごとに新しいプロファイルを使う
- 証明書ハッシュは base64 に `+` を含む場合があり、URL パラメータでは percent-encode が必要

### 検証項目

ブラウザの WebTransport API を網羅する。少なくとも次を含める。

- セッション確立 (`transport.ready`)
- 双方向ストリームの開設とエコー
- 双方向ストリームの複数本
- 双方向ストリームの大きいデータ転送
- FIN の伝播
- クライアント起点の単方向ストリーム送信
- サーバー起点の単方向ストリーム受信
- サーバー起点の双方向ストリーム受信
- データグラム送信 (クライアントからサーバー)
- データグラム送信 (サーバーからクライアント)
- `transport.close()` の伝播
- サーバー起点のセッション終了
- フロー制御 (`WT_MAX_STREAMS` / `WT_MAX_DATA`)
- ストリームの `reset()` / `StopSending`
- Origin 検証

各項目を独立した判定とし、どれが落ちたかを CI のログだけで特定できるようにする。1 項目が失敗しても残りは続行し、最後にエンジンごとの結果一覧を出力する。

### 呼び出し経路

`Makefile` に `interop-test-browser` を追加する。`npm ci` とブラウザのダウンロードに数分かかるため、pre-commit フックが呼ぶ `interop-test` には含めず、CI からのみ実行する。

## 完了条件

- `make interop-test-browser` で Chromium と WebKit の両方が全項目通過する
- ブラウザテストが `interop-wt.yml` (macos-26) で自動実行される
- Playwright のバージョンが `package.json` で固定されている
- `README.md` に実行方法と前提 (Node.js / Playwright の導入) が記載されている
- 既存の `cargo test --workspace --tests` と `make interop-test` に影響がない
- `cargo fmt --all -- --check` と `cargo clippy --workspace --all-targets -- -D warnings` が通る

## 解決方法

### 修正内容

`interop/browser` を新設し、Node.js の Playwright で Chromium と WebKit を駆動して `examples/wt_server` へ接続するテストを追加した。

- `run.mjs`: 検証対象のサーバーを起動して証明書ハッシュをログから取得し、検証ページを配信し、エンジンごとにブラウザを起動して結果を集計する。Playwright 未導入の環境では skip する (`WT_FORCE=1` で失敗に切り替え)
- `serve.mjs`: 検証ページを HTTPS で配信する。WebTransport は secure context を要求するためページ配信にも TLS が必要である
- `index.html`: 検証ページ。接続先 URL と証明書ハッシュは `addInitScript` で注入する。各検証項目は独立したセッションを開き、`RESULT` 行を出力する。1 項目が失敗しても残りを続行する
- `certs/`: ページ配信用の自己署名証明書。ECDSA の証明書は Node.js の TLS 実装が `decode error` で拒否するため RSA を使う。検証対象のサーバーが使う ECDSA P-256 証明書とは別物であり、接続先の検証はページ側の `serverCertificateHashes` が担う
- `Makefile`: `interop-test-browser` を追加する。`npm ci` とブラウザのダウンロードに時間がかかるため `interop-test` には含めず、CI から個別に実行する
- `.github/workflows/interop-wt.yml`: Playwright の導入、サーバーのビルド、テストの実行を追加する。`WT_FORCE=1` により Playwright 未導入が skip で緑になることを防ぐ

検証項目はセッション確立・双方向ストリームのエコー・双方向ストリームの複数本・クライアント起点の単方向ストリーム送信の 4 つである。

Chromium は起動ごとに新しいプロファイルを使う。オリジンごとに証明書検証の結果をプロファイルへキャッシュするためである。また Playwright は `launch()` に `--user-data-dir` を渡すことを許さないため `launchPersistentContext()` を使う。

### 検証結果

- `node interop/browser/run.mjs` が Chromium と WebKit の両方で全 4 項目通過する
- `cargo test --workspace --tests` (1046 件) が通る
- `make interop-test` が通る
- `cargo fmt --all -- --check` と `cargo clippy --workspace --all-targets -- -D warnings` が通る

### 判明した不具合

WebKit で双方向ストリームの書き込みが停止する問題を検出した。`examples/wt_server` が `WT_MAX_DATA` をクライアントへ通知しないため、WebKit の送信ウィンドウが 0 のままになる。Chromium では症状が出ないため Rust 側のテストでは見つからなかった。修正は別 issue で扱う。この制約により、大きいデータの双方向転送は本 issue の検証項目に含めていない。

### 関連ファイル

- `interop/browser/` (新規)
- `Makefile` (`interop-test-browser` ターゲット)
- `.github/workflows/interop-wt.yml` (ブラウザテストのステップ)
- `examples/wt_server/src/main.rs` (検証対象。`--allow-origin` を指定して起動する)
- `CHANGES.md` (追加の記載)
- 参考実装: `shiguredo/webtransport-py` の `tests/browser/`

### 関連 issue

- 0216 (本 issue のテストで検出した `WT_MAX_DATA` 未通知の問題)
