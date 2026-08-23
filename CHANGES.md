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

- [TEST] `tokio-s2n-quic` に `connection_state.rs` のユニットテストを追加する (QPACK ストリーム初期化 / SETTINGS 受信 / リクエスト・レスポンス送信 / エラー透過)
- [ADD] `Server::run_by_conn_id` / `ServerWebTransportSession::run_by_conn_id` / `ServerWebTransportSession::recv_once_by_conn_id` を追加し、ハンドラでコネクション ID (サーバー生成 SCID) を受け取れるようにする (同一 SocketAddr からの複数接続のイベント区別用。既存 `run` / `recv_once` は後方互換維持)
  - @voluntas
- [CHANGE] MSRV (Minimum Supported Rust Version) を 1.88 から 1.93 に引き上げる
  - @voluntas
- [CHANGE] `Client::connect` / `ClientWebTransportSession::connect` / `TlsContext::new_client` でサーバー証明書のチェーン検証とホスト名検証を有効にする (従来 `verify_peer=true` は検証なしと同等の挙動だった)。検証に失敗する既存接続は失敗するようになる (RFC 9114 Section 3.1 / RFC 9001 Section 4.4)
  - @voluntas
- [CHANGE] `server_name` を DNS 名 (FQDN) に限定し、IP アドレス / 空文字列 / ワイルドカード / 255 文字超を `InvalidArgument` エラーで拒否する (`connect_insecure` を含む全接続 API に適用。RFC 6066 Section 3 / RFC 1035 Section 2.3.4)
  - @voluntas
- [CHANGE] 公開エラー型 / 設定型などから `#[non_exhaustive]` を撤去し、`match` の網羅性チェックを利用側で保てるようにする
  - @voluntas
- [CHANGE] `shiguredo_ngtcp2` の `Http3SettingsExt` / `TransportParamsExt` トレイトを `Http3Settings` / `TransportParams` newtype に置き換え、`nghttp3_sys` / `ngtcp2_sys` 型の再エクスポートを廃止する
  - @voluntas
- [CHANGE] `Connection::peer_goaway_request_boundary` の戻り値型を `Option<u64>` から `Option<VarInt>` に変更し、GOAWAY ID の値域を型で保証する
  - @voluntas
- [CHANGE] `FrameDecodeError::Http2Frame` / `FrameDecodeError::ServerPushNotSupported` のフィールド型を `u64` から `VarInt` に変更し、HTTP/3 frame type の値域 (RFC 9000 Section 16) を型で保証する
  - @voluntas
- [CHANGE] `stream::request::ReceivedData` enum を削除する (未使用の死にコード)
  - @voluntas
- [CHANGE] `Connection::send_request` / `Connection::send_response` を `pub(crate)` に変更し `ClientConnection` / `ServerConnection` 経由でのみ呼び出し可能にする
  - @voluntas
- [CHANGE] `Event` enum の WebTransport バリアント 14 個を `Event::WebTransport(WebTransportEvent)` にネスト化する
  - @voluntas
- [CHANGE] 重複した Host ヘッダーを持つリクエストを `H3_MESSAGE_ERROR` で拒否する (RFC 9110 Section 5.3)
  - @voluntas
- [CHANGE] `:authority` が無く Host ヘッダーのみで authority を運ぶリクエストの Host 値を uri-host[:port] 構文で検証し、不正値を `H3_MESSAGE_ERROR` で拒否する (RFC 9110 Section 7.2)
  - @voluntas
- [CHANGE] `varint::encode` / `varint::encode_into_vec` / `varint::decode` のシグネチャを `VarInt` を扱う形に変更する
  - @voluntas
- [CHANGE] `varint::MAX_VALUE` / `varint::encoded_len(u64)` / `varint::try_encoded_len` / `varint::try_encode_into_vec` / `varint::EncodeError::ValueTooLarge` を削除する (`VarInt` 型が値域を保証するため)
  - @voluntas
- [CHANGE] `frame::encoded_frame_len` の戻り値型を `usize` から `Option<usize>` に変更し、`Frame::Unknown` 等の `u64` フィールドが VarInt 範囲外の場合に `None` を返すようにする
  - @voluntas
