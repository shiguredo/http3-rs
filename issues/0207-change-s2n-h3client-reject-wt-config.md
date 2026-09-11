# tokio-s2n-quic の `H3Client` が WebTransport 有効化済み `ClientConfig` を拒否しない

- Created: 2026-09-12
- Completed: {YYYY-MM-DD}
- Branch: feature/change-h3client-reject-wt-config
- Polished: {YYYY-MM-DD}

## 目的

`H3Client::connect` に WebTransport を有効化した `ClientConfig` を渡す誤用を接続確立前に `Error::InvalidState` で拒否し、`H3Server::bind` と対称な事前検証を持たせる。

## 現状

- `crates/tokio-s2n-quic/src/h3/client.rs` の `H3Client::connect` は `config.h3_settings` を `ClientConnectionState` に渡すだけで `is_webtransport_enabled()` を検査しない
- `ClientConnectionState::init_h3_streams` は SETTINGS を送るため、WebTransport 有効化済みの設定ではピアに WT 対応を広告する
- `H3Client::connect` は `set_webtransport_transport_verified` を呼ばず、`acceptor.split()` のサーバー開始 bidi アクセプターを `_bidi_acceptor` として破棄する。このためピアが開始する WT bidi ストリームを受理できず、WT セッションも確立できない (draft-ietf-webtrans-http3-16 Section 4.3)
- 0195 で `H3Server::bind` は WebTransport 有効化済み `ServerConfig` を `Error::InvalidState` で拒否するようになったが、クライアント側に対称の検証がない
- `WtClient::connect` は `set_webtransport_transport_verified(true, true)` を呼び、WT セッションを確立する経路を持つ

## 設計方針

- `H3Client::connect` の先頭 (`s2n_quic::Client` 構築より前) で `config.h3_settings.is_webtransport_enabled()` を検査し、true なら `Error::InvalidState` を返す
- `H3Client` と `H3Client::connect` の doc に「WebTransport を扱わない。WT を使う場合は `WtClient::connect` を使う」と明記する
- `ClientConfig::enable_webtransport` の doc に `H3Client` では使えない旨を追記する
- エラーメッセージは英語とし、`H3Server::bind` の拒否メッセージと対になる表現にする

## 完了条件

- WebTransport 有効化済みの `ClientConfig` を `H3Client::connect` に渡すと `Error::InvalidState` で拒否される (接続処理を開始しない)
- WebTransport 無効の `ClientConfig` は従来どおり接続できる
- 拒否を検証するテストを追加する (モック・スタブ不使用)
- `cargo test --workspace --tests` / `cargo fmt --all -- --check` / `cargo clippy --workspace --all-targets -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `crates/tokio-s2n-quic/src/h3/client.rs` (`H3Client` / `H3Client::connect` の doc と検証)
- `crates/tokio-s2n-quic/src/config.rs` (`ClientConfig::enable_webtransport` の doc)
- `crates/tokio-s2n-quic/tests/` (テスト追加)

### 一次資料

- `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 3.1 (WebTransport-capable HTTP/3 connection) / Section 4.3 (双方向ストリーム)

### 関連 issue

- 0195 (`H3Server::bind` が WebTransport 有効化済み `ServerConfig` を拒否する。本 issue はクライアント側の対称対応)
- 0208 (`WtServer::bind` / `WtClient::connect` の事前検証)
