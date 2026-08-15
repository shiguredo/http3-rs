# init_h3_streams にストリーム ID の重複検証がない

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-init-h3-streams-duplicate-id
- Polished: 2026-08-15

## 目的

ストリーム ID の重複設定による無警告のプロトコル破壊 (ピア側の誤解釈による接続エラー) を防ぐ。

## 現状

- `src/connection/mod.rs` の `Connection::init_h3_streams` は `set_control_stream_id` → `set_encoder_stream_id` → `set_decoder_stream_id` を順に呼ぶが、各 setter は自分のフィールドが None かどうかと単方向ストリーム ID の妥当性しか検査せず、他の 2 つの送信ストリーム ID との重複は検査しない
- 例: `set_control_stream_id(2)` の後に `set_encoder_stream_id(2)` を呼ぶと両方成功する
- 同一 QUIC ストリーム ID に 2 つの stream type が割り当てられ、制御・エンコーダー・デコーダーの各ストリームデータが 1 本のストリームに混在して送信される。ピア側は制御ストリーム上の encoder stream type バイト (0x02) 等をフレームとして誤解釈し、接続エラーになる (ローカル側はエラーを返さない)
- RFC 9114 Section 6.2「The purpose is indicated by a stream type, which is sent as a variable-length integer at the start of the stream」の前提が壊れる。エンコーダー / デコーダーストリームの重複禁止は RFC 9204 Section 4.2 が定める (H3_STREAM_CREATION_ERROR)

## 設計方針

- 3 つの setter (`set_control_stream_id` / `set_encoder_stream_id` / `set_decoder_stream_id`) で、自身の ID が他の 2 つの送信ストリーム ID と重複していないかを検証し、重複時は既存の「設定済み」時と同様に `Error::ConnectionError(ErrorCode::StreamCreationError)` を返す (RFC 9114 Section 6.2 の「ストリームの開始時に stream type を 1 つ送る」モデルに反する設定の拒否として、既存 setter と同じ H3_STREAM_CREATION_ERROR を使用する)
- `init_h3_streams` は 3 つの setter を呼ぶため、setter 側の検証で自動的にカバーされる (setter は公開 API のため、`init_h3_streams` 内限定の実装では完了条件を満たさない)

## 完了条件

- 重複するストリーム ID の設定がエラーになる
- テストが追加される
- CHANGES.md の `## develop` セクションに変更履歴が記載される
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/connection/mod.rs` (`Connection::init_h3_streams` / `set_control_stream_id` / `set_encoder_stream_id` / `set_decoder_stream_id`)
- 一次資料: `refs/h3/rfc9114.txt` Section 6.2 / `refs/h3/rfc9204.txt` Section 4.2
