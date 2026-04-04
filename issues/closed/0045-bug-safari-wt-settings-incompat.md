# Safari 26.4 WebTransport 接続不可 (応答 SETTINGS に WT_INITIAL_MAX_* を含めると拒否される)

Created: 2026-04-06
Completed: 2026-04-07
Model: Opus 4.6

## 概要

Safari 26.4 (Network.framework) から WebTransport セッションを確立しようとすると、クライアントが H3_REQUEST_CANCELLED (0x10C) を返してセッションが確立できない。

## 再現手順

1. サーバーを draft-14 対応で起動し、応答 SETTINGS に以下を含める。
   - SETTINGS_WT_MAX_SESSIONS (0x14e9cd29)
   - SETTINGS_WEBTRANSPORT_MAX_SESSIONS (0xc671706a)
   - SETTINGS_WT_INITIAL_MAX_STREAMS_UNI
   - SETTINGS_WT_INITIAL_MAX_STREAMS_BIDI
   - SETTINGS_WT_INITIAL_MAX_DATA
2. Safari 26.4 から `new WebTransport(...)` で接続する。
3. Safari は CONNECT ストリームを開いた直後に H3_REQUEST_CANCELLED (0x10C) で打ち切る。

## 観察

- Safari が送る CLIENT SETTINGS には `SETTINGS_WEBTRANSPORT_MAX_SESSIONS` (draft-07) と `SETTINGS_WT_MAX_SESSIONS` (draft-14) が両方入っており、さらに `SETTINGS_WT_INITIAL_MAX_*` を送ってくる。
- 一方でサーバー側が応答 SETTINGS に `SETTINGS_WT_INITIAL_MAX_*` を含めると即座に拒否される。
- Safari は `SETTINGS_WT_ENABLED` (draft-15) を送らないため、従来のサーバー側「peer が SETTINGS_WT_ENABLED を送っている MUST」検証を通らない (nghttp3 も同じ理由で TODO コメント付きで検証を外している: `lib/nghttp3_conn.c` L62-71)。

## 根拠資料

- draft-ietf-webtrans-http3-07 / -14 / -15
- nghttp3 `lib/nghttp3_conn.c` L62-71 (peer WT 広告検証の interop 緩和)
- 実機 Safari 26.4 (Network.framework) の実測

## 解決方法

- `detect_draft_pattern` の判定順を draft-07 優先に変更 (Safari が draft-14 固有応答 SETTINGS を拒否するため、SETTINGS ネゴシエーションとしては draft-07 を採用する)。
- `DraftVersion::Draft14` のサーバー応答 SETTINGS から `WT_INITIAL_MAX_*` を除外。
- 代わりに `Settings::requires_initial_capsule_flow_control_compat()` を新設し、peer が `WT_INITIAL_MAX_*` を要求している場合に限り、セッション確立直後に `WT_MAX_STREAMS` / `WT_MAX_DATA` カプセル (draft-14 Section 5) を pending キューに積んで送出する。
- サーバー側 `is_wt_ready` と CONNECT 受理時の peer WT 広告検証を「ローカルが draft-15 を採用している時のみ要求」に緩和する (nghttp3 準拠)。
