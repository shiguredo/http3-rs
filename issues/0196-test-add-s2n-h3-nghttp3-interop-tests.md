# tokio-s2n-quic の H3 で動的テーブル参照ピア (nghttp3 等) との interop テストを追加する

- Created: 2026-08-27
- Completed: {YYYY-MM-DD}
- Branch: feature/add-s2n-h3-nghttp3-interop-tests
- Polished: {YYYY-MM-DD}

## 目的

QPACK 動的テーブルを使用するピア (nghttp3 等) と `tokio-s2n-quic` の H3 実装の interop テストを追加し、0159 で修正した QPACK ブロック解除経路が動的テーブル参照時にも動作することを検証する。

## 現状

- 0159 の修正で uni タスクを `feed_stream_only` + `Notify` 通知方式に変更し、QPACK ブロック解除で生成されたヘッダー・ボディ・StreamEnd が失われないようにした
- しかし、tokio-s2n-quic 自身の QPACK エンコーダー (`src/qpack/encoder.rs`) は動的テーブルへ挿入しないため、s2n↔s2n のループバックではこのバグは再現できず、interop テストがない
- 動的テーブル参照を行うピア (nghttp3 等) との接続でのみバグが再現する
- 0159 の issue で「テストは 0165 (interop テストの空振り修正) の後に追加する」と明記された

## 設計方針

- `interop_h3` クレートに nghttp3 (`crates/nghttp3-rs` or `tokio-nghttp3`) を使ったクライアント / サーバーテストを追加する
- 既存の `interop_h3` テストパターン (`quinn_client_s2n_server` 等) を参考にする
- 動的テーブル使用時のレスポンス欠落を検出するため、複数リクエストの連続送信で完全なレスポンスが得られることを検証する
- 0165 (interop テストの空振り修正) の後に着手する

## 完了条件

- `interop_h3` に nghttp3 との interop テストが追加される (`nghttp3_client_s2n_server` / `s2n_client_nghttp3_server` 等)
- 動的テーブル参照時のレスポンス欠落が起きないこと (0159 修正の回帰防止) を検証する
- `cargo test --workspace --tests` / `cargo fmt --all -- --check` / `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `crates/interop_h3/tests/` (新規テストファイル)
- 参考: `crates/interop_h3/tests/quinn_client_s2n_server.rs` (既存の interop テスト)
- 参考: `crates/nghttp3-rs/` (nghttp3 バインディング)

### 依存 issue

- 0165 (interop テストの空振り修正) が完了してから着手する

### 一次資料

- `refs/h3/rfc9204.txt` Section 3 (Wire Format / 動的テーブル)
