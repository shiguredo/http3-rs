# init_h3_streams にストリーム ID の重複検証がない

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-init-h3-streams-duplicate-id
- Polished: {YYYY-MM-DD}

## 目的

ストリーム ID の重複設定による無警告のデータ喪失を防ぐ。

## 現状

- `src/connection/mod.rs` の `Connection::init_h3_streams` は `set_control_stream_id` → `set_encoder_stream_id` → `set_decoder_stream_id` を順に呼ぶが、各 setter は「自分のフィールドが None か」しか検査しない
- 例: `set_control_stream_id(2)` の後に `set_encoder_stream_id(2)` を呼ぶと両方成功する
- 同一 QUIC ストリーム ID に 2 つの stream type が割り当てられ、`Connection::get_stream_data` は control を優先するためエンコーダー / デコーダーの初期データが静かに消失する (エラーも返らない)
- RFC 9114 Section 6.2 の「ストリームタイプはストリームの開始に 1 つ」の前提が壊れる

## 設計方針

- `set_control_stream_id` / `set_encoder_stream_id` / `set_decoder_stream_id` (または `init_h3_streams` 内) で ID の重複を検証し、重複時はエラーを返す

## 完了条件

- 重複するストリーム ID の設定がエラーになる
- テストが追加される
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/connection/mod.rs` (`Connection::init_h3_streams` / `set_control_stream_id` / `set_encoder_stream_id` / `set_decoder_stream_id`)
- 一次資料: `refs/h3/rfc9114.txt` Section 6.2