- [CHANGE] `qpack::Header::new` を `Result<Self, HeaderError>` 化し、field-name / field-value / 疑似ヘッダー名・値の構築時検査を強制する
  - @voluntas
- [CHANGE] `qpack::Header` のフィールドを private 化し、`name()` / `value()` / `size()` アクセサを提供する。内部表現を `Cow<'static, [u8]>` に変更する (将来の `Bytes` 化と統合予定)
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
- [CHANGE] `webtransport::Settings::flow_control_enabled` と
  `webtransport::Settings::allows_multiple_sessions_with_peer` を削除する
  (それぞれ `declares_flow_control` / `flow_control_enabled_with_peer` への単純委譲だった)
  - @voluntas
- [CHANGE] `frame::DataPayload` / `frame::HeadersPayload` / `frame::GoawayPayload` の
  全フィールドを private 化する。アクセサ経由でのみ参照可能にすることで構築後の改ざんを防ぐ
  - @voluntas
- [CHANGE] `frame::Frame::Unknown` を struct variant から tuple variant
  (`Unknown(UnknownFrame)`) に変更し、フィールドを private 化する
  - @voluntas
- [CHANGE] `frame::GoawayPayload.id` / `frame::Frame::MaxPushId` /
  `frame::UnknownFrame::frame_type` / `frame::Frame::frame_type()` の型を `u64` から
  `VarInt` に変更し、RFC 9000 Section 16 の値域を型レベルで担保する。
  `frame::GoawayPayload::new(id: VarInt)` のシグネチャも変更する
  - @voluntas
- [CHANGE] `frame::FrameHeader.frame_type` / `payload_len` の型を `u64` から `VarInt` に
  変更してフィールドを private 化し、`frame_type()` / `payload_len()` / `header_len()` の
  アクセサを提供する。`total_len()` の戻り値型を `usize` から `Option<usize>` に変更し、
  32bit プラットフォームで `payload_len` が `usize` を超える場合に `None` を返す
  - @voluntas
- [CHANGE] `frame::encode_frame_header` のシグネチャを `(buf, frame_type: VarInt, payload_len: VarInt)`
  に変更し、`u64` 受けに伴う値域検査経路を削除する
  - @voluntas
- [CHANGE] `event::Event::GoawayReceived.id` の型を `u64` から `VarInt` に変更する
  - @voluntas
- [CHANGE] `Connection::send_goaway` / `ClientConnection::send_goaway` /
  `ServerConnection::send_goaway` の引数型を `u64` から `VarInt` に変更し、
  ローカル API 利用時の値域違反を型レベルで排除する
  - @voluntas
- [CHANGE] `qpack::DynamicTable::insert_with_name_ref` を削除する (未使用の公開 API。RFC 9204 Section 4.3.2 の relative index を absolute index として誤解釈するバグを含んでいた)
  - @voluntas
- [CHANGE] `CapsuleProcessError::Connection` variant を削除し、WT_MAX_STREAMS > 2^60 のエラーを接続エラーからセッションエラー (`WT_FLOW_CONTROL_ERROR`) に変更する (draft-ietf-webtrans-http3-16 Section 5.6.2)
  - @voluntas
- [CHANGE] フロー制御カプセルの単調性チェックを「前より小さい」から「前より増加しない」に厳密化し、同値の再送も `WT_FLOW_CONTROL_ERROR` で拒否する (draft-ietf-webtrans-http3-16 Section 5.6.2 / 5.6.4)
  - @voluntas
- [CHANGE] クライアントが `SETTINGS_WT_ENABLED > 1` を受信した際に `H3_SETTINGS_ERROR` 接続エラーを返す検証を追加する (draft-ietf-webtrans-http3-16 Section 3.1)
  - @voluntas
- [CHANGE] `RequestStream::new` のシグネチャを `new(stream_id: u64, role: Role)` に変更し、WT_STREAM (0x41) の先頭位置判定に接続ロールを使用する (draft-ietf-webtrans-http3-16 Section 4.3)
  - @voluntas
