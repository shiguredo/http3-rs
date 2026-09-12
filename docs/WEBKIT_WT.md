# WebKit WebTransport 対応状況

## 確認環境

- WebKit 26.6 (Playwright 1.63.0 の `webkit` エンジン、macOS 26 arm64)
- 検証対象: `examples/wt_server`
- 初回確認日: 2026-09-12

## TL;DR

- WebKit から `examples/wt_server` へ接続し、セッション確立・双方向ストリームの小さいデータの往復・
  単方向ストリーム送信は成功する
- **双方向ストリームへ 4 KiB 以上を書き込むと `writer.write()` が完了しない**。
  1 KiB は成功し、4 KiB で停止する
- 停止時、**サーバーにはクライアントのデータが 1 バイトも届かない**。
  サーバーの受信ログに `Bidi stream N: received` が出ない
- 同じサーバーへ Chromium から接続した場合は 512 KiB の往復も成功する
- **原因は未確定**。本リポジトリ側の問題か WebKit (Network.framework) 側の問題かを切り分けられていない

## 検証方法

`interop/browser` の検証ページで、双方向ストリームを開いて `payload` を書き込み、
エコーを読み戻すまでの時間を計測する。

```js
const stream = await transport.createBidirectionalStream();
const writer = stream.writable.getWriter();
await writer.write(payload);   // ここで停止する
await writer.close();
const { value, done } = await stream.readable.getReader().read();
```

各サイズは独立したセッションで 1 回ずつ計測する (セッションを使い回すと前の結果に影響されるため)。

## 実測結果

### データサイズと成否

| サイズ | WebKit | Chromium |
|---|---|---|
| 17 バイト | 成功 | 成功 |
| 1 KiB | 成功 | 成功 |
| 4 KiB | **停止 (15 秒でタイムアウト)** | 成功 |
| 8 KiB | **停止** | 成功 |
| 16 KiB | **停止** | 成功 |
| 64 KiB | **停止** | 成功 |
| 256 KiB | **停止** | 成功 |
| 512 KiB | **停止** | 成功 |

閾値は 1 KiB と 4 KiB の間にあり、4 KiB 以上はすべて停止する。
サイズを変えても停止の仕方は同じ (タイムアウトまで戻らない)。

### 停止時にサーバーが観測したこと

検証セッションのサーバーログ。CONNECT ストリームは受信しているが、
双方向ストリームのデータが届いていない。

```
INFO wt_server::webtransport: WebTransport: received CONNECT bidi stream 0 (0x0)
INFO wt_server::webtransport: WebTransport: received 82 bytes on CONNECT stream
INFO wt_server: [127.0.0.1:56590] CONNECT stream received 9 bytes: [00, 07, 68, 43, 04, 00, 00, 00, 00]
    ← 15 秒間、Bidi stream N: received が出ない
```

参考として、成功した小さいデータの場合は次のようにデータ受信が記録される。

```
INFO wt_server: [127.0.0.1:58245] Bidi stream 4: received 18 bytes (total: 18)
INFO wt_server: [127.0.0.1:58245] Bidi stream 4: closed (total received: 18 bytes)
```

### 初期フロー制御の値による差

サーバーがセッション確立直後に送る `WT_MAX_STREAMS` / `WT_MAX_DATA` カプセルの値を
変えても、閾値は変化しなかった。

| サーバーの初期クレジット | 4 KiB | 16 KiB |
|---|---|---|
| カプセルで指定する (`set_wt_session_flow_control_limits`) | 停止 | 停止 |
| 指定しない (ローカル SETTINGS の値を使う) | 停止 | 停止 |

この結果から、**停止は WebTransport のデータフロー制御の値に起因するものではない**と考えられる。

## 原因の切り分け状況

判明していることと未確定なことを分けて記録する。

### 判明していること

- 停止するのはクライアントの送信側である。`writer.write()` が完了しない
- サーバーは CONNECT ストリームを受信しており、セッションは確立している
- サーバーは双方向ストリームのデータを 1 バイトも受信していない
- Chromium では同じサーバーで 512 KiB まで成功する
- サーバーの初期クレジットの値を変えても閾値は変わらない

### 未確定なこと

- WebKit が STREAM フレームを送信しているかどうか (パケットレベルで未確認)
- WebKit が `createBidirectionalStream()` の直後に何を待っているか
- サーバーのカプセルを WebKit がどう解釈しているか
- サーバーの SETTINGS / カプセルの内容が原因かどうか

### 判断できない理由

`docs/SAFARI_WT.md` に記録されているとおり、Safari / WebKit の WebTransport は
Apple の Network.framework 経由で実装されており、H3 / QUIC のプロトコル処理は
クローズドソース側で行われる。WebKit のソースを読んでも原因特定ができない。

また、過去に Safari で「Datagram が受信できない」事象が W3C WebTransport API 側の
問題であった前例がある (`docs/SAFARI_WT.md` の Datagram の節)。ブラウザ側の問題で
ある可能性を排除できない。

## 現時点の運用

- `interop/browser` の検証項目は、WebKit でも成功する範囲 (小さいデータの往復) に留めている
- 大きいデータの往復は検証項目に含めていない。含めると WebKit で常に失敗する
- 本ドキュメントの閾値は、検証項目を増やす際の判断材料として使う

## 再現方法

```bash
cargo build --manifest-path examples/wt_server/Cargo.toml
cd interop/browser
npm ci
npx playwright install chromium webkit
node run.mjs          # 検証項目 (小さいデータ) は WebKit でも成功する
```

サイズごとの閾値を確認する場合は、検証ページに一時的な計測項目を追加して
`WT_BROWSER_ENGINES=webkit` で実行する (上記の表はこの方法で計測した)。

## rcgen が生成する証明書について (参考)

WebKit / Chromium へ接続する検証では、自己署名証明書を `serverCertificateHashes` で
ピン留めする。このとき **`rcgen` が生成する ECDSA 鍵は Chrome が受け付けない**。

`rcgen` の `generate_simple_self_signed` が生成する鍵は PKCS#8 の ECPrivateKey 内で
explicit parameters 形式を使っており、Chrome は TLS ハンドシェイクで
`DECODE_ERROR` を返す。named curve (`prime256v1` 等) の OID を使う鍵が必要である。

`examples/wt_server` は openssl 相当の named curve 証明書を生成するため問題ないが、
検証用の証明書を自前で生成する場合は注意すること。ページ配信側の証明書は
Node.js の TLS 実装が ECDSA を `decode error` で拒否するため RSA を使っている
(`interop/browser/certs/`)。

## 参照

- `docs/SAFARI_WT.md` (Safari 固有の SETTINGS 制約とカプセル要件)
- `interop/browser/README.md` (検証の実行方法)
- `refs/webtrans/draft-ietf-webtrans-http3-14.txt` Section 5 (カプセルベースフロー制御)
- `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 5 (フロー制御)
- RFC 9000 (QUIC)
- RFC 9114 (HTTP/3)
