# tokio-ngtcp2

shiguredo_http3 と [shiguredo_ngtcp2_tokio](https://crates.io/crates/shiguredo_ngtcp2_tokio) を組み合わせた Tokio 非同期 HTTP/3 / WebTransport クライアント / サーバーです。

## 概要

QUIC トランスポートは crates.io の `shiguredo_ngtcp2_tokio` が提供するイベントベースの API を使用し、HTTP/3 の状態機械は `shiguredo_http3` が担います。このクレートは両者をつなぐドライバと、相互運用テストで使用するクライアント / サーバー API を提供します。

## 依存関係

- `shiguredo_http3` - Sans I/O HTTP/3 / WebTransport
- `shiguredo_ngtcp2_tokio` - ngtcp2 の tokio 統合 (crates.io)
- `tokio` - 非同期ランタイム

## ライセンス

Apache License 2.0