- [ADD] `Client::connect_with_ca` / `ClientWebTransportSession::connect_with_ca` / `TlsContext::add_ca_cert_pem` を追加し、検証に使用するカスタム CA 証明書 (PEM) をトラストストアに追加できるようにする
  - @voluntas
- [ADD] `VarInt::from_static` / `qpack::Header::from_static` / `frame::GoawayPayload::from_static` の doc に `compile_fail` ブロックを追加し、`const fn` 検査のリグレッションを CI (`cargo test --doc`) で防止する
  - @voluntas
- [ADD] 構築時検査の `from_static` ↔ `new` 一貫性、`from_validated_parts` ↔ `new` 整合性、`Header::new` ↔ QPACK ラウンドトリップ完全性を検証する PBT を追加する
  - @voluntas
- [ADD] `Makefile` に `doc-test` ターゲットを追加し `cargo test --doc --workspace --exclude nghttp3-sys --exclude ngtcp2-sys` を実行する (sys クレートは bindgen 生成 doc が rustc でパースできないため除外)
  - @voluntas
- [ADD] `pbt` クレートに `strategies` モジュールを追加し、構築時検査型の `valid_*` / `invalid_*` 戦略を集約する (`pbt/Cargo.toml` の `proptest` / `shiguredo_http3` を `[dev-dependencies]` から `[dependencies]` に格上げ)
  - @voluntas
- [ADD] `VarInt` / `VarIntError` を新設し、RFC 9000 Section 16 の値域 (`0..=2^62 - 1`) を型で表現する
  - @voluntas
- [ADD] `VarInt::from_static` を `const fn` で提供し、`2^62` 以上のリテラル定数を `const` / `static` 宣言時にコンパイル時 panic として検出可能にする
  - @voluntas
- [ADD] `webtransport::DatagramError::SessionIdOutOfRange` を追加し、`Datagram::new` に session_id の VarInt 範囲検査を導入する
  - @voluntas
- [ADD] `webtransport::stream::StreamHeaderDecodeError::SessionIdOutOfRange` を追加し、`StreamHeader::new` に session_id の VarInt 範囲検査を導入する
  - @voluntas
- [ADD] `qpack::Header::from_static` を `const fn` で追加し、リテラル定数の RFC 9114 / RFC 9110 違反を `const` / `static` 宣言時にコンパイル時 panic として検出可能にする
  - @voluntas
- [ADD] `qpack::HeaderError` を新設し、`Header::new` のバリデーションエラーを構造化する
  - @voluntas
- [ADD] `validation` に `:protocol` 値検査 (RFC 8441 Section 4 / RFC 9220 Section 3 / RFC 9110 Section 7.8 の HTTP Upgrade Token 構文) を追加する
  - @voluntas
- [ADD] `qpack::Header::new` / `Header::from_static` の `:protocol` 値検査 (HTTP Upgrade Token 構文) を構築時検査に組み込む
  - @voluntas
- [ADD] `Setting` enum を新設し、SETTINGS パラメータの ID と型安全な値 (`VarInt` または `bool`) を一体で表現する
  - @voluntas
- [ADD] `UnknownSetting` 構造体を新設し、`Setting::Unknown(UnknownSetting)` のフィールドを
  private 化することで HTTP/2 専用 ID / 予約 ID が `Setting::Unknown` 経由で構築できない
  不変条件を保証する
  - @voluntas
- [ADD] `SettingError` を新設し、`Setting::from_wire` / `SettingsPayload::add` の検査エラー
  (HTTP/2 専用 ID / 予約 ID / bool 値域外 / 重複 ID) を構造化エラーで通知する
  - @voluntas
- [ADD] `GoawayPayload::from_static` を `const fn` で追加し、不正リテラルの GOAWAY ID
  (RFC 9000 Section 16 の VarInt 範囲外) をコンパイル時 panic として検出可能にする
  - @voluntas
- [ADD] `frame::DataPayload` / `frame::HeadersPayload` にアクセサ
  (`data` / `into_data` / `encoded_field_section` / `into_encoded_field_section` /
  `len` / `is_empty`) を追加し、フィールド private 化後も所有権付きで取り出せるようにする
  - @voluntas
