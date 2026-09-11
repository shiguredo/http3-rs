# tokio-s2n-quic の `WtServer::bind` / `WtClient::connect` が WebTransport 無効設定を検証しない

- Created: 2026-09-12
- Completed: {YYYY-MM-DD}
- Branch: feature/change-wt-require-wt-config
- Polished: {YYYY-MM-DD}

## 目的

`WtServer::bind` / `WtClient::connect` に WebTransport 無効の `ServerConfig` / `ClientConfig` を渡す誤用を起動時に拒否し、セッション受付・確立の時点まで誤りが露見しない状態を解消する。

## 現状

- `crates/tokio-s2n-quic/src/webtransport/server.rs` の `WtServer::bind` は `config.h3_settings` をバックグラウンドの `WtSessionRequest::from_connection` に渡すだけで `is_webtransport_enabled()` を検査しない
- WebTransport 無効の `ServerConfig` でも bind は成功する。ピアが WT CONNECT を送ると sans-I/O 層の `validate_wt_connect_request_server` (`src/connection/wt_session.rs`) がローカル設定の WT 無効を検出して `StreamError(MessageError)` を返し、`WtServer::accept` に `Http3(StreamError(MessageError))` が届いて初めて誤用に気づく
- `crates/tokio-s2n-quic/src/webtransport/client.rs` の `WtClient::connect` は `config.h3_settings` を検査せず、`set_webtransport_transport_verified(true, true)` を無条件に呼ぶ。WT 無効の設定では SETTINGS で WT を広告しないまま CONNECT を送るため、ピア側の `validate_wt_connect_request_server` が拒否し (`H3_MESSAGE_ERROR`)、セッション確立時にエラーになる
- 0195 で `H3Server::bind` は WebTransport 有効化済み `ServerConfig` を拒否するようになり、「設定を事前検証する API」と「誤用がセッション確立まで露見しない API」が同居している
- `WtServer::bind` / `WtClient::connect` の doc には WebTransport 設定を有効化しておく前提が書かれていない

## 設計方針

- `WtServer::bind` の先頭 (`s2n_quic::Server` 構築より前) で `config.h3_settings.is_webtransport_enabled()` を検査し、false なら `Error::InvalidState` を返す
- `WtClient::connect` の先頭 (`s2n_quic::Client` 構築より前) で同様に検査し、false なら `Error::InvalidState` を返す
- `WtServer::bind` / `WtClient::connect` の doc に「`ServerConfig::enable_webtransport` / `ClientConfig::enable_webtransport` で有効化した設定を渡すこと」を明記する
- エラーメッセージは英語とし、`H3Server::bind` の拒否メッセージと対になる表現にする

## 完了条件

- WebTransport 無効の `ServerConfig` を `WtServer::bind` に渡すと `Error::InvalidState` で拒否される
- WebTransport 無効の `ClientConfig` を `WtClient::connect` に渡すと `Error::InvalidState` で拒否される (接続処理を開始しない)
- WebTransport 有効の設定は従来どおり bind / connect できる (既存 e2e テストが pass する)
- 拒否を検証するテストを追加する (モック・スタブ不使用)
- `cargo test --workspace --tests` / `cargo fmt --all -- --check` / `cargo clippy --workspace --all-targets -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `crates/tokio-s2n-quic/src/webtransport/server.rs` (`WtServer` / `WtServer::bind` の doc と検証)
- `crates/tokio-s2n-quic/src/webtransport/client.rs` (`WtClient` / `WtClient::connect` の doc と検証)
- `crates/tokio-s2n-quic/tests/` (テスト追加)

### 一次資料

- `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 3.1 (WebTransport-capable HTTP/3 connection / SETTINGS ネゴシエーション)

### 関連 issue

- 0195 (`H3Server::bind` が WebTransport 有効化済み `ServerConfig` を拒否する。本 issue は WtServer / WtClient 側の事前検証)
- 0207 (`H3Client` が WebTransport 有効化済み `ClientConfig` を拒否しない。クライアント側の対称対応)
