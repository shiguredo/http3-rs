# Pending WebTransport セッションに紐づく先行ストリームのイベントを即時発火している

Created: 2026-04-06
Completed: 2026-04-07
Model: Opus 4.6

## 解決方法

`WtSession` に `buffered_stream_entries: HashMap<u64, BufferedStreamEntry>` を追加し、Pending セッション宛の先行ストリームについて Open / Data / End イベントを発火せず、ペイロードと FIN フラグを蓄積するように変更した。`associate_or_buffer_stream()` の戻り値を `AssocOutcome { Established, Buffered, BufferOverflow }` の三値に拡張し、呼び出し側で Pending 経路と Established 経路を分岐させる。`handle_unidirectional_stream` / `handle_wt_bidi_stream` のデータ・FIN 受信パスでも Pending セッションを判定してバッファに追記する。クライアント / サーバー双方の establishment パス (`process_recv_headers` / `send_response`) で、ストリーム数フロー制御 + データ量フロー制御を改めて適用しつつ、`buffered_streams` 順に Open → Data → End イベントを一括発火する。バッファペイロードには `WT_MAX_BUFFERED_STREAM_BYTES = 64KB` の DoS 上限を設け、超過時は `WT_BUFFERED_STREAM_REJECTED` でストリームを拒否する。Pending 中の先行ストリームペイロードがバッファに積まれ、Open/Data イベントが発火されないことを確認する単体テスト `test_wt_pending_stream_data_buffered_until_established` を追加した。

## 優先度

P1

## 概要

`associate_or_buffer_stream()` で先行 WT stream を `Pending` セッションに紐づけてバッファリング扱いにした後も、呼び出し元 (`resolve_wt_uni_stream_header`, `resolve_wt_bidi_stream_header`) が `WebTransportUniStreamOpen` / `WebTransportBidiStreamOpen` および続く `*StreamData` イベントをそのまま `events` キューに `push_back` してしまう。

draft-ietf-webtrans-http3-15 Section 4.6 では「セッションが established になるまでバッファリングする」ことを要求しており、アプリケーションが 2xx 応答前のストリームペイロードを観測できる現状は仕様違反かつ Sans I/O の状態機械としても破綻している。

## 根拠

- draft-ietf-webtrans-http3-15 Section 4.6
- nghttp3 `lib/nghttp3_conn.c` L3694, L3793: pending session に紐づく stream は buffer に積み、確立時に解放
- `src/connection/mod.rs` L1261-1306 (`resolve_wt_uni_stream_header`)
- `src/connection/mod.rs` L1825-1892 (`resolve_wt_bidi_stream_header`)
- `src/connection/mod.rs` L1744-1775 (`associate_or_buffer_stream`)

`buffer_stream()` は `wt_sessions[session_id].buffered_streams` に stream_id を積むだけで、その後到着するペイロードや FIN を保持する仕組みが存在しない。

## 影響

- 2xx 応答前にアプリケーションが先行ストリームの Open / Data / End を観測できてしまう
- セッションが結局拒否 (4xx) された場合でもアプリ側にイベントが残る
- Sans I/O API としての健全性が崩れる

## 対応方針

1. `associate_or_buffer_stream()` の戻り値を 3 値化する (例: `enum AssocOutcome { Established, Buffered, BufferOverflow, Gone }`)
2. `WtSession` に「先行ストリームに紐づくペイロードと FIN 状態」を保持する構造を追加する (datagram 側の `buffer_datagram()` と対称)
3. `resolve_wt_uni_stream_header` / `resolve_wt_bidi_stream_header` および後続の Data / StreamEnd 受信パスで、Pending セッション宛のものは `events` に流さず `WtSession` 内部に積む
4. セッション確立時 (CONNECT 2xx 応答確定時) に、バッファした stream について `Open` → `Data` → 必要なら `StreamEnd` の順でイベントを一括発火する
5. セッションが拒否 (4xx 等) された場合は、バッファしたストリームを `WT_SESSION_GONE` 相当で破棄する
6. PBT で「buffered → established 解放」「buffered → rejected 破棄」の両ラウンドトリップを検証する

## 参照

- draft-ietf-webtrans-http3-15 Section 4.6
- nghttp3 `lib/nghttp3_conn.c` L3694, L3793
- `src/connection/mod.rs` L1261, L1303, L1825, L1867