- [ADD] `frame::UnknownFrame` / `frame::UnknownFrameError` を新設し、`Frame::Unknown` のフィールドを
  private 化することで既知の HTTP/3 フレームタイプ (DATA / HEADERS / CANCEL_PUSH / SETTINGS /
  PUSH_PROMISE / GOAWAY / MAX_PUSH_ID) や HTTP/2 専用 ID (RFC 9114 Section 11.2.1 Table 2 で
  Reserved 登録、Section 7.2.8 で受信時 H3_FRAME_UNEXPECTED: 0x02 / 0x06 / 0x08 / 0x09) を
  `Unknown` で偽装できない不変条件を保証する
  - @voluntas
- [ADD] ngtcp2-rs クレートの `Http3Event` に `WebTransportCloseSession` バリアントを追加し、nghttp3 の `recv_wt_close_session` コールバック経由で WT_CLOSE_SESSION Capsule の受信 (アプリケーションエラーコード・メッセージ) を通知する
  - @voluntas
- [ADD] SETTINGS フレームに GREASE 予約設定 (RFC 9114 Section 7.2.4.1, ID=0x21) を追加する
  - @voluntas
- [ADD] `Error::classify_connection_error` / `ConnectionErrorKind` を追加し、ngtcp2 の API 契約に従った接続単位のエラー種別をサーバー実装に提供する
  - @voluntas
- [ADD] `Connection::poll_issued_cids` を追加し、NEW_CONNECTION_ID で発行した CID をサーバーがルーティングテーブルに登録できるようにする (RFC 9000 Section 5.1.1)
  - @voluntas
- [ADD] `Server::get_conn_ids` / `Server::send_response_by_conn_id` を追加し、同一アドレスからの複数接続をコネクション ID で指定できるようにする
  - @voluntas
- [ADD] `ServerWebTransportSession::get_established_conn_ids` / `open_bidi_stream_by_conn_id` / `open_uni_stream_by_conn_id` / `send_stream_data_by_conn_id` / `send_datagram_by_conn_id` / `recv_datagram_by_conn_id` を追加し、同一アドレスからの複数接続をコネクション ID で指定できるようにする
  - @voluntas
- [ADD] `Connection::register_local_wt_stream` / `ClientConnection::register_local_wt_stream` / `ServerConnection::register_local_wt_stream` を追加し、ローカル開始の WebTransport 双方向ストリームをセッションに登録できるようにする (RFC 9000 Section 2.1 / draft-ietf-webtrans-http3-16 Section 4.3)
  - @voluntas
- [FIX] content-length 値の検査を 1*DIGIT 文法検査に変更し、`+5` / `-1` 等の符号付き値の受理を拒否する (RFC 9110 Section 8.6)
  - @voluntas
- [FIX] `Connection` に接続エラー状態 (`self.error`) を本番経路で設定し、接続エラー後の `feed_stream` / ポーリング / 送信 API を拒否する (RFC 9114 Section 8.1)
  - @voluntas
- [TEST] `SETTINGS_WT_ENABLED > 1` を受信したクライアントが `H3_SETTINGS_ERROR` になるテストを追加する (draft-ietf-webtrans-http3-16 Section 3.1)
  - @voluntas
- [CHANGE] `webtransport::StreamHeader::session_id` フィールドを private 化し、構築を検証済みの `StreamHeader::new` 経由のみに制限する (値域違反の session_id で encode するとパニックする問題の構造的排除。`session_id()` アクセサを追加)
  - @voluntas
- [FIX] `Frame::Unknown` (frame_type = 0x41) のエンコードを拒否し、WT_STREAM をフレームとして送信できないようにする (draft-ietf-webtrans-http3-16 Section 4.3)
  - @voluntas
- [FIX] QPACK エンコーダーストリームレシーバーでテーブル操作前にバッファを drain していた処理順序を修正する
  - @voluntas
- [FIX] send_request で GOAWAY 境界超過およびフロー制御なし WT セッション上限超過時に ConnectionError ではなく StreamError を返すよう修正する
  - @voluntas
- [FIX] feed_stream がエラー状態で本来のエラーではなく InternalError を返していた問題を修正する
  - @voluntas
- [FIX] send_request / send_response で track_section が send_encoded_headers の前に呼ばれていた問題を修正する
  - @voluntas
