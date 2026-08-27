# tokio-s2n-quic の `H3Server` に WT 有効化 `ServerConfig` を渡し、ピアが WT bidi ストリームを開くとハングする

- Created: 2026-08-27
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-s2n-h3-webtransport-bidi-hang
- Polished: {YYYY-MM-DD}

## 目的

`H3Server::bind` に `enable_webtransport` された `ServerConfig` を渡すと、ピアが WebTransport bidi ストリーム (`0x41` 始まり) を開いた場合に `accept_request` が無限にハングする問題を修正する。

## 現状

- `crates/tokio-s2n-quic/src/h3/server.rs` の `H3ServerConnection::accept_request` は sans-I/O 層の `Event::WebTransport(BidiStreamOpen)` 等を処理しない (`_ => {}` で捨てる)
- `ServerConfig::enable_webtransport(_)` を渡して `H3Server::bind` した場合、`is_wt_fully_negotiated()` が true になる
- この状態でピアが `0x41` 始まりの bidi ストリームを開くと `dispatch_client_bidi_stream` (`src/connection/mod.rs`) が WT bidi として扱い、`Event::WebTransport(BidiStreamOpen)` 等のみを発火する
- `accept_request` の match ガード (`if sid == stream_id`) はこれらを `_ => {}` で捨て、`Event::HeadersEnd` / `Event::StreamEnd` は永久に来ない
- 旧コード (`if headers_complete || fin { break; }`) はピア FIN で phase 1 を break し、空の `H3Request` を返して (壊れているが) 制御を返した
- 0159 で導入した新コード (`while !(headers_complete && stream_ended)`) はループ内で `select!` が `receive` ブランチを `peer_fin=true` で無効化した後、`notified` / 10 ms タイマーで無限に spin する
- 失敗モードが「壊れた戻り値」から「100 Hz spin ハング」に悪化

## 設計方針

- `H3Server` は WebTransport を扱わない設計であることを doc に明示する
- `H3Server::bind` に `enable_webtransport` された `ServerConfig` を渡した場合、ビルド時ではなく実行時に警告するか、`Error::InvalidState` で拒否する (どちらにするかは実装時に判断)
- 併せて `WtServer::bind` を使うべきであることを doc に案内する

## 完了条件

- `H3Server::bind` に `enable_webtransport` された `ServerConfig` を渡すと明確なエラーで拒否される、または doc で「H3Server は WebTransport を扱わない」ことが明示される
- 統合テストを追加する (WT 有効化 ServerConfig を H3Server::bind に渡してエラーになることを検証)
- `cargo test --workspace --tests` / `cargo fmt --all -- --check` / `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `crates/tokio-s2n-quic/src/h3/server.rs` (`H3Server::bind` / doc)
- `crates/tokio-s2n-quic/src/config.rs` (`ServerConfig` の doc)

### 一次資料

- `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 4.2 (WT bidi ストリーム、`0x41` 始まり)
