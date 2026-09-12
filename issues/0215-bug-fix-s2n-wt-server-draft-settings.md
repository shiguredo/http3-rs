# tokio-s2n-quic の WtServer がクライアントの draft に一致しない SETTINGS を返し Safari から接続できない

- Created: 2026-09-12
- Completed: 2026-09-12
- Branch: feature/fix-s2n-wt-server-draft-settings
- Polished: {YYYY-MM-DD}

## 目的

`crates/tokio-s2n-quic` の `WtServer` が Safari 26.4 から接続できない問題を修正する。Chrome と Safari の両方から必ず接続できる状態にする。

## 現状

`WtSessionRequest::from_connection` は接続直後にサーバーの制御ストリームを開いて SETTINGS を送信し、その後にクライアントの制御ストリームを読む。この順序ではクライアントが選んだ draft を確定できないため、設定 (`ServerConfig::enable_webtransport` に渡した値) をそのまま広告する。

Safari 26.4 は SETTINGS で `SETTINGS_WEBTRANSPORT_MAX_SESSIONS` (draft-07) と `SETTINGS_WT_MAX_SESSIONS` / `SETTINGS_WT_INITIAL_MAX_*` (draft-13/14) を併送するハイブリッド実装であり、サーバーが返してよい WT 系応答 SETTINGS が `SETTINGS_WEBTRANSPORT_MAX_SESSIONS` 単体に限られる (`docs/SAFARI_WT.md`)。

Playwright の WebKit エンジン (Safari 相当) で実測した結果は次のとおり。

| サーバーが広告する内容 | 結果 |
|---|---|
| `SETTINGS_WT_ENABLED` (draft-16) | CONNECT を `H3_REQUEST_REJECTED` (268) で RESET される |
| `SETTINGS_WT_INITIAL_MAX_*` を含む | CONNECT を拒否される |
| `SETTINGS_WT_MAX_SESSIONS` のみ | セッションは確立するが `createBidirectionalStream()` がクレジット待ちで停止する |

一方 `examples/wt_server` の `WtSessionRequest::from_connection` はクライアントの制御ストリームを先に読んで draft を確定し、draft に一致する形状だけを返すため、`--allow-origin` を指定すれば Chromium と WebKit の両方から接続できる (実測済み)。

## 設計方針

- `WtSessionRequest::from_connection` の初期化順を「サーバーの制御ストリームを開く → クライアントの制御ストリームを読んで SETTINGS から draft を確定する → draft に一致する SETTINGS を構築して送信する」に変更する。クライアントは QUIC ハンドシェイク直後に制御ストリームを送るため、待ち合わせによるデッドロックは生じない
- draft ごとのサーバー SETTINGS の構築は `webtransport::Settings::build_server_settings` (`src/webtransport/connect/draft.rs`) に委譲し、形状の定義を一箇所に集約する
- draft を判定できない場合は設定で広告された SETTINGS をそのまま使う
- `SETTINGS_WT_INITIAL_MAX_*` は draft-14 形状の応答に含めない。初期クレジットはセッション確立直後の `WT_MAX_STREAMS` / `WT_MAX_DATA` カプセルで通知する (draft-ietf-webtrans-http3-14 Section 5)
- クライアントが `createBidirectionalStream()` で停止する問題は、draft-14 形状の応答が初期上限を含まないため、送信側がピア上限をカプセル受信まで確定できないことに起因する。サーバーのローカル設定 (カプセル生成に使う値) と SETTINGS フレームで広告する内容の分離が必要になる

## 完了条件

- WebKit (Safari 相当) から `WtServer` へ接続し、セッション確立・双方向ストリーム開設・エコー往復が成功する
- Chromium からも従来どおり接続できる
- `crates/tokio-s2n-quic/tests/` の既存 e2e テストがすべて通る
- `interop/wt` の `ngtcp2_client_s2n_server` と `s2n_client_ngtcp2_server` が通る
- `cargo test --workspace --tests` / `cargo fmt --all -- --check` / `cargo clippy --workspace --all-targets -- -D warnings` が通る

## 解決方法

ブラウザ経由の WebTransport 動作は `interop/browser` の Playwright テスト (Chromium / WebKit) でカバーする。単独 issue としては closed にする。

理由:

- 検証手段が存在しない状態では完了を確認できない。`crates/tokio-s2n-quic` の `WtServer` をブラウザで検証する仕組みは、別 issue で追加する `interop/browser` のテストが担う
- ブラウザ検証の対象は `examples/wt_server` であり、`WtServer` を対象に含めるかは `interop/browser` の整備後に判断する
- Safari が拒否する応答 SETTINGS の形状 (どの ID を返すと `H3_REQUEST_REJECTED` になるか) は `interop/browser` の設計方針に根拠として引き継いでいるため、知見は失われない

`WtServer` をブラウザで保証する必要が出た時点で、`interop/browser` のテストが落ちる形を確認したうえで改めて起票する。

### 関連ファイル

- `crates/tokio-s2n-quic/src/webtransport/server.rs` (`WtSessionRequest::from_connection` / `WtSessionRequest::accept`)
- `src/webtransport/connect/draft.rs` (`DraftVersion::build_server_settings` / `ServerSettingsParams`)
- 参考実装: `examples/wt_server/src/webtransport.rs` の `WtSessionRequest::from_connection`
- 検証手段: `interop/browser` (Chromium / WebKit の Playwright テスト)