- [FIX] WebTransport CONNECT リクエストで fin=true が拒否されない問題を修正する
  - @voluntas
- [FIX] close_with_error がクローズ済みセッションで close_session_sent フラグを誤設定する問題を修正する
  - @voluntas
- [FIX] validate_max_data に VarInt 上限チェックを追加し MaxDataExceedsLimit エラーの発生経路を実装する
  - @voluntas
- [FIX] QPACK エンコーダーの ack_section を Result 化し、Post-Base 参照エンコードを実装し、RIC エンコードの max_entries=0 時のエッジケースを修正する
  - @voluntas
- [FIX] nghttp3 webtransport ブランチに追加された `wt_data_stream_open` コールバックに追従し、`nghttp3-sys` / `ngtcp2-sys` の bindings を再生成して `nghttp3_callbacks` のレイアウト不整合による SIGSEGV を修正する
  - @voluntas
- [FIX] ngtcp2 / nghttp3 webtransport ブランチに追加された `stream_close2` / `recv_stop_sending` / `recv_wt_close_session` コールバックに追従し、`ngtcp2-sys` / `nghttp3-sys` の bindings を再生成してコールバック構造体のレイアウト不整合による WebTransport interop テスト失敗を修正する
  - @voluntas
- [FIX] `ngtcp2-sys` / `nghttp3-sys` の build.rs でクローン済みの上流リポジトリを fetch してリモートブランチの最新にリセットし、stale なキャッシュから古いヘッダで bindings の再生成やビルドが行われる問題を修正する
  - @voluntas
- [FIX] QPACK 整数エンコード/デコードのシフトオーバーフローを修正する (encode_integer: prefix_bits >= 64, decode_integer: prefix_bits >= 16)
  - @voluntas
- [FIX] Huffman デコードで EOS シンボル検出時に `Ok` を返していた RFC 7541 Section 5.2 違反を修正し `Err(QpackError::InvalidHuffman)` を返すようにする
  - @voluntas
- [FIX] STOP_SENDING 受信時のクリティカルストリーム判定を受信側から送信側 (`control_send` / ローカル QPACK encoder・decoder) に修正し、送信側クリティカルストリームへの STOP_SENDING を `H3_CLOSED_CRITICAL_STREAM` とする (RFC 9114 Section 6.2.1, RFC 9204 Section 4.2)
  - @voluntas
- [FIX] payload が欠落した GOAWAY フレームのデコードエラーを `BufferTooShort` から `InvalidLength` に変更し、`H3_FRAME_ERROR` に集約されるよう修正する (RFC 9114 Section 7.1)
  - @voluntas
- [FIX] QPACK Encoder の Indexed Field Line / Literal with Name Reference で手書きの prefix 境界分岐 (`index < 64` / `index < 16`) が continuation byte を欠落させていたバグを修正し、`integer::encode_integer` への一本委譲に統一する (RFC 7541 Section 5.1, RFC 9204 Section 4.5.2 / 4.5.4)
  - @voluntas
- [FIX] WebTransport フロー制御カウンタの u64 加算を `saturating_add` / `checked_sub` に置き換え、オーバーフロー時のフロー制御素通りを防止する (draft-ietf-webtrans-http3-15 Section 5.6)
  - @voluntas
- [FIX] QPACK Post-Base 参照デコードの `base + post_base_index` 加算を `checked_add` に置き換え、算術オーバーフローによる panic / wrap-around を防止する (RFC 9204 Section 4.5.3 / 4.5.5)
  - @voluntas
- [FIX] `Capsule::decode` の `length as usize` 素朴キャストを `usize::try_from` + `checked_add` に置き換え、32-bit 環境での境界判定緩みと 64-bit でのオーバーフローを防止する
  - @voluntas
- [FIX] 禁止 Capsule (`WT_MAX_STREAM_DATA` / `WT_STREAM_DATA_BLOCKED`) 受信時のエラーレベルを接続エラーからセッションエラーに修正する (draft-ietf-webtrans-http3-15 Section 5.4)
  - @voluntas
