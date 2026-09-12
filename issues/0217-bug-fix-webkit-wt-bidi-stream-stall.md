# WebKit から双方向ストリームへ 4 KiB 以上を書き込むと完了しない

- Created: 2026-09-12
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-webkit-wt-bidi-stream-stall
- Polished: {YYYY-MM-DD}

## 目的

WebKit (Safari 相当) から `examples/wt_server` の双方向ストリームへ 4 KiB 以上を書き込むと `writer.write()` が完了しない問題を解決する。Chromium では同じ操作が 512 KiB まで成功しており、ブラウザ間で挙動が異なる状態を解消する。

## 現状

`interop/browser` の検証ページで、独立したセッションごとに双方向ストリームへ書き込み、エコーを読み戻すまでの時間を計測した。

| サイズ | WebKit | Chromium |
|---|---|---|
| 1 KiB | 成功 | 成功 |
| 4 KiB | 停止 (15 秒でタイムアウト) | 成功 |
| 16 KiB | 停止 | 成功 |
| 256 KiB | 停止 | 成功 |
| 512 KiB | 停止 | 成功 |

停止時、サーバーは CONNECT ストリームを受信しているが、双方向ストリームのデータを 1 バイトも受信していない。サーバーログに `Bidi stream N: received` が出ない。

```
INFO wt_server::webtransport: WebTransport: received CONNECT bidi stream 0 (0x0)
INFO wt_server::webtransport: WebTransport: received 82 bytes on CONNECT stream
INFO wt_server: [127.0.0.1:56590] CONNECT stream received 9 bytes: [00, 07, 68, 43, 04, 00, 00, 00, 00]
    ← 15 秒間、Bidi stream N: received が出ない
```

サーバーがセッション確立直後に送る `WT_MAX_STREAMS` / `WT_MAX_DATA` カプセルの値を変えても閾値は変化しない。このため WebTransport のデータフロー制御の値に起因するものではないと考えられる。

### 原因が未確定である理由

停止しているのがクライアントの送信側であることは判明しているが、次のいずれかを特定できていない。

- WebKit が QUIC の STREAM フレームを送信していない
- WebKit が `createBidirectionalStream()` の直後に何かを待っている
- サーバーの SETTINGS / カプセルの解釈に問題がある

`docs/SAFARI_WT.md` に記録されているとおり、Safari / WebKit の WebTransport は Apple の Network.framework 経由で実装され、H3 / QUIC のプロトコル処理はクローズドソース側で行われる。WebKit のソースを読んでも原因を特定できない。また、過去に Safari で「Datagram が受信できない」事象が W3C WebTransport API 側の問題であった前例があり、ブラウザ側の問題である可能性を排除できない。

実測の詳細は `docs/WEBKIT_WT.md` に記録している。

## 設計方針

原因を確定させてから修正方針を決める。原因がこのリポジトリ側にあるのか WebKit 側にあるのかで対応が変わる。

### 原因確定のために必要な作業

1. WebKit が送信する QUIC パケットを捕捉し、双方向ストリームの STREAM フレームが送出されているかを確認する。送出されていれば WebKit 側の送信待ち、送出されていなければこのリポジトリ側の広告内容を疑う
2. WebKit が `createBidirectionalStream()` の直後に何を待っているかを実測する (DevTools 等で内部状態を観察する)
3. サーバーの `WT_MAX_STREAMS` / `WT_MAX_DATA` カプセルを WebKit がどう解釈しているかを確認する。カプセルを送らない場合・値を変えた場合の挙動差を測る

### 原因判明後の分岐

- このリポジトリ側の問題であれば修正する
- WebKit 側の問題であれば `docs/WEBKIT_WT.md` に記録して本 issue は closed にする

## 完了条件

- 停止の原因がこのリポジトリ側か WebKit 側かが確定している
- このリポジトリ側であれば、WebKit から 512 KiB の双方向往復が成功する
- WebKit 側であれば、その根拠が `docs/WEBKIT_WT.md` に記録されている
- 原因確定後、`interop/browser` の検証項目に大きいデータの双方向転送を追加できる状態になっている

## 解決方法

### pending にする理由

停止の原因がこのリポジトリ側か WebKit 側かを確定できていないため pending にする。原因が未確定のままでは修正方針を決められず、着手しても完了条件を満たせるかどうかを判断できない。

`docs/WEBKIT_WT.md` に記録したとおり、次に必要なのはパケットレベルの観測であり、これを行わないと「WebKit が STREAM フレームを送っていない」のか「送っているがサーバーが受け取れていない」のかを区別できない。原因が WebKit 側であればこのリポジトリでの修正対象は無く、ドキュメントへの記録で closed になる。

原因が確定した時点で reopened にして、判明した内容に応じて設計方針と完了条件を書き直す。

### 関連ファイル

- `interop/browser/index.html` (検証ページ)
- `examples/wt_server/src/webtransport.rs` (`WtSessionRequest::from_connection` / `WtSessionRequest::accept`)
- `examples/wt_server/src/main.rs` (`handle_bidi_echo`)
- `docs/WEBKIT_WT.md` (実測の記録)
- `docs/SAFARI_WT.md` (Safari の制約と、ブラウザ側問題の前例)
- 一次資料: `refs/webtrans/draft-ietf-webtrans-http3-14.txt` Section 5 (カプセルベースフロー制御)
