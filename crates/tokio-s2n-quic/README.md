# shiguredo_s2n_quic

shiguredo_http3 と s2n-quic を組み合わせた HTTP/3 と WebTransport over HTTP/3 のリファレンス実装です。

## 概要

[s2n-quic](https://github.com/aws/s2n-quic) は AWS が開発した Rust ネイティブの QUIC 実装です。
このクレートは shiguredo_http3 と s2n-quic を組み合わせて HTTP/3 通信を行うサンプルを提供します。

## 特徴

- shiguredo_http3 の Sans I/O 設計を活用
- s2n-quic による QUIC トランスポート
- Tokio 非同期ランタイム

## WebTransport over HTTP/3

WebTransport のセッション API を提供します。

s2n-quic 1.91 は `RESET_STREAM_AT` (`draft-ietf-quic-reliable-stream-reset`) の送出 API も、ピアの `reset_stream_at` transport parameter を取得する API も公開していません。このクレートは、sans-I/O 層の能力検証に `reset_stream_at` 対応済み (`true`) を渡します。利用者が `enable_webtransport` と `wt_draft_version` を相手の版に合わせて設定した場合に限り、draft-14 / 15 / 16 / 17 のピアとのセッションを確立できます。ただし、実際のリセットは `RESET_STREAM_AT` ではなく通常の `RESET_STREAM` になり、draft-14 以降が要求するストリームヘッダーの確実な配送は保証されません。s2n-quic が両 API を公開するまでの既知の制限です。

`ClientConfig::wt_draft_version` (既定値: `Draft15`) は、CONNECT の `:protocol` のトークン系統 (`webtransport` / `webtransport-h3`) だけを決めます。接続で使う版は SETTINGS のコードポイントで決まるため、この設定で版を固定するものではありません。接続で選択された版のトークン系統と設定が一致しない場合、CONNECT は `WtSetupError::ProtocolMismatch` で拒否されます。相手が使う版に合わせて設定してください。

## サンプルの実行

### サーバー

```bash
cargo run --example shiguredo_s2n_quic_server
```

### クライアント

```bash
cargo run --example shiguredo_s2n_quic_client
```

## 依存関係

- `shiguredo_http3` - Sans I/O HTTP/3 ライブラリ
- `s2n-quic` - AWS の QUIC 実装
- `tokio` - 非同期ランタイム

## s2n-quic ライセンス

<https://github.com/aws/s2n-quic/blob/main/LICENSE>

```text
// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
```

## ライセンス

Apache License 2.0

```text
Copyright 2026-2026, Shiguredo Inc.

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
```