- [FIX] `send_request` / `send_body` / `send_response` で `fin=true` を設定しても FIN が交付されず、QUIC 層へ送信方向クローズが通知されない問題を修正する (RFC 9114 Section 4.1)
  - @voluntas
- [FIX] 完走・リセット・セッション終了後のストリームと WT セッションが接続終了まで蓄積し続けるメモリリークを修正する。終了済みセッションへの DATA / FIN / RESET / 新規ストリーム / データグラムの拒否・破棄を追加する (RFC 9297 Section 3.2 / draft-ietf-webtrans-http3-16 Section 6)
  - @voluntas
- [FIX] WebTransport CONNECT ストリームの DATA が受信バッファに累積され続ける問題を修正し、content-length / content-type ヘッダー付きの WT CONNECT を H3_MESSAGE_ERROR で拒否する (RFC 9297 Section 3.2)
  - @voluntas
- [FIX] リクエストストリーム・制御ストリームで誤った位置に受信した WT_STREAM (0x41) を H3_FRAME_ERROR 接続エラーとして検出する (draft-ietf-webtrans-http3-16 Section 4.3)
  - @voluntas
- [FIX] WebTransport ネゴシエーション未完了時に受信した 0x54 単方向ストリームを接続エラーではなくストリームエラー (H3_STREAM_CREATION_ERROR) で拒否する (RFC 9114 Section 6.2 / draft-ietf-webtrans-http3-16 Section 4.6)
  - @voluntas
- [FIX] ローカル開始の WebTransport 双方向ストリームにピアから受信したデータが HTTP/3 リクエストストリームとして誤処理される問題を修正する。FIN 時は WT_MAX_STREAMS クレジットを返却せず、RESET_STREAM_AT の reliable size 計算のため登録をセッション終了時まで維持する (RFC 9000 Section 2.1 / draft-ietf-webtrans-http3-16 Section 4.3 / 4.4 / 5.3)
  - @voluntas
- [FIX] RESET_STREAM 受信時に final_size を受信側データフロー制御に計上し、RESET により破棄されるデータのウィンドウ (WT_MAX_DATA) を回復する。既計上分との二重計上を防ぎ、ピア開始ストリームでは WT_MAX_STREAMS クレジットも回復する (RFC 9000 Section 19.4 / draft-ietf-webtrans-http3-16 Section 5.3 / 5.4 / 5.6.4)
  - @voluntas
- [FIX] tokio-s2n-quic の送信経路が FIN 交付をループで取得せず、H3 層に FIN 未交付状態が残留する問題を修正する (RFC 9114 Section 4.1)
  - @voluntas
- [FIX] RESET_STREAM されたバッファリング中 WT ストリームの stale エントリが残存しセッション確立後に誤配送される問題を修正する。deliver_buffered_streams の FC 違反中断時に未配送ストリームが喪失する問題も修正する (draft-ietf-webtrans-http3-16 Section 4.4 / 4.6 / 6)
  - @voluntas
- [FIX] サーバーがクライアント SETTINGS 受信前に WT CONNECT リクエストを受信した場合に即時拒否せず保留し、SETTINGS 受信後に検証するように修正する (draft-ietf-webtrans-http3-16 Section 3.1 / 4.6 / 7.1)
  - @voluntas
- [FIX] ローカル側の CONNECT ストリーム FIN で WT セッションが終了しない問題を修正する (draft-ietf-webtrans-http3-16 Section 6)
  - @voluntas
- [FIX] GOAWAY 送信後に新規リクエスト・WT CONNECT が拒否されず処理され続ける問題を修正する (RFC 9114 Section 5.2)
  - @voluntas

### misc

- [ADD] `fuzz_qpack` に DynamicEncoder, 整数エンコード/デコードの fuzz 経路を追加する
  - @voluntas
- [ADD] `refs/` に RFC 7541, RFC 9110, RFC 9651 の一次資料を追加する
  - @voluntas
- [ADD] ngtcp2/nghttp3 と s2n-quic の WebTransport 相互運用テストを平日 JST 11:00 に実行する GitHub Actions ワークフローを追加する
  - @voluntas
- [ADD] fuzz 用に `fuzz/rust-toolchain.toml` を追加し nightly toolchain を指定する
  - @voluntas
