# tokio-s2n-quic の WtSession::close を self 消費型に変更して二重呼び出しを型で防ぐ

- Created: 2026-08-27
- Completed: {YYYY-MM-DD}
- Branch: feature/change-s2n-wt-session-close-consume-self
- Polished: {YYYY-MM-DD}

## 目的

`WtSession::close` の二重呼び出しを型システムで防ぎ、`finish()` 済み CONNECT ストリームへの再送信によるエラー・不定挙動を排除する。

## 現状

- `crates/tokio-s2n-quic/src/webtransport/session.rs` の `WtSession::close` は `&mut self` で定義されており、返り値は `crate::Result<()>` (呼び出し後もセッションオブジェクトが有効)
- 内部では `SendStream::send()` でカプセルを送出後、`SendStream::finish()` で CONNECT ストリームに FIN を送る
- `finish()` 済みストリームに対する 2 回目以降の `close()` は `send()` / `finish()` が s2n-quic 側からエラーを返す (もしくは silent に失敗する可能性)
- 呼び出し側の誤用防止・API 明示性の観点で、close 後にセッションオブジェクトを再利用できてしまう現状は footgun
- 既存呼び出し元は `crates/tokio-s2n-quic/examples/wt_echo_client.rs` の 1 箇所のみで、1 回しか呼んでいない

## 設計方針

- `WtSession::close` のシグネチャを `pub async fn close(self, code: u32, reason: &str) -> crate::Result<()>` に変更する (`&mut self` → `self` 消費)
- 呼び出し側でセッションオブジェクトを consume するようにすることで、2 回目の `close()` はコンパイルエラーになる
- 呼び出しが `close()` 後にセッションから何かを取り出す必要が無いことを既存呼び出し元 (`examples/wt_echo_client.rs`) で確認する
- 破壊的変更 (API シグネチャ変更) なので `CHANGES.md` は `[CHANGE]` として記載
- 代替案として `close()` 内部で idempotent フラグを持たせて 2 回目以降を no-op にする方法も考えられるが、`&mut self` のままだと呼び出し側でセッションを持ち続けられ、close 済みかどうかがオブジェクト外部から判別できない。型で防ぐ方が意図が明確

## 完了条件

- `WtSession::close` のシグネチャが `pub async fn close(self, code: u32, reason: &str) -> crate::Result<()>` になる
- `crates/tokio-s2n-quic/examples/wt_echo_client.rs` を含む全呼び出し元がコンパイルできる
- 型システムで二重呼び出しがコンパイルエラーになることをコンパイルフェイルテスト (`compile_fail` docstring) 等で確認する (任意)
- `CHANGES.md` の `## develop` セクションに `[CHANGE]` エントリが追加される
- `cargo test --workspace --tests` / `cargo fmt --all -- --check` / `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `crates/tokio-s2n-quic/src/webtransport/session.rs` (`WtSession::close` シグネチャ変更)
- `crates/tokio-s2n-quic/examples/wt_echo_client.rs` (呼び出し側の対応)

### 一次資料

- `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 6 (Session Termination。close は 1 セッションに対して 1 回)
