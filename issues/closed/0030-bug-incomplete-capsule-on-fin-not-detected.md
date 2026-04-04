# CONNECT ストリーム FIN 時に未完成 Capsule を malformed として検出していない

Created: 2026-04-05
Completed: 2026-04-05
Model: Opus 4.6

## 優先度

P1

## 概要

WebTransport CONNECT ストリームが FIN で終端した際、`capsule_buf` に残った不完全な Capsule データを malformed message として検出していない。未完成 Capsule が無視されたままセッション終端に進む。

## 根拠

`process_wt_capsule_data()` (`src/connection/mod.rs:1223`) は完全な Capsule が得られるまでデータを `capsule_buf` にバッファリングする設計になっている。

`StreamEnd` 処理 (`src/connection/mod.rs:1852-1865`) では:

1. content-length の整合性を検証 (L1853-1859)
2. `Event::StreamEnd` を発行 (L1861)
3. `terminate_wt_session()` を呼ぶ (L1865)

この流れで `capsule_buf` に残ったデータの確認が一切ない。FIN 到着時に不完全な Capsule が残っている場合、それは送信側が Capsule を途中で打ち切ったことを意味し、malformed message として `H3_MESSAGE_ERROR` で拒否すべき。

## 再現手順

1. WebTransport セッションを確立する
2. CONNECT ストリーム上で Capsule のヘッダー部分のみ送信し FIN を送る
3. サーバー側でエラーなくセッションが終端される

## 対応方針

`StreamEnd` 処理で `terminate_wt_session()` を呼ぶ前に、対象セッションの `capsule_buf` が空でないかを確認する。空でない場合は `H3_MESSAGE_ERROR` を返す。

## 解決方法

`StreamEnd` 処理で `terminate_wt_session()` を呼ぶ前に、対象セッションの `capsule_buf` が空でないかを確認するガードを追加した。空でない場合は `H3_MESSAGE_ERROR` でストリームエラーを返す。

## 参照

- draft-ietf-webtrans-http3-15 Section 5.6
- RFC 9114 Section 4.1.2 (malformed message handling)
- `src/connection/mod.rs:1223` (process_wt_capsule_data)
- `src/connection/mod.rs:1852-1865` (StreamEnd 処理)
