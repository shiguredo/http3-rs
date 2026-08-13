# サポート外バージョンの Initial に Version Negotiation パケットを返す

- Created: 2026-08-13
- Completed: {YYYY-MM-DD}
- Branch: feature/add-version-negotiation
- Polished: {YYYY-MM-DD}

## 目的

サポート外の QUIC バージョンで接続してきたクライアントに、サポートしているバージョンを通知する。現在は黙って破棄するため、バージョン不一致のクライアントはタイムアウトまで原因を知ることができない。

## 現状

- `Server` / `ServerWebTransportSession` の `handle_new_connection` は、`parse_new_connection_packet` で取り出したバージョンが `QuicVersion::V1` と異なる場合、接続状態を作らずに破棄する (コメントで「RFC 9000 Section 5.2.2 の Version Negotiation パケット送信は未実装」と明記済み)
- RFC 9000 Section 5.2.2 は「パケットがサポート済みバージョンの新規接続を開始できる大きさなら、サーバーは SHOULD で Version Negotiation パケットを送る」と定める (応答数を制限する MAY もある)
- `ngtcp2_sys::ngtcp2_pkt_write_version_negotiation` は bindings.rs に定義済み
- `QuicVersion::V2` (RFC 9369) は定数として定義済みだが、`Connection::server_new` は V1 固定

## 設計方針

- `handle_new_connection` でバージョンが V1 と異なる場合、`ngtcp2_pkt_write_version_negotiation` で Version Negotiation パケット (Long header, version = 0, DCID = 受信パケットの SCID, SCID = 受信パケットの DCID) を送る (RFC 9000 Section 6.1)。接続状態は作らない
- パケットが小さすぎて新規接続を開始できない場合は MUST drop のまま破棄する (RFC 9000 Section 5.2.2)
- 応答数を制限する (RFC 9000 Section 5.2.2 の MAY)。過剰なバージョン不一致パケットで送信コストを強制させない
- サーバーループは継続させる
- 対応バージョン一覧は `QuicVersion` から導出し、将来 V2 に対応した際に自動で反映される形にする

## 完了条件

- サポート外バージョンの Initial を受信すると Version Negotiation パケットが返る (DCID / SCID が RFC 9000 Section 6.1 のとおり)
- 小さすぎるパケットには応答せず破棄する
- 応答数制限を超えた場合は破棄し、サーバーは継続する
- テストが追加される (既存の `test_server_survives_malformed_packets` の「サポート外の QUIC バージョンの Initial」を VN 応答の検証に拡張する)
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `crates/tokio-ngtcp2/src/server.rs` / `crates/tokio-ngtcp2/src/webtransport.rs` (`handle_new_connection` のバージョン判定と VN 送信)
- `crates/ngtcp2-sys/src/bindings.rs` (`ngtcp2_pkt_write_version_negotiation` の doc)
- `crates/ngtcp2-rs/src/types.rs` (`QuicVersion` enum)
- 一次資料: `refs/quic/rfc9000.txt` Section 5.2.2 (Server Packet Handling)、Section 6.1 (Sending Version Negotiation Packets)、Section 17.2.1 (Version Negotiation Packet)
