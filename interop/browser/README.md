# interop_browser

ブラウザ (Chromium / WebKit) との WebTransport 相互運用テスト

## 概要

実ブラウザが公開している WebTransport API を網羅的に呼び出し、検証対象の
WebTransport サーバーが応答できることを確認するテストスイート。

Rust 側のテスト (`interop/wt`) ではブラウザ固有の要件を再現できない。実際に
Safari 26.4 は draft-07 と draft-13/14 のハイブリッド実装であり、サーバーが
返してよい WT 系応答 SETTINGS が `SETTINGS_WEBTRANSPORT_MAX_SESSIONS` 単体に
限られる (`docs/SAFARI_WT.md`)。この制約はブラウザで接続して初めて分かる。

## テスト対象

| 実装 | エンジン | 備考 |
|---|---|---|
| Chromium | Playwright の chromium | `examples/wt_server` に接続する |
| WebKit | Playwright の webkit | Safari 相当。Safari 固有の制約を持つ |

検証対象のサーバーは `examples/wt_server` のバイナリである。

## 実行方法

```bash
cargo build --manifest-path examples/wt_server/Cargo.toml
cd interop/browser
npm ci
npx playwright install chromium webkit
npm test
```

`node_modules` とブラウザが揃っていない環境では skip する。CI では必ず導入する。
skip を失敗として扱いたい場合は `WT_FORCE=1` を設定する。

### 環境変数

| 変数 | 既定値 | 説明 |
|---|---|---|
| `WT_BROWSER_ENGINES` | `chromium,webkit` | 検証するエンジン (カンマ区切り) |
| `WT_SERVER_BIN` | `target/debug/wt_server` | 検証対象のサーバーバイナリ |
| `WT_PORT` | `4443` | 検証対象のサーバーのポート |
| `WT_PAGE_PORT` | `0` | 検証ページのポート (0 は自動割り当て) |
| `WT_FORCE` | 未設定 | `1` のとき Playwright 未導入でも失敗させる |

## 構成

| ファイル | 役割 |
|---|---|
| `run.mjs` | サーバー起動、検証ページの配信、ブラウザの駆動、判定 |
| `serve.mjs` | 検証ページを HTTPS で配信する |
| `index.html` | 検証ページ。WebTransport クライアントとして各項目を実行する |
| `certs/` | 検証ページ配信用の自己署名証明書 (RSA) |

検証ページの配信に HTTPS が必要なのは、WebTransport が secure context を要求する
ためである。ページ配信の証明書は RSA を使う。ECDSA の証明書は Node.js の TLS
実装が受け付けない (ハンドシェイクが decode error になる) ためである。検証対象の
サーバーが使う ECDSA P-256 証明書とは別物であり、接続先の検証はページ側の
`serverCertificateHashes` が担う。

## 検証項目

`index.html` の `checks` に定義する。各項目は独立した WebTransport セッションを
開き、結果を `RESULT` 行として出力する。1 項目が失敗しても残りを続行する。

| 項目 | 内容 |
|---|---|
| `session` | セッション確立 (`transport.ready`) |
| `bidiEcho` | 双方向ストリームの開設とエコー |
| `bidiMultiple` | 双方向ストリームの複数本 |
| `uniSend` | クライアント起点の単方向ストリーム送信 |

## 既知の制約

- 検証対象のサーバーは draft-07 形状の応答 SETTINGS で初期フロー制御値を広告
  できないため、`WT_MAX_DATA` がクライアントへ届かず、WebKit では双方向
  ストリームへの書き込みが停止する。このため大きいデータの転送は検証項目に
  含めていない
- 双方向ストリームのデータ経路は、初期フロー制御の範囲内でのみ検証している
- サーバー起点の単方向 / 双方向ストリーム、データグラム、`reset()`、
  `StopSending` は未検証である

## ブラウザ起動時の注意

- Chromium は起動ごとに新しいプロファイルを使う。オリジンごとに証明書検証の
  結果をプロファイルへキャッシュするためである
- Chromium を `launch()` で起動する際に `--user-data-dir` を渡すことは
  Playwright が許さないため、`launchPersistentContext()` を使う
- 検証ページは接続先と同じホスト (`127.0.0.1`) で配信する。公開オリジンから
  ローカルアドレスへ接続する構成では、Chromium の Local Network Access チェックに
  より接続がブロックされる
