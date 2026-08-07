# WT 未ネゴシエーション時の 0x54 uni stream 受信が接続エラーになる

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-wt-uni-stream-negotiation-error
- Polished: {YYYY-MM-DD}

## 目的

RFC 9114 Section 6.2 の MUST 違反 (未知ストリームタイプを接続エラーにしている) を修正する。

## 現状

- `src/connection/mod.rs` の `Connection::handle_new_unidirectional_stream` はストリームタイプ 0x54 (WT uni stream) を受信したとき、`Connection::is_wt_fully_negotiated()` が false なら `Error::ConnectionError(ErrorCode::StreamCreationError)` を返し**接続全体**をエラーにする
- RFC 9114 Section 6.2「The recipient MUST NOT consider unknown stream types to be a connection error of any kind」に違反。WT 未ネゴシエーション時の 0x54 は unsupported stream type であり、ストリーム単位の拒否 (abort / discard) か無視が正しい
- クライアントの SETTINGS より先に同一フライトで送られうる 0x54 で接続が死ぬ DoS 経路でもある。draft-16 Section 4.6 のバッファリング推奨とも矛盾
- `src/connection/mod.rs` の inline テスト `test_wt_uni_stream_disabled_returns_error` がこの違反を期待値として固定化している

## 設計方針

- 0x54 をネゴシエーション未完了時に受信したら、接続エラーではなくストリームレベルの拒否 (例: `Error::StreamError(ErrorCode::StreamCreationError)` または破棄) に変更する
- テストを仕様準拠の期待値に修正する

## 完了条件

- WT 未ネゴシエーション時の 0x54 受信で接続エラーにならない
- テストが修正・追加される
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/connection/mod.rs` (`Connection::handle_new_unidirectional_stream` / `Connection::is_wt_fully_negotiated` / `test_wt_uni_stream_disabled_returns_error`)
- 一次資料: `refs/h3/rfc9114.txt` Section 6.2、`refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 4.6
