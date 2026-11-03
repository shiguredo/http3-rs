# tokio-ngtcp2

shiguredo_http3 と [shiguredo_ngtcp2_tokio](https://crates.io/crates/shiguredo_ngtcp2_tokio) を組み合わせた Tokio 非同期 HTTP/3 / WebTransport クライアント / サーバーです。

## 概要

QUIC トランスポートは crates.io の `shiguredo_ngtcp2_tokio` が提供するイベントベースの API を使用し、HTTP/3 の状態機械は `shiguredo_http3` が担います。このクレートは両者をつなぐドライバと、相互運用テストで使用するクライアント / サーバー API を提供します。

## WebTransport over HTTP/3

WebTransport のクライアント / サーバー API を提供します。

- SETTINGS では draft-02 / 07 / 14 / 15 / 16 / 17 のコードポイントを広告します。draft-15 と draft-16 は同じコードポイント (`0x2c7cf000`)、draft-17 は別のコードポイント (`0x20b8933b`) です。接続で使う版は SETTINGS のコードポイントから接続単位で決めます
- `SETTINGS_WT_INITIAL_MAX_STREAMS_*` / `SETTINGS_WT_INITIAL_MAX_DATA` を送信しません。そのため WebTransport のフロー制御は無効になり、ストリーム数とデータ量は QUIC のフロー制御だけで制限されます。相互運用テストではフロー制御カプセルのやり取りを検証しません
- クライアントの SETTINGS に応じて応答 SETTINGS を変えません。Safari 26.4 (Network.framework) が要求する「draft-07 のコードポイントだけを返す」制約を満たさないため、Safari との相互運用には別途対応が必要です。sans-I/O 層は接続ごとに SETTINGS を構築できるため、この制約はアダプター固有のものです
- `reset_stream_at` transport parameter の対応状況は下位 QUIC 層に依存します。このクレートは前提条件を検証せず、対応済みとして扱います。Reliable Size を伴うリセットの配送保証は、下位 QUIC 層の API が提供する範囲に限られます

## 依存関係

- `shiguredo_http3` - Sans I/O HTTP/3 / WebTransport
- `shiguredo_ngtcp2_tokio` - ngtcp2 の tokio 統合 (crates.io)
- `tokio` - 非同期ランタイム

## ライセンス

Apache License 2.0
