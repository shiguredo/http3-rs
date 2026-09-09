# tokio-s2n-quic の `H3Server` がピアの WebTransport bidi ストリーム (`0x41`) でハングする

- Created: 2026-08-27
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-s2n-h3-webtransport-bidi-hang
- Polished: 2026-09-09

## 目的

`H3Server` は WebTransport を扱わない設計だが、ピアが `0x41` 始まりの WebTransport bidi ストリームを開くと `H3ServerConnection::accept_request` が WT イベントを捨て、終了条件 (`headers_complete && stream_ended`) が満たされず 100 Hz spin で無限にハングする問題を修正する。あわせて、`H3Server::bind` に `enable_webtransport` された `ServerConfig` を渡す誤用を `Error::InvalidState` で拒否する。

## 現状

- `crates/tokio-s2n-quic/src/h3/server.rs` の `H3ServerConnection::accept_request` は `Event::Header` / `HeadersEnd` / `Data` / `StreamEnd` 以外を `_ => {}` で捨てる
- `H3ServerConnection::new` は `set_webtransport_transport_verified` を呼ばないため、`is_wt_fully_negotiated()` は常に false になる (`src/connection/wt_session.rs`。WT 経路の `WtSessionRequest::from_connection` や `WtClient::connect` は呼ぶが、H3Server 経路では呼ばれない)
- この状態でピアが `0x41` 始まりの bidi ストリームを開くと `dispatch_client_bidi_stream` (`src/connection/mod.rs`) は `handle_wt_bidi_stream` に到達せず、SETTINGS 受信済みなら `Event::WebTransport(BufferedStreamRejected)` を発火し、未受信なら `pending_wt_bidi_pre_negotiation` に保留する。`BidiStreamOpen` はセッション確立後のみで、`H3Server` では発火しない
- `accept_request` の match は `BufferedStreamRejected` を `_ => {}` で捨て、`Event::HeadersEnd` / `Event::StreamEnd` は永久に来ない
- 旧コード (`if headers_complete || fin { break; }`) はピア FIN で phase 1 を break し、空の `H3Request` を返して (壊れているが) 制御を返した
- 0159 で導入した新コード (`while !(headers_complete && stream_ended)`) はループ内で `select!` が `receive` ブランチを `peer_fin=true` で無効化した後、`notified` / 10 ms タイマーで無限に spin する
- 失敗モードが「壊れた戻り値」から「100 Hz spin ハング」に悪化
- このハングは `enable_webtransport` の有無に関わらず、ピアが `0x41` bidi を開けば発生する

## 設計方針

- `H3ServerConnection::accept_request` の match に `Event::WebTransport(BufferedStreamRejected { stream_id: sid, .. }) if sid == stream_id` (および `BidiStreamOpen` 等の WT イベント) のアームを追加し、`Error::InvalidState` で即 return する。必要なら該当ストリームを reset する
- `H3Server::bind` の先頭で `config.h3_settings.is_webtransport_enabled()` を検査し、WT 有効なら `Error::InvalidState` を返して拒否する
- `H3Server` / `H3Server::bind` の doc に「WebTransport を扱わない。WT を使う場合は `WtServer::bind` を使う」と明示する
- 0193 (`add-s2n-h3-request-timeout`) はリクエストタイムアウト、0194 (`add-s2n-h3-concurrent-requests`) は `accept_request` の並行リクエスト対応を担う。本 issue は「`0x41` bidi をリクエストとして誤受信した場合に即エラー終了する」ことに限定し、役割を重複させない

## 完了条件

- `H3ServerConnection::accept_request` がピアの `0x41` bidi ストリームを受信した場合、ハングせず `Error::InvalidState` で return する
- `H3Server::bind` に `enable_webtransport` された `ServerConfig` を渡すと `Error::InvalidState` で拒否される
- 統合テストを追加する (実 QUIC 接続でピアが `0x41` bidi を開いても `accept_request` がタイムアウトせずエラー終了すること、WT 有効化 `ServerConfig` が拒否されることを検証。モック・スタブは使わない)
- `cargo test --workspace --tests` / `cargo fmt --all -- --check` / `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `crates/tokio-s2n-quic/src/h3/server.rs` (`H3Server::bind` / `H3ServerConnection::accept_request` / doc)
- `crates/tokio-s2n-quic/src/config.rs` (`ServerConfig` の doc)
- `crates/tokio-s2n-quic/src/error.rs` (`Error::InvalidState` は既存)
- `crates/tokio-s2n-quic/tests/` (H3 e2e テスト。新規追加)

### 一次資料

- `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 4.3 (WT bidi ストリーム、`0x41` 始まり)
