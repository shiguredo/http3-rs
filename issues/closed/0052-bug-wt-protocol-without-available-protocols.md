# WT-Available-Protocols 未送時の WT-Protocol レスポンスを違反扱いしていない

Created: 2026-04-07
Completed: 2026-04-07
Model: Opus 4.6

## 概要

draft-ietf-webtrans-http3-15 Section 3.3 では、`WT-Protocol` レスポンスヘッダはクライアントが `WT-Available-Protocols` を送ったときに限り server が MAY で返してよいものであり、server が返す値は「the client's list」から選ばれた single choice でなければならない (MUST) と規定されている。クライアントが `WT-Available-Protocols` を送っていない場合、server には `WT-Protocol` を返す根拠が無い。

現在の実装は、この「リスト未送信時の `WT-Protocol`」を検証していない。

- クライアント側: `Connection::recv_headers` の WebTransport 2xx 処理 (現在 L2722-2749) は `!session.available_protocols.is_empty()` で gate しており、空のとき (= `WT-Available-Protocols` を送っていないとき) は `WT-Protocol` の有無も値も検証せず、そのまま Established に遷移する。
- サーバー側: `Connection::send_response` の検証ブロック (現在 L3252-3280) も同じ条件で gate しており、リスト未受信のときに自分が `WT-Protocol` を付けて 2xx を返してもエラーにならない。

## 該当箇所

- `src/connection/mod.rs` `Connection::recv_headers` (WebTransport クライアント側 2xx 処理、現在 L2722-2749)
- `src/connection/mod.rs` `Connection::send_response` (WebTransport サーバー側 2xx 検証、現在 L3252-3280)

## 根拠

draft-ietf-webtrans-http3-15 Section 3.3 (L538-545):

> The client MAY include a WT-Available-Protocols header field in the CONNECT request. ... If the server receives such a header, it MAY include a WT-Protocol field in a successful (2xx) response. If it does, the server MUST include a single choice from the client's list in that field.

クライアントが `WT-Available-Protocols` を送っていない場合、"the client's list" は存在しない。したがって server が `WT-Protocol` を返す状況自体が仕様の前提を満たさず、整合性として違反扱いするのが妥当である。

## 修正方針案

- クライアント側: `recv_headers` の WebTransport 2xx 処理で、`session.available_protocols.is_empty()` のときに限り「`WT-Protocol` ヘッダが存在しないこと」を検証する。存在した場合は protocol violation として扱い、セッションを終了する。エラーコードは `WT_ALPN_ERROR` ではなく一般的な protocol violation 系 (要検討) を割り当てる。
- サーバー側: `send_response` で `session.available_protocols.is_empty()` のときに `WT-Protocol` を含む 2xx を送ろうとした場合は内部エラーで拒否する (アプリケーション実装ミスを早期に検出する)。
- テスト: 単体テストで「クライアントが `WT-Available-Protocols` を送らずに 2xx + `WT-Protocol` を受信」「server が空リストに対して `WT-Protocol` 付き 2xx を送ろうとする」両方をエラーとして検証する。

## 注意点

- 仕様は明示的な MUST 文言ではないため優先度は高くない (P2 相当)。ただし spec の整合性の観点で正当な指摘である。
- エラーコードの割り当ては draft-15 の WebTransport エラーコード一覧を参照のうえ決定する。`WT_ALPN_ERROR` は「クライアントが application protocol negotiation を必須とした場合」のためのコードなので、本件には流用しない方針も検討する。

## 解決方法

draft-ietf-webtrans-http3-15 Section 3.3 の規定 (server が `WT-Protocol` を返してよいのは client が `WT-Available-Protocols` を送ったときに限る) を実装に反映した。

- `Connection::recv_headers()` のクライアント側 WebTransport 2xx 処理から `!session.available_protocols.is_empty()` の gate を外し、`available_protocols` が空のときも `wt-protocol` ヘッダの有無を検証するようにした。`available_protocols` が空かつ `wt-protocol` を含む場合は違反として扱い、既存の `terminate_wt_session_with(WtErrorCode::AlpnError)` 経路でセッションを閉じる (エラーコードは現状のセッション終了経路と整合させるため `WT_ALPN_ERROR` を流用している)。
- `Connection::send_response()` のサーバー側検証ブロックでも同じ gate を外し、`available_protocols` が空のときに `wt-protocol` を含む 2xx を送ろうとした場合は `Error::ConnectionError(ErrorCode::InternalError)` で拒否するようにした (アプリケーション実装ミスを早期検出する意図)。
- `tests/test_webtransport_draft_connect.rs` に `wt_protocol_without_available_protocols` モジュールを追加し、サーバー側 `send_response` の拒否とクライアント側受信時の `WebTransportSessionClosed` 発火を検証する単体テストを追加した。
