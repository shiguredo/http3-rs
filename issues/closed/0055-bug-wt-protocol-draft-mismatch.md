# WebTransport `:protocol` と SETTINGS で決まる draft の整合が検証されていない

Created: 2026-04-07
Completed: 2026-04-07
Model: Opus 4.6

## 概要

WebTransport CONNECT の判定 `is_webtransport_connect` / `is_webtransport_connect_decoded` は `:protocol` が `webtransport-h3` または `webtransport` のどちらでも CONNECT として通している。`Connection::send_request` および受信側 `emit_header_events` も「相手 SETTINGS から決まる draft に対して、この `:protocol` が正しいか」を検証していない。

`DraftVersion::protocol_value()` は以下の対応を定義している:

- `Draft02` / `Draft07` / `Draft14` → `webtransport`
- `Draft15` → `webtransport-h3`

しかし送受信時にこの対応がチェックされないため、以下が成立してしまう:

1. draft-15 を SETTINGS でネゴシエートした接続に対して、旧値 `webtransport` を持つ CONNECT が通る
2. draft-02/07/14 を SETTINGS でネゴシエートした接続に対して、新値 `webtransport-h3` を持つ CONNECT が通る

draft-ietf-webtrans-http3-15 Section 3.2 / 7.1 の version identification 要件を満たしていない。

## 該当箇所

- `src/connection/mod.rs` `is_webtransport_connect` (現在 L3707 付近)
- `src/connection/mod.rs` `is_webtransport_connect_decoded` (現在 L3720 付近)
- `src/connection/mod.rs` `Connection::send_request` の WT 分岐 (現在 L3177 付近)
- `src/connection/mod.rs` `Connection::emit_header_events` のサーバー WT 受信分岐 (現在 L2628 付近)
- `src/webtransport/connect.rs` `DraftVersion::protocol_value` (現在 L82 付近)

## 根拠

- draft-ietf-webtrans-http3-15 Section 3.2: `:protocol` の値は `webtransport-h3`
- draft-ietf-webtrans-http3-15 Section 7.1: SETTINGS と `:protocol` でドラフト識別を行う
- draft-ietf-webtrans-http3-02 Section 3.2: 旧値 `webtransport`
- `refs/webtrans/draft-ietf-webtrans-http3-15.txt`

## 修正方針

破壊的変更を許容するが、Chrome / Safari interop を壊さないため、不一致時の挙動は段階的に分岐する。

1. クライアント送信側 (`send_request` の WT 分岐):
   - peer SETTINGS から決定される `DraftVersion` を取得し、`headers` の `:protocol` がそのドラフトの `protocol_value()` と一致するかを検証する。
   - 不一致の場合は `Error::ConnectionError(ErrorCode::InternalError)` を返してアプリ側のバグとして拒否する。アプリは `DraftVersion::protocol_value()` を使ってヘッダーを組むべき。
2. サーバー受信側 (`emit_header_events` の WT 分岐):
   - ローカル SETTINGS および peer SETTINGS から決定される `DraftVersion` を取得し、受信した `:protocol` がそのドラフトの値と一致するか検証する。
   - 不一致の場合は `Error::StreamError(ErrorCode::MessageError)` で当該 CONNECT を拒否する (接続全体は閉じない)。
3. `is_webtransport_connect` / `is_webtransport_connect_decoded` 自体は「`:method=CONNECT` かつ `:protocol` がいずれかの WT 値」のままでよい。draft 整合判定は呼び出し側の WT 分岐で行う。これにより、非 WT の CONNECT 経路は影響を受けない。
4. draft-14 と draft-07 は `:protocol` 値が同じ `webtransport` なので、SETTINGS パターンによる draft 判定との組み合わせで矛盾しない。
5. `CHANGES.md` に `[CHANGE]` (拒否側) と `[FIX]` (整合検証追加) として記載する。
6. テストで以下を追加する:
   - draft-15 接続で `:protocol = webtransport` を送ったクライアントの `send_request` がエラーになる
   - draft-15 接続でサーバーが `:protocol = webtransport` の CONNECT を受信した場合に MessageError でストリームを拒否する
   - draft-07/14 接続で `:protocol = webtransport-h3` の CONNECT を受信した場合に MessageError でストリームを拒否する
   - 既存の draft 別 happy path (`:protocol` 一致) が引き続き通る

## 補足

draft-14 以前の peer `wt_enabled` を緩めている既存判断 (Safari interop) はこの issue では変更しない。`:protocol` の整合は SETTINGS から確定的に決まる draft 判定に基づくため、Safari interop を壊さずに導入できる。

## 解決方法

- `src/connection/mod.rs` `Connection::send_request` の WebTransport CONNECT 分岐で、`peer_wt_draft_version()` から決まる `DraftVersion::protocol_value()` とリクエストヘッダーの `:protocol` を比較し、不一致時に `Error::ConnectionError(ErrorCode::InternalError)` を返すようにした。
- `src/connection/mod.rs` `Connection::emit_header_events` のサーバー WebTransport CONNECT 分岐で同様に、`peer_wt_draft_version()` から決まる draft の `protocol_value()` と受信ヘッダーの `:protocol` を比較し、不一致時に `Error::StreamError(ErrorCode::MessageError)` で当該 CONNECT を拒否するようにした (接続全体は閉じない)。
- `is_webtransport_connect` / `is_webtransport_connect_decoded` 自体は両値を許容するままとし、draft 整合性は WT 分岐内でのみ判定する。これにより plain CONNECT 経路は影響を受けない。
- `tests/test_webtransport_draft_connect.rs` に `protocol_draft_alignment` モジュールを追加し、以下を検証する:
  - `server_rejects_legacy_protocol_on_draft15` (draft-15 の peer に `webtransport` を送ると拒否)
  - `server_rejects_new_protocol_on_draft07` (draft-07 の peer に `webtransport-h3` を送ると拒否)
  - `server_rejects_new_protocol_on_draft14` (draft-14 の peer に `webtransport-h3` を送ると拒否)
  - `client_send_request_rejects_mismatched_protocol_on_draft15` (クライアント送信側の不一致拒否)
- `CHANGES.md` の `## develop` に `[FIX]` として記載した。
