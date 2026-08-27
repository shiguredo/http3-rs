# tokio-s2n-quic の H3 リクエスト受信ループに HTTP レベルタイムアウト機構を追加する

- Created: 2026-08-27
- Completed: {YYYY-MM-DD}
- Branch: feature/add-s2n-h3-request-timeout
- Polished: {YYYY-MM-DD}

## 目的

`H3Client::send_request` / `H3ServerConnection::accept_request` のループが `peer_fin=true` かつ終了条件未達の状態で無限に spin するのを HTTP レベルタイムアウトで打ち切れるようにする。

## 現状

- 0159 の修正で受信ループは `Event::StreamEnd` (client では `finished`、server では `headers_complete && stream_ended`) を唯一の終了条件とする
- ピアが FIN を送出した後 (`peer_fin=true`)、QPACK エンコーダーストリームの更新を送らない場合、`qpack_unblock_notify` も発火せず、`finished` / `stream_ended` は永遠に true にならない
- 10ms フォールバックポーリングで `drain_events` を回し続けるが空を返すため、CPU を消費し続ける
- 現状は接続レベルタイムアウト (s2n-quic 側 idle timeout) に依存する形で暗黙に救済されるが、HTTP レベルの明示的なタイムアウトがない
- 悪意あるピア / 実装バグ耐性が低い

## 設計方針

- `ClientConfig` / `ServerConfig` に `request_timeout_ms` (デフォルト値は要検討、例: 30 秒) を追加する
- `H3Client::send_request` / `H3ServerConnection::accept_request` の受信ループに `tokio::time::sleep(request_timeout).notified()` ブランチを追加し、経過したら `Error::TransportError` 等でエラー return する
- タイムアウト後は `RESET_STREAM` を送出してピアに通知する

## 完了条件

- `ClientConfig` / `ServerConfig` に `request_timeout_ms` が追加される (`Duration` でも可)
- ピアが FIN 後にエンコーダー更新を送らないケースで、リクエスト受信ループが指定タイムアウトで打ち切られる
- 統合テストを追加する (実 QUIC で意図的にタイムアウトを発生させる)
- `cargo test --workspace --tests` / `cargo fmt --all -- --check` / `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `crates/tokio-s2n-quic/src/config.rs` (`request_timeout_ms` 追加)
- `crates/tokio-s2n-quic/src/h3/client.rs` (`H3Client::send_request` の受信ループにタイムアウト追加)
- `crates/tokio-s2n-quic/src/h3/server.rs` (`H3ServerConnection::accept_request` の受信ループにタイムアウト追加)
- `crates/tokio-s2n-quic/src/error.rs` (必要ならタイムアウト用エラー variant)

### 一次資料

- `refs/h3/rfc9114.txt` Section 5.2 (接続シャットダウン)
