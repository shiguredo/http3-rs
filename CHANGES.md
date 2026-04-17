# 変更履歴

- UPDATE
  - 後方互換がある変更
- ADD
  - 後方互換がある追加
- CHANGE
  - 後方互換のない変更
- FIX
  - バグ修正

## develop

### misc

- [ADD] ngtcp2/nghttp3 と s2n-quic の WebTransport 相互運用テストを平日 JST 11:00 に実行する GitHub Actions ワークフローを追加する
  - @voluntas
- [UPDATE] edition と rust-version を `[workspace.package]` で共通化し、workspace member は `.workspace = true` で継承するようにする
  - @voluntas
- [ADD] fuzz 用に `fuzz/rust-toolchain.toml` を追加し nightly toolchain を指定する
  - @voluntas
