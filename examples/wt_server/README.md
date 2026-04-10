# wt_server

WebTransport エコーサーバーのサンプル実装。

draft-ietf-webtrans-http3 の draft-02 / 07 / 14 / 15 に対応しており、クライアントの SETTINGS から draft バージョンを自動判定して応答する。

接続後はエコーサーバーとして動作する。

- 双方向ストリーム: 受信データをそのまま返す
- 単方向ストリーム: 受信データを新しい単方向ストリームで返す
- DATAGRAM (RFC 9221): 受信データをそのまま返す

## 起動方法

workspace に含まれていないため、`examples/wt_server` ディレクトリで直接実行する。

デフォルト (`127.0.0.1:4443`) で起動する。

```bash
cd examples/wt_server
cargo run
```

リッスンアドレスを指定する。

```bash
cd examples/wt_server
cargo run -- --listen 127.0.0.1:4443
```

全ての WebTransport CONNECT を 404 で拒否する (`WtSessionRequest::reject()` の動作確認用)。

```bash
cd examples/wt_server
cargo run -- --reject-connect
```

ログレベルを変更する。

```bash
cd examples/wt_server
RUST_LOG=debug cargo run
```

## オプション

| オプション | 説明 | デフォルト |
| --- | --- | --- |
| `-l`, `--listen <ADDR>` | リッスンアドレス | `127.0.0.1:4443` |
| `--reject-connect` | 全セッションを 404 で拒否 | 無効 |
| `-h`, `--help` | ヘルプを表示 | |
| `--version` | バージョンを表示 | |

## 証明書

起動時に `rcgen` で自己署名証明書を自動生成する。クライアント側は証明書検証をスキップするか、生成された証明書を信頼する必要がある。
