# tokio-s2n-quic の H3 で動的テーブル参照ピア (nghttp3) との interop テストを追加する

- Created: 2026-08-27
- Completed: {YYYY-MM-DD}
- Branch: feature/add-s2n-h3-nghttp3-interop-tests
- Polished: {YYYY-MM-DD}

## 目的

QPACK 動的テーブルを使用するピア (nghttp3) と `tokio-s2n-quic` の H3 実装の interop テストを追加し、0159 で修正した QPACK ブロック解除経路が動的テーブル参照時にも動作することを検証する。

## 現状

- 0159 の修正で uni タスクを `feed_stream_only` + `Notify` 通知方式に変更し、QPACK ブロック解除で生成されたヘッダー・ボディ・StreamEnd が失われないようにした
- しかし、tokio-s2n-quic 自身の QPACK エンコーダー (`crates/tokio-s2n-quic/src/qpack/encoder.rs`) は動的テーブルへ挿入しないため、s2n↔s2n のループバックではこのバグは再現できず、interop テストがない
- 動的テーブル参照を行うピア (nghttp3 等) との接続でのみバグが再現する
- ngtcp2 系の crates.io 移行でローカルの `crates/nghttp3-sys` / `crates/ngtcp2-rs` が削除され、ngtcp2 側ピアは shiguredo_http3 を使うようになったため、独立した HTTP/3 実装ピアが存在しない
- nghttp3 のバインディングは ngtcp2-rs で新規作成が必要 (`shiguredo_nghttp3_sys` / `shiguredo_nghttp3`) で、ACK 済みストリームデータの公開も前提になる

## 設計方針

- ngtcp2-rs に nghttp3 のバインディングと ACK 済みストリームデータの公開が実装されたら、interop の ngtcp2 系ピア (現在 shiguredo_http3) を nghttp3 ベースに差し替える
- `interop/h3/tests/` に既存パターン (`quinn_client_s2n_server` 等) に合わせて `nghttp3_client_s2n_server` / `s2n_client_nghttp3_server` を追加する
- 動的テーブル使用時のレスポンス欠落を検出するため、複数リクエストの連続送信で完全なレスポンスが得られることを検証する
- 0165 (interop テストの空振り修正) は完了済み

## 完了条件

- `interop/h3` に nghttp3 との interop テストが追加される (`nghttp3_client_s2n_server` / `s2n_client_nghttp3_server` 等)
- 動的テーブル参照時のレスポンス欠落が起きないこと (0159 修正の回帰防止) を検証する
- `cargo test --workspace --tests` / `cargo fmt --all -- --check` / `cargo clippy --all-targets --all-features -- -D warnings` が通る

## pending にした理由

nghttp3 のバインディング (`shiguredo_nghttp3_sys` / `shiguredo_nghttp3`) と ACK 済みストリームデータの公開が ngtcp2-rs に存在せず、本リポジトリの interop に nghttp3 ピアを追加できないため。ngtcp2-rs の issue 0001 / 0002 / 0003 が完了したら reopened にして対応する。

## 解決方法

### 関連ファイル

- `interop/h3/tests/` (新規テストファイル)
- `crates/tokio-ngtcp2/` (ngtcp2 系ピアのドライバ)
- `crates/tokio-s2n-quic/src/qpack/encoder.rs` (動的テーブルを使わないエンコーダー)
- ngtcp2-rs の `nghttp3-sys` / `nghttp3` (新規)
- 一次資料: `refs/h3/rfc9204.txt` Section 3 (Wire Format / 動的テーブル)

### 依存 issue

- ngtcp2-rs の issue 0001 (ACK 済みストリームデータの公開) / 0002 (nghttp3-sys) / 0003 (nghttp3 ラッパー)
- 0165 (interop テストの空振り修正) は完了済み
