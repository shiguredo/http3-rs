# tokio-ngtcp2 サーバーにアドレス検証とリソース消費の上限を追加する

- Created: 2026-08-13
- Completed: {YYYY-MM-DD}
- Branch: feature/add-server-dos-mitigations
- Polished: {YYYY-MM-DD}

## 目的

アドレス検証前のサーバーのリソース消費を攻撃から守る。現在は有効な (正しく復号できる) Initial を 1 個送るだけで TLS セッション・QUIC 接続・HTTP/3 接続の状態が作成され、デフォルトの `max_idle_timeout` (30 秒) の間保持されるため、DCID を変えながら Initial を送り続けるとサーバーのメモリを枯渇させられる。

## 現状

- `Server` / `ServerWebTransportSession` に接続数上限・レート制限・アドレス検証のいずれも無い
- `Connection::server_new` は `settings.tokenlen` を設定しておらず、ngtcp2 は `read_pkt` で NGTCP2_ERR_RETRY を返すケースがあるものの (クライアントの ClientHello が 1 つの Initial に収まらない場合など)、`Error::classify_connection_error` が RETRY を SilentDrop に分類して黙って破棄している (Retry パケットを送らない設計としてコメントで明記済み)
- 新規接続パケットのパース (`crates/tokio-ngtcp2/src/conn.rs` の `parse_new_connection_packet`) は状態を作らない不正パケットを弾くが、有効な Initial に対する制限は無い

## 設計方針

- **Retry によるアドレス検証 (RFC 9000 Section 8.1.2)**: `read_pkt` が NGTCP2_ERR_RETRY を返したら `ngtcp2_crypto_write_retry` (bindings.rs に定義済み) で Retry パケットを送り、接続状態は破棄する。トークンは乱数で生成する (RFC 9000 Section 8.1.1)。検証側は `settings.tokenlen` を設定する形とし、トークン不一致の Initial は ngtcp2 が DROP_CONN を返すことを利用する。`classify_connection_error` の RETRY 分類は SilentDrop から見直す (将来 Retry 送信を実装する場合の注記が error.rs の doc に既にある)
- **接続数上限**: `Server` / `ServerWebTransportSession` に最大接続数の設定を追加する (bind 引数またはビルダー)。`connections` のサイズが上限に達した場合は新規 Initial を状態を作らずに破棄する (RFC 9000 Section 5.2.2 の MUST drop に合致)
- **新規接続のレート制限**: 新規接続の作成レートを制限する (例: トークンバケット)。超過分は状態を作らずに破棄する。既存接続のパケット処理は制限しない
- サーバーループは既存どおり継続させる (制限超過でサーバーを停止しない)
- クライアント側は変更しない

## 完了条件

- Retry パケットが送信され、トークン付き Initial でハンドシェイクが継続できる
- 接続数上限を超えた新規 Initial が破棄され、既存接続は影響を受けない
- レート制限超過時の新規 Initial が破棄され、サーバーは継続する
- テストが追加される (Retry 経路、接続数上限、レート制限のそれぞれでサーバーが継続すること)
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `crates/tokio-ngtcp2/src/server.rs` / `crates/tokio-ngtcp2/src/webtransport.rs` (`handle_new_connection`、Retry 送信、接続数上限、レート制限)
- `crates/ngtcp2-rs/src/error.rs` (`classify_connection_error` の RETRY 分類)
- `crates/ngtcp2-rs/src/conn.rs` (`server_new` の settings、token 検証のための設定)
- `crates/ngtcp2-sys/src/bindings.rs` (`ngtcp2_crypto_write_retry` / `ngtcp2_pkt_write_retry` の doc)
- 一次資料: `refs/quic/rfc9000.txt` Section 8.1 (Address Validation)、Section 8.1.1 (Token Construction)、Section 8.1.2 (Retry)、Section 5.2.2 (Server Packet Handling)
