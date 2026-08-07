# ngtcp2 系クライアントの証明書検証が無効 (verify_peer=true でも検証ゼロ)

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-ngtcp2-certificate-verification
- Polished: {YYYY-MM-DD}

## 目的

`shiguredo_ngtcp2` / `tokio-ngtcp2` のクライアントが「検証あり」を謳いながら一切証明書を検証しておらず、MITM 攻撃に無防備な状態を修正する。

## 現状

- `crates/ngtcp2-rs/src/crypto.rs` の `TlsContext::new_client_with_options` は `SSL_CTX_set_verify(ctx, SSL_VERIFY_NONE, ...)` を `verify_peer == false` の時しか呼ばない
- BoringSSL (aws-lc) の `SSL_CTX` デフォルト検証モードは `SSL_VERIFY_NONE` のため、`verify_peer == true` の時は何も設定されず検証なしのまま。ホスト名検証 (`SSL_set1_host` 相当) も未設定
- `TlsContext::new_client` と `tokio-ngtcp2` の `Client::connect` のデフォルトは true であり、「検証する」設定がそのまま「検証しない」動作になる
- RFC 9114 Section 3.3 の証明書検証 MUST に違反

## 設計方針

- `verify_peer == true` の時に `SSL_CTX_set_verify(ctx, SSL_VERIFY_PEER, ...)` とトラストストア設定 (デフォルト CA パス等) を追加する
- 自己署名証明書を使う interop テスト側は `verify_peer = false` を明示して既存挙動を維持する

## 完了条件

- `verify_peer = true` で接続したとき、自己署名証明書のサーバーへの接続が失敗する (検証が効く)
- `verify_peer = false` で接続したとき、自己署名証明書のサーバーへの接続が成功する (既存挙動維持)
- テストが追加される
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `crates/ngtcp2-rs/src/crypto.rs` (`TlsContext::new_client_with_options`)
- `crates/tokio-ngtcp2/src/client.rs` (`Client::connect` の verify_peer 引数)
- 一次資料: `refs/h3/rfc9114.txt` Section 3.3
