# examples/wt_server を draft-14 以降の RESET_STREAM_AT 必須クライアントに対応させる

- Created: 2026-11-02
- Completed: {YYYY-MM-DD}
- Branch: feature/change-wt-example-reset-stream-at
- Polished: {YYYY-MM-DD}

## 目的

`examples/wt_server` が draft-14 / 15 / 16 の WebTransport クライアントと接続できるようにする。これらの draft は `reset_stream_at` transport parameter と `RESET_STREAM_AT` フレームを必須とするが、現在の example は `reset_stream_at_supported = false` を固定で渡しており、当該 draft のクライアントから CONNECT が `H3_MESSAGE_ERROR` で拒否される。

## 現状

- `examples/wt_server/src/webtransport.rs` は `set_webtransport_transport_verified(true, false)` を呼び、`reset_stream_at_supported` を `false` 固定にしている
- `examples/wt_server/README.md` は「s2n-quic 1.86 は RESET_STREAM_AT の送出 API も、ピアの `reset_stream_at` transport parameter を取得する API も公開していない」ため draft-02 / 07 クライアントにしか接続できないと明記している
- `src/connection/wt_session.rs` のサーバー側検証は、ネゴシエートした draft が `reset_stream_at` を要求する場合に CONNECT を `H3_MESSAGE_ERROR` で拒否する
- sans-I/O 層の transport parameter 伝達 API は `issues/pending/0016-design-transport-parameter-api.md` で別途検討中

## 設計方針

- s2n-quic がピアの `reset_stream_at` transport parameter を取得する API と `RESET_STREAM_AT` を送出する API を公開したら、取得結果を `set_webtransport_transport_verified` の第 2 引数に渡す
- WebTransport データストリームをリセットする際は、Reliable Size を WT ヘッダー長以上に設定する (draft-ietf-webtrans-http3-16 Section 4)
- 公開されるまでは `false` 固定と README の既知の制限を維持する
- 上流 API の有無を実行時に判定できる場合は、両方の経路をテストする

## 完了条件

- draft-14 / 15 / 16 のクライアントが `examples/wt_server` に接続できる
- WT データストリームをリセットしても、ピアがセッション関連付けを維持できる (E2E テスト)
- s2n-quic が API を公開していない間は、README の既知の制限が維持されている
- `cargo test --workspace --tests` / `cargo fmt --all -- --check` / `cargo clippy --workspace --all-targets -- -D warnings` が通る

## pending の理由

s2n-quic 1.86 が `RESET_STREAM_AT` の送出 API とピア transport parameter の取得 API を公開しておらず、このリポジトリ側だけでは実装できない。上流のリリース待ちであるため pending とする。sans-I/O 層の transport parameter API (`issues/pending/0016-design-transport-parameter-api.md`) の設計とも関係する。

## 解決方法

### 関連ファイル

- `examples/wt_server/src/webtransport.rs` (`set_webtransport_transport_verified` の呼び出し)
- `examples/wt_server/README.md` (既知の制限)
- `src/connection/wt_session.rs` (`reset_stream_at` を要求する draft の CONNECT 検証)
- `src/connection/server.rs` / `src/connection/client.rs` (`set_webtransport_transport_verified`)

### 一次資料

- `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 3.1 (Transport Parameters) / Section 4 (Stream Resets)
- `refs/quic/rfc9000.txt` Section 19.4 (RESET_STREAM)
