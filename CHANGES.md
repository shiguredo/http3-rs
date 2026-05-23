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

- [ADD] `VarInt` / `VarIntError` を新設し、RFC 9000 Section 16 の値域 (`0..=2^62 - 1`) を型で表現する
  - @voluntas
- [ADD] `VarInt::from_static` を `const fn` で提供し、`2^62` 以上のリテラル定数を `const` / `static` 宣言時にコンパイル時 panic として検出可能にする
  - @voluntas
- [ADD] `webtransport::DatagramError::SessionIdOutOfRange` を追加し、`Datagram::new` に session_id の VarInt 範囲検査を導入する
  - @voluntas
- [ADD] `webtransport::stream::StreamHeaderDecodeError::SessionIdOutOfRange` を追加し、`StreamHeader::new` に session_id の VarInt 範囲検査を導入する
  - @voluntas
- [ADD] `varint::DecodeError` / `varint::EncodeError` / `webtransport::DatagramError` / `webtransport::stream::StreamHeaderDecodeError` に `#[non_exhaustive]` を付与し、将来のバリアント追加を後方互換にする
  - @voluntas
- [CHANGE] `varint::encode` / `varint::encode_into_vec` / `varint::decode` のシグネチャを `VarInt` を扱う形に変更する
  - @voluntas
- [CHANGE] `varint::MAX_VALUE` / `varint::encoded_len(u64)` / `varint::try_encoded_len` / `varint::try_encode_into_vec` / `varint::EncodeError::ValueTooLarge` を削除する (`VarInt` 型が値域を保証するため)
  - @voluntas
- [CHANGE] `frame::encoded_frame_len` の戻り値型を `usize` から `Option<usize>` に変更し、`Frame::Unknown` 等の `u64` フィールドが VarInt 範囲外の場合に `None` を返すようにする
  - @voluntas

### misc

- [UPDATE] 仕様引用の節番号を一次資料 (`refs/`) に合わせてコメントを修正する
  - @voluntas
- [ADD] `refs/` に RFC 7541, RFC 9110, RFC 9651 の一次資料を追加する
  - @voluntas
- [UPDATE] 相互運用テスト用クレートの配置を `interop_h3` / `interop_wt` から `interop/h3` / `interop/wt` に移す
  - @voluntas
- [UPDATE] `aws-lc-sys` を `0.40` 系へ更新する
  - @voluntas
- [UPDATE] `examples/wt_server` を workspace member に含め、`edition` / `rust-version` を workspace 継承に変更し、個別の `Cargo.lock` を削除する
  - @voluntas
- [UPDATE] edition と rust-version を `[workspace.package]` で共通化し、workspace member は `.workspace = true` で継承するようにする
  - @voluntas
- [UPDATE] fuzz ターゲットからラウンドトリップ等のプロパティ検証を削除し、パニック安全性の検証だけに絞る
  - @voluntas
- [ADD] ngtcp2/nghttp3 と s2n-quic の WebTransport 相互運用テストを平日 JST 11:00 に実行する GitHub Actions ワークフローを追加する
  - @voluntas
- [ADD] fuzz 用に `fuzz/rust-toolchain.toml` を追加し nightly toolchain を指定する
  - @voluntas
- [ADD] `prop_qpack.rs` に `DynamicEncoder` / `DynamicDecoder` ラウンドトリップと Blocked/Unblocked のプロパティ検証を追加する
  - @voluntas
- [FIX] `fuzz/fuzz_targets/fuzz_settings.rs` が `Settings::from_payload` の `Result` 戻り値に追従しておらず fuzz crate がコンパイルできなかった問題を修正する
  - @voluntas
- [FIX] CI の共通 workspace job から `interop/h3` / `interop/wt` を除外し、相互運用テストは macOS 専用 step でのみ実行する
  - @voluntas
