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
- [ADD] `qpack::Header::from_static` を `const fn` で追加し、リテラル定数の RFC 9114 / RFC 9110 違反を `const` / `static` 宣言時にコンパイル時 panic として検出可能にする
  - @voluntas
- [ADD] `qpack::HeaderError` を新設し、`Header::new` のバリデーションエラーを構造化する (`#[non_exhaustive]`)
  - @voluntas
- [ADD] `validation` に `:protocol` 値検査 (RFC 8441 Section 4 / RFC 9220 Section 3 / RFC 9110 Section 7.8 の HTTP Upgrade Token 構文) を追加する
  - @voluntas
- [ADD] `qpack::Header::new` / `Header::from_static` の `:protocol` 値検査 (HTTP Upgrade Token 構文) を構築時検査に組み込む
  - @voluntas
- [ADD] `internal-test` フィーチャーを追加し、PBT / fuzz / 統合テストから検査バイパス API (`Header::from_validated_parts`) を利用できるようにする (通常のアプリケーションでは有効化しない)
  - @voluntas
- [ADD] `Setting` enum を新設し、SETTINGS パラメータの ID と型安全な値 (`VarInt` または `bool`) を一体で表現する (`#[non_exhaustive]`)
  - @voluntas
- [ADD] `UnknownSetting` 構造体を新設し、`Setting::Unknown(UnknownSetting)` のフィールドを
  private 化することで HTTP/2 専用 ID / 予約 ID が `Setting::Unknown` 経由で構築できない
  不変条件を保証する
  - @voluntas
- [ADD] `SettingError` を新設し、`Setting::from_wire` / `SettingsPayload::add` の検査エラー
  (HTTP/2 専用 ID / 予約 ID / bool 値域外 / 重複 ID) を構造化エラーで通知する (`#[non_exhaustive]`)
  - @voluntas
- [CHANGE] `varint::encode` / `varint::encode_into_vec` / `varint::decode` のシグネチャを `VarInt` を扱う形に変更する
  - @voluntas
- [CHANGE] `varint::MAX_VALUE` / `varint::encoded_len(u64)` / `varint::try_encoded_len` / `varint::try_encode_into_vec` / `varint::EncodeError::ValueTooLarge` を削除する (`VarInt` 型が値域を保証するため)
  - @voluntas
- [CHANGE] `frame::encoded_frame_len` の戻り値型を `usize` から `Option<usize>` に変更し、`Frame::Unknown` 等の `u64` フィールドが VarInt 範囲外の場合に `None` を返すようにする
  - @voluntas
- [CHANGE] `qpack::Header::new` を `Result<Self, HeaderError>` 化し、field-name / field-value / 疑似ヘッダー名・値の構築時検査を強制する
  - @voluntas
- [CHANGE] `qpack::Header` のフィールドを private 化し、`name()` / `value()` / `size()` アクセサを提供する。内部表現を `Cow<'static, [u8]>` に変更する (将来 issue 0059 の `Bytes` 化と統合予定)
  - @voluntas
- [CHANGE] `qpack::DecodedHeader` を削除し、`qpack::Decoder::decode` / `DynamicDecoder::decode` の戻り値型を `Vec<Header>` / `DecodeOutput::Decoded(Vec<Header>)` に統一する
  - @voluntas
- [CHANGE] `qpack::StaticEntry` を削除し、`STATIC_TABLE` を `&[Header]` 化する。`get_static_entry` の戻り値型を `Option<&'static Header>` に変更する
  - @voluntas
- [CHANGE] `validation::HeaderField` トレイトを削除し、`validate_request_headers` 等の
  検証関数群 (`validate_response_headers` / `validate_headers` / `validate_content_length` /
  `validate_trailer_headers` / `calculate_field_section_size` / `check_field_section_size`)
  を `&[Header]` 直受けに変更する
  - @voluntas
- [CHANGE] `webtransport::ConnectRequest::to_headers` / `webtransport::ConnectResponse::to_headers` の戻り値型を `Result<Vec<Header>, HeaderError>` に変更し、フィールド値の RFC 違反を構造化エラーで通知する
  - @voluntas
- [CHANGE] `settings::SettingsId` enum と `webtransport::SettingsId` enum を削除し、`Setting` enum に統合する
  - @voluntas
- [CHANGE] `Settings` / `webtransport::Settings` の値フィールドの型を `u64` から `VarInt` に
  変更し、ビルダーメソッドのシグネチャを `VarInt` 受けに変更する
  - @voluntas
- [CHANGE] `Settings::iter()` および `webtransport::Settings::iter()` の戻り値型を
  `impl Iterator<Item = (u64, u64)>` から `impl Iterator<Item = Setting>` に変更する
  - @voluntas
- [CHANGE] `Settings::from_payload` を `&[Setting]` 経由のマッピングに書き換え、bool 値検査と
  HTTP/2 専用 / 予約 ID 検査を `Setting::from_wire` に集約する。WebTransport 設定は
  `webtransport::Settings::from_payload(&[Setting])` 経由で組み立てる。重複検出を
  `SettingsPayload::add` に移したことで失敗経路が無くなり、戻り値型を
  `Result<Self, Error>` から `Self` に変更する
  - @voluntas
- [CHANGE] `webtransport::Settings::from_payload` を `pub fn from_payload(&SettingsPayload) -> Result<Option<Self>, Error>`
  から `pub(crate) fn from_payload(&[Setting]) -> Option<Self>` に変更する (外部 API から削除)
  - @voluntas
- [CHANGE] `Settings::from_limits` のシグネチャを `Result<Self, VarIntError>` に変更し、
  `Limits` の値が VarInt 範囲外でも panic しないようにする
  - @voluntas
- [CHANGE] `frame::SettingsPayload.entries: Vec<(u64, u64)>` (pub フィールド) を private 化し、
  `settings: Vec<Setting>` + 重複検出用 `HashSet<VarInt>` を内部に保持する。
  `add(id: u64, value: u64)` のシグネチャを `add(setting: Setting) -> Result<(), SettingError>`
  に変更し、SETTINGS フレーム内の重複 ID を構築時に弾く (RFC 9114 Section 7.2.4 MUST NOT)。
  `settings()` / `len()` / `is_empty()` アクセサを提供する
  - @voluntas
- [CHANGE] `webtransport::ServerSettingsParams` の各フィールド型を `u64` から `VarInt` に
  変更し、`DraftVersion::build_*_settings` の panic 経路を解消する
  - @voluntas
- [CHANGE] `error::FrameDecodeError::InvalidSettingsId(u64)` を削除し、
  `InvalidSetting(SettingError)` で HTTP/2 専用 / 予約 / bool 値域外 / 重複 ID の各 SETTINGS
  検査エラーを単一バリアントで伝播する形に変更する。`core::error::Error::source()` 経由で
  `SettingError` を辿れるようにする
  - @voluntas
- [CHANGE] `error::Error` および `error::FrameDecodeError` に `#[non_exhaustive]` を付与し、
  将来のバリアント追加を後方互換に保つ
  - @voluntas
- [CHANGE] `webtransport::Settings::flow_control_enabled` と
  `webtransport::Settings::allows_multiple_sessions_with_peer` を削除する
  (それぞれ `declares_flow_control` / `flow_control_enabled_with_peer` への単純委譲だった)
  - @voluntas

### misc

- [UPDATE] 仕様引用の節番号を一次資料 (`refs/`) に合わせてコメントを修正する
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
- [ADD] `refs/` に RFC 7541, RFC 9110, RFC 9651 の一次資料を追加する
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
