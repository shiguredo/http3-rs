# 未登録のローカル開始 uni ストリームへのデータがピア開始ストリームとして誤処理される

- Created: 2026-09-12
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-unregistered-local-uni-guard
- Polished: {YYYY-MM-DD}

## 目的

未登録のローカル開始 uni ストリームにデータ / FIN が sans-I/O に到達した場合の防御を `handle_unidirectional_stream` の入口に追加し、ピア開始ストリーム (制御 / QPACK / 新規 WT 等) として誤処理されないようにする。

## 現状

- `src/connection/mod.rs` の `handle_unidirectional_stream` はストリーム ID の種別を判定せず、既知ストリーム (`control_recv` / `peer_encoder_stream_id` / `peer_decoder_stream_id` / `wt_uni_streams` / `pending_wt_uni_streams`) に一致しなければ `handle_new_unidirectional_stream` に委譲する
- ローカル開始 uni (ローカルにとって送信専用) にピアからデータが到達することは RFC 9000 Section 19.8 の STREAM_STATE_ERROR に相当する不正入力であり、通常は QUIC 層で拒否される
- しかし統合層の不具合や将来の QUIC 実装変更で到達した場合、例えばサーバーで `feed_stream(7, &[0x02], false)` を実行すると `peer_encoder_stream_id` に 7 が登録され、QPACK エンコーダーストリームとして誤処理される (0209 のレビューで実測確認)
- 0209 で登録済みローカル開始 uni の STREAM / FIN は防御したが、未登録経路は残っている

## 設計方針

- `handle_unidirectional_stream` の入口 (無視対象チェックの後) で `is_local_initiated_uni(kind)` を判定し、該当する場合は `Ok(())` で静かに吸収する
- 制御 / QPACK ストリームを含むすべてのローカル開始 uni が対象 (受信データは常に不正)
- 0209 の登録済みガードは残し、多層防御とする (入口ガードで到達しなくなるが、関数単体の防御として維持する)

## 完了条件

- 未登録のローカル開始 uni にデータ / FIN を feed しても、既知ストリームへの誤登録・イベント発火・データ FC 計上が起きない
- テストを追加する (データ / FIN の両方。モック・スタブ不使用)
- ピア開始 uni の処理は従来どおり
- `cargo test --workspace --tests` / `cargo fmt --all -- --check` / `cargo clippy --workspace --all-targets -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/connection/mod.rs` (`handle_unidirectional_stream`)
- `src/connection/mod.rs` (テスト追加)

### 一次資料

- `refs/quic/rfc9000.txt` Section 19.8 (STREAM_STATE_ERROR) / Section 2.1 (ストリーム種別)

### 関連 issue

- 0209 (登録済みローカル開始 uni の STREAM / FIN 防御。本 issue は未登録経路の残存対応)
