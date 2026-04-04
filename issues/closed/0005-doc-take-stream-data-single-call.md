# take_stream_data() のドキュメントに 1 回呼び出しで全データ返却する旨を明記する

Created: 2026-03-29
Completed: 2026-03-29
Model: Opus 4.6

## 概要

`Connection::take_stream_data()` が 1 回の呼び出しで全バッファデータを返すことをドキュメントに明記する。

## 根拠

`take_stream_data()` の実装は `get_stream_data()` で全バッファの参照を取得し、`to_vec()` でコピー後、`consume_stream_data()` で全量消費する。つまり 1 回の呼び出しで全データを返す。

しかし moqt-rust-private の publisher / subscriber / relay の全てで while ループで呼んでいる:

```rust
let mut data = Vec::new();
while let Some((chunk, _fin)) = s.h3_conn.take_stream_data(h3_id) {
    data.extend_from_slice(&chunk);
}
```

これは API の契約が不明確なために防御的に書かれたコード。ドキュメントに明記すればループは不要になり、利用者コードが簡潔になる。

## 対応方針

- `Connection::take_stream_data()` の doc comment に以下を追記する:
  - 「ストリームの送信バッファにある全データを 1 回の呼び出しで返す。ループで呼ぶ必要はない」
- `ClientConnection::take_stream_data()` と `ServerConnection::take_stream_data()` の doc comment も同様に更新する

## 解決方法

`Connection::take_stream_data()`, `ClientConnection::take_stream_data()`, `ServerConnection::take_stream_data()` の doc comment に「ストリームの送信バッファにある全データを 1 回の呼び出しで返す。ループで繰り返し呼ぶ必要はない」を追記した。
