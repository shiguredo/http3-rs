# wt_server

WebTransport エコーサーバーのサンプル実装。

draft-ietf-webtrans-http3 の draft-02 / 07 クライアントに接続できる。クライアントの SETTINGS から draft バージョンを判定し、接続ごとに応答 SETTINGS を構築する。

この example が使う s2n-quic 1.91 は RESET_STREAM_AT の送出 API も、ピアの `reset_stream_at` transport parameter を取得する API も公開していない。そのため draft-14 / 15 / 16 / 17 では必須となる `reset_stream_at` に対応できず、当該 draft のクライアントからは CONNECT が `H3_MESSAGE_ERROR` で拒否される。この example は Safari 以外のクライアントには draft-02 / 07 / 14 / 15 / 16 / 17 のコードポイント (draft-15 と 16 は同じコードポイント) を広告するが、実際に接続できるのは draft-02 / 07 クライアントに限られる (s2n-quic が両 API を公開するまでの既知の制限)。

- Safari 26.4 (Network.framework) は draft-07 と draft-14 のコードポイントを併送する。この組み合わせを検出した場合、応答の WebTransport のコードポイントには draft-07 だけを含め、`SETTINGS_WT_INITIAL_MAX_*` も含めない。Safari は draft-14 のコードポイント (`0x14e9cd29`) や `SETTINGS_WT_INITIAL_MAX_*` を受け取ると、CONNECT を `H3_REQUEST_CANCELLED` で拒否するためである
- Safari 以外のクライアントには、対応するすべてのコードポイント (draft-02 / 07 / 14 / 15 / 16 / 17。draft-15 と 16 は同じコードポイント) と初期フロー制御値を返す
- 実際に使う版は、応答 SETTINGS の送信後に `ServerConnection::negotiated_wt_draft_version` で決める

sans-I/O 層の `shiguredo_http3` は draft-02 / 07 / 14 / 15 / 16 / 17 の SETTINGS 構築、版選択、フロー制御を提供する。しかし、この example と各アダプターが実際に保証できることは同じではない。sans-I/O 層が対応していても、下位 QUIC ライブラリが API を公開していなければ `reset_stream_at` や部分配送付きリセットは実現できない。example の動作を確認するときは、どの層が何を保証するかを区別すること。

接続後はエコーサーバーとして動作する。

- 双方向ストリーム: 受信データをそのまま返す
- 単方向ストリーム: 受信データを新しい単方向ストリームで返す
- DATAGRAM (RFC 9221): 受信データをそのまま返す

## 起動方法

workspace メンバーなのでリポジトリルートから `-p wt_server` で実行できる。

デフォルト (`127.0.0.1:4443`) で起動する。

```bash
cargo run -p wt_server
```

リッスンアドレスを指定する。

```bash
cargo run -p wt_server -- --listen 127.0.0.1:4443
```

すべての WebTransport CONNECT を 405 で拒否する (`WtSessionRequest::reject()` の動作確認用)。

```bash
cargo run -p wt_server -- --reject-connect
```

ログレベルを変更する。

```bash
RUST_LOG=debug cargo run -p wt_server
```

## オプション

| オプション | 説明 | デフォルト |
| --- | --- | --- |
| `-l`, `--listen <ADDR>` | リッスンアドレス | `127.0.0.1:4443` |
| `--reject-connect` | すべてのセッションを 405 で拒否 | 無効 |
| `-h`, `--help` | ヘルプを表示 | |
| `--version` | バージョンを表示 | |

## 証明書

起動時に `rcgen` で自己署名証明書を自動生成する。クライアント側は証明書検証をスキップするか、生成された証明書を信頼する必要がある。
