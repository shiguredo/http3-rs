# tokio-s2n-quic の送信経路を FIN 交付ループに追従させる

- Created: 2026-08-08
- Completed: 2026-08-08
- Branch: feature/refactor-s2n-fin-delivery
- Polished: {YYYY-MM-DD}

## 目的

sans-I/O 層の FIN 交付仕様 (FIN はデータ全消費後の追加呼び出しで `(空, fin=true)` として交付される) に tokio-s2n-quic の送信経路を追従させる。現状の 1 回呼び出しのままだと H3 層に FIN 未交付状態が残留し、`writable_streams` に報告され続ける状態が恒久化する。

## 現状

- `crates/tokio-s2n-quic/src/h3/client.rs` のリクエスト送信経路: `get_stream_data` を 1 回呼び、fin 要素を捨てている
- `crates/tokio-s2n-quic/src/h3/server.rs` のレスポンス送信経路: 同様
- `crates/tokio-s2n-quic/src/webtransport/server.rs` の `accept` / `reject`: 同様
- いずれも QUIC 層の `send_stream.finish()` で送信方向クローズを代替しており、ワイヤ上は正しく動く。ただし H3 層の `SendBuffer` に FIN 未交付状態が残り、当該ストリームが `writable_streams` に報告され続ける
- 本リポジトリに `writable_streams` の本番コンシューマは現状存在しないため実害はないが、イベントループベースの送信ループを導入した時点で想定外の `(空, fin=true)` 交付や無限報告が発生する火種になる

## 設計方針

- `take_stream_data` (または `get_stream_data` + `consume_stream_data`) を `None` までループし、`fin=true` を受領したら QUIC 層で `finish()` を呼ぶ形に更新する
- `crates/tokio-s2n-quic/src/internal/connection_state.rs` の `drain_qpack_data` は制御 / QPACK ストリームが対象で FIN を持たないため変更不要
- 公開 API (クライアント / サーバーの送信メソッドのシグネチャ) は変えない

## 完了条件

- 各送信経路 (リクエスト / レスポンス / WebTransport accept / reject) で FIN 交付までループし、H3 層に FIN 未交付状態が残留しない
- tokio-s2n-quic の既存テストが通る

## 解決方法

(実装時に追記)

## 備考

0139 (Connection のストリーム / WT セッションが無制限に蓄積する) の完了条件に統合層の FIN ドレイン対応が必須として含まれており、本 issue の内容と完全に重複するため、0139 に吸収して closed にする。0139 の実装ブランチ (feature/fix-connection-resource-leak) で対応する。
