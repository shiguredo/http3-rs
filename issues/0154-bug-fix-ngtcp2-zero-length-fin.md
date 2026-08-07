# tokio-ngtcp2 の 0 長データ + FIN が破棄され終端が送信されない

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-ngtcp2-zero-length-fin
- Polished: {YYYY-MM-DD}

## 目的

nghttp3 の「0 長データ + FIN」を送信できない問題 (0 バイトボディの POST、空レスポンスでピアに終端が届かない) を修正する。

## 現状

- 送信ループ (クライアント: `crates/tokio-ngtcp2/src/client.rs` の `write_h3_streams`、サーバー: `server.rs` の `write_h3_streams`、WebTransport: `webtransport.rs` の `write_h3_streams` / `write_h3_streams_tracked` / サーバー側ヘルパー) は `count == 0` で break する
- `crates/nghttp3-sys/src/bindings.rs` の `nghttp3_conn_writev_stream` ドキュメントは「count が 0 で `*pfin` が非ゼロのケース (0 長データ + FIN) は QUIC スタックに渡し、`nghttp3_conn_add_write_offset` を 0 バイトで呼ぶこと」と規定しており、このケースで FIN が送信されない
- さらに `data_written > 0` のガードにより、fin のみのケースの `add_write_offset(stream_id, 0)` もスキップされ、nghttp3 が FIN 送信完了を認識しない
- テストは「データ + FIN 一体型」のみで、FIN のみの送信経路が検出されていない

## 設計方針

- `count == 0` でも `fin == true` の場合は処理を継続して QUIC に FIN を渡す
- `data_written == 0` でも fin が立っている場合は `add_write_offset(stream_id, 0)` を呼ぶ

## 完了条件

- 0 バイトボディ + FIN の送信でピアに StreamEnd が届く
- テストが追加される (0 長ボディの POST / 空レスポンス)
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `crates/tokio-ngtcp2/src/client.rs` (`write_h3_streams`)
- `crates/tokio-ngtcp2/src/server.rs` (`write_h3_streams`)
- `crates/tokio-ngtcp2/src/webtransport.rs` (`write_h3_streams` / `write_h3_streams_tracked` / サーバー側ヘルパー)
- 参照: `crates/nghttp3-sys/src/bindings.rs` (`nghttp3_conn_writev_stream` / `nghttp3_conn_add_write_offset` のドキュメント)