- [ADD] `prop_qpack.rs` に `DynamicEncoder` / `DynamicDecoder` ラウンドトリップと Blocked/Unblocked のプロパティ検証を追加する
  - @voluntas
- [ADD] 0-RTT 再開時に前回接続の WebTransport フロー制御値 (wt_initial_max_streams_uni / wt_initial_max_streams_bidi / wt_initial_max_data) を注入し、値の減少を H3_SETTINGS_ERROR で検出する API を `ClientConnection` に追加する (draft-ietf-webtrans-http3-16 Section 3.2)
  - @voluntas
- [ADD] サーバー側で WebTransport CONNECT の Pending 中に受信したカプセルデータをバッファリングし、2xx 受理時に処理する楽観的カプセル送信をサポートする (draft-ietf-webtrans-http3-16 Section 3.2)
  - @voluntas
- [UPDATE] QPACK 整数エンコード/デコードの重複実装を src/qpack/integer.rs に一本化する
  - @voluntas
- [UPDATE] disassociate_stream の不要な `#[allow(dead_code)]` を削除する
  - @voluntas
- [UPDATE] connection/mod.rs の重複定数を webtransport/session.rs に集約する
  - @voluntas
- [UPDATE] prop_webtransport.rs をディレクトリモジュールに分割し PBT 間の重複テストを削除する
  - @voluntas
- [UPDATE] validation.rs のインラインテストを tests/test_validation.rs に分割する
  - @voluntas
- [UPDATE] `Header::from_static` の `compile_fail` doctest を 3 件追加し、`check_header` の全 7 検査経路をカバーする (token 外 / 先頭末尾空白 / 疑似ヘッダー値構文)
  - @voluntas
- [UPDATE] `pbt/tests/prop_qpack.rs` の未使用 strategy (`valid_capacity` / `valid_relative_index`) と空虚なテスト (`prop_huffman_length_varies`) を削除する
  - @voluntas
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
- [UPDATE] interop/h3 と interop/wt の edition / rust-version を workspace 継承に変更する
  - @voluntas
- [UPDATE] `examples/wt_server` の Base64 依存を `base64` から `base64ct` に切り替える
  - @voluntas
- [UPDATE] shiguredo-rust 規約に合わせ `.unwrap()` を `.expect(...)` に、手書きの `#[allow]` を `#[expect]` に置き換える
  - @voluntas
- [UPDATE] `examples/wt_server` の `--reject-connect` デモの拒否ステータスを 404 から 405 に変更する (draft-ietf-webtrans-http3-16 Section 3.2)
  - @voluntas
- [UPDATE] CI の GitHub Actions ワークフローから neqo-crypto ビルド用の NSS セットアップステップを削除する (neqo は既に依存から削除済みのため)
  - @voluntas
- [UPDATE] ngtcp2 1.25.90 に合わせて `ngtcp2-sys` の bindings を再生成する
  - @voluntas
- [UPDATE] `s2n-tls` を 0.3.42 に更新する
  - @voluntas
- [UPDATE] PBT を proptest から noprop に置き換え、命令形クロージャと符号化長・空・上限の境界サンプリングで全プロパティテストを書き換える
  - @voluntas
- [FIX] `fuzz/fuzz_targets/fuzz_settings.rs` が `Settings::from_payload` の `Result` 戻り値に追従しておらず fuzz crate がコンパイルできなかった問題を修正する
  - @voluntas
- [FIX] CI の共通 workspace job から `interop/h3` / `interop/wt` を除外し、相互運用テストは macOS 専用 step でのみ実行する
  - @voluntas
- [FIX] `Server` / `ServerWebTransportSession` が 1 クライアントアドレスあたり 1 接続しか保持できず、不正なパケット 1 個や接続単位のエラーでサーバーが停止していた問題を修正する。到着パケットを DCID で接続に振り分け (RFC 9000 Section 5.2)、パケット処理・ストリーム処理・タイムアウトのエラーを接続単位で処理してサーバーループを継続するようにする。併せて `Server::send_response` 等のアドレス指定 API は同一アドレスからの複数接続時に一意に特定できないためエラーを返すようになる
  - @voluntas
