# :protocol 疑似ヘッダーの値を draft-15 に整合させる

Created: 2026-03-29
Completed: 2026-03-29
Model: Opus 4.6

## 概要

`ConnectRequest` のドキュメントと実際の使用箇所で `:protocol` の値が不整合になっている問題を解決する。

## 根拠

- draft-ietf-webtrans-http3-02 Section 3.2: `:protocol` = `webtransport`
- draft-ietf-webtrans-http3-15 Section 3.2: `:protocol` = `webtransport-h3`

`ConnectRequest` のドキュメントは `webtransport-h3` (draft-15) と記載しているが、moqt-rust-private の examples は `webtransport` (draft-02) を使用している。

shiguredo_http3 は draft-15 準拠を謳っているので、ライブラリとしては `webtransport-h3` が正。ただし Chrome 等の実装が draft-02 互換で `webtransport` を送る場合があるため、サーバー側での受信時は両方を受け入れる必要がある可能性がある。

## 対応方針

- `ConnectRequest::to_headers()` (#0001) では `:protocol` = `webtransport-h3` を生成する
- `ConnectRequest::from_headers()` (#0003) では `webtransport` と `webtransport-h3` の両方を受け入れる
- 定数として `PROTOCOL_WEBTRANSPORT_H3` と `PROTOCOL_WEBTRANSPORT_DRAFT02` を定義する
- draft-02 互換の値を受け入れることをコードコメントで明記し、将来変更される可能性を注記する

## 解決方法

`src/webtransport/connect.rs` に以下の定数を追加した:

- `PROTOCOL_WEBTRANSPORT_H3 = "webtransport-h3"` (draft-15)
- `PROTOCOL_WEBTRANSPORT_DRAFT02 = "webtransport"` (draft-02 互換)

`ConnectRequest::to_headers()` は draft-15 の `webtransport-h3` を生成し、`ConnectRequest::from_headers()` は両方の値を受け入れる。
