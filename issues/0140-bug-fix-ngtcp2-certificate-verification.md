# ngtcp2 系クライアントの証明書検証が無効 (verify_peer=true でも検証ゼロ)

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-ngtcp2-certificate-verification
- Polished: {YYYY-MM-DD}

## 目的

`shiguredo_ngtcp2` / `tokio-ngtcp2` のクライアントが「検証あり」を謳いながら一切証明書を検証しておらず、MITM 攻撃に無防備な状態を修正する。

## 現状

- `crates/ngtcp2-rs/src/crypto.rs` の `TlsContext::new_client_with_options` は `SSL_CTX_set_verify(ctx, SSL_VERIFY_NONE, ...)` を `verify_peer == false` の時しか呼ばない
- BoringSSL (aws-lc) の `SSL_CTX` デフォルト検証モードは `SSL_VERIFY_NONE` のため、`verify_peer == true` の時は何も設定されず検証なしのまま (`ngtcp2_crypto_boringssl_configure_client_context` は TLS 1.3 固定と QUIC method の設定のみで検証モードには触れない)
- ホスト名検証 (`SSL_set1_host` 相当) も未設定。`TlsSession::set_server_name` は SNI 送信 (`SSL_set_tlsext_host_name`) のみ
- `TlsContext::new_client` と `tokio-ngtcp2` の `Client::connect` / `ClientWebTransportSession::connect` のデフォルトは true であり、「検証する」設定がそのまま「検証しない」動作になる
- RFC 9114 Section 3.1 の証明書検証 MUST に違反

RFC 9114 Section 3.1 (Discovering an HTTP/3 Endpoint):

> Upon receiving a server certificate in the TLS handshake, the client MUST verify that the certificate is an acceptable match for the URI's origin server using the process described in Section 4.3.4 of [HTTP].

## 設計方針

- `verify_peer == true` の時に `SSL_CTX_set_verify(ctx, SSL_VERIFY_PEER, ...)` とトラストストア設定 (`SSL_CTX_set_default_verify_paths`) を追加する。`SSL_CTX_set_default_verify_paths` の戻り値は無視し、トラストストアが空でも接続作成は成功させる (検証失敗はハンドシェイク時に発生する)
  - トラストストアは aws-lc のデフォルト CA パス (`/etc/ssl/cert.pem` / `/etc/ssl/certs`) と環境変数 `SSL_CERT_FILE` / `SSL_CERT_DIR` に依存する。macOS では `/etc/ssl/cert.pem` が存在しない環境があり、その場合はトラストストアが空になり検証が全て失敗する (プラットフォーム依存を許容する)
- **ホスト名検証 (`SSL_set1_host` 相当) も追加する**。`TlsSession::set_server_name` (SNI 設定) の中で `SSL_set1_host` も呼び (戻り値は `SSL_set_tlsext_host_name` と同様にチェックする)、`Connection::client_new` が受け取る `server_name` を検証に使う。`verify_peer == false` の時は `SSL_VERIFY_NONE` のためホスト名検証も効かない (安全)
- **カスタム CA を読み込む手段を追加する**。`tokio-ngtcp2` の `Client` / `ClientWebTransportSession` に CA の PEM 文字列を受け取る公開 API を追加する (tokio-s2n-quic の `ca_cert_pem` と同様の形。破壊的変更を避けるため新メソッドとして追加し、既存の `connect` 系は変更しない)。`TlsContext` 側は `SSL_CTX_load_verify_locations` 相当で実装する。CA ロードは `verify_peer == true` の時のみ有効にする
- 自己署名証明書を使う interop テスト側は既に `verify_peer = false` (`connect_insecure`) を使用しており、既存挙動を維持する (変更不要。確認のみ)

## 完了条件

- `verify_peer = true` で接続したとき、自己署名証明書のサーバーへの接続が失敗する (チェーン検証が効く)
- `verify_peer = true` + カスタム CA ロードで接続したとき、ロードした CA が署名した証明書でホスト名不一致のサーバーへの接続が失敗する (ホスト名検証が効く)
- `verify_peer = true` + カスタム CA ロードで接続したとき、ロードした CA が署名した証明書でホスト名一致のサーバーへの接続が成功する (CA ロードとチェーン検証が正しく機能する。失敗系だけでは CA ロードの破損とホスト名検証を区別できないため)
- `verify_peer = false` で接続したとき、自己署名証明書のサーバーへの接続が成功する (既存挙動維持)
- テストが追加される: `crates/tokio-ngtcp2/tests/` に rcgen でルート CA (`IsCa::Ca` を設定) と、CA 署名の自己署名 / ホスト名不一致証明書を生成し、追加した CA ロード API 経由で接続結果を検証するテストを追加する (検証失敗は `handshake()` の `Err` または短いタイムアウトで観測する)
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `crates/ngtcp2-rs/src/crypto.rs` (`TlsContext::new_client_with_options` / `TlsSession::set_server_name` / `SSL_CTX_set_default_verify_paths` / `SSL_set1_host` / `SSL_CTX_load_verify_locations`)。`set_server_name` の doc コメントを「SNI 兼ホスト名検証の設定」に更新する
- `crates/ngtcp2-rs/src/conn.rs` (`Connection::client_new` の server_name 受け渡し確認)
- `crates/tokio-ngtcp2/src/client.rs` (CA の PEM 文字列を受け取る公開 API の追加。既存の `connect` / `connect_insecure` / `connect_insecure_default` は変更しない)
- `crates/tokio-ngtcp2/src/webtransport.rs` (`ClientWebTransportSession` にも同様の CA 受け取り API を追加)
- 影響範囲 (変更不要): `crates/tokio-ngtcp2/tests/integration.rs` の `Client::connect` 使用テスト (`test_client_creation` / `test_client_is_send` / `test_webtransport_client_creation` はハンドシェイクを実行しないため影響なし、`test_client_server_handshake` はハンドシェイク失敗を許容する構造のため壊れないが、実装後は必ず検証失敗になる性質に変わる)
- 一次資料: `refs/h3/rfc9114.txt` Section 3.1 (Discovering an HTTP/3 Endpoint)、`refs/quic/rfc9001.txt` Section 4.4 (Peer Authentication)
