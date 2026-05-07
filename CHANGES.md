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

- [UPDATE] tokio-s2n-quic の HTTP/3・WebTransport receive ループで s2n-quic 由来の `Bytes` をそのまま `Connection::feed_stream_bytes` に流し、`Vec<u8>` への詰め替えを排除する (issue 0059)
  - @voluntas
- [UPDATE] tokio-s2n-quic の QPACK 送信チャンネルを `(u64, Vec<u8>)` から `(u64, Bytes)` に変更し、send 経路の冗長な `Bytes::from(...)` 変換を排除する (issue 0059)
  - @voluntas
- [UPDATE] `frame::decode_frame` を `BytesMut::split_to(payload_len).freeze()` による zero-copy 切り出しに変更し、`DATA` / `HEADERS` / `Unknown` ペイロードのコピーを排除する (issue 0059)
  - @voluntas
- [UPDATE] `Connection` 内部 dispatch (`handle_unidirectional_stream` / `handle_wt_bidi_stream` / `dispatch_client_bidi_stream` 等) を `&Bytes` 引き回しに変更し、WebTransport bidi/uni ストリームの Event 発行を refcount clone と `Bytes::slice(...)` による zero-copy 切り出しで行う (issue 0059)
  - @voluntas
- [UPDATE] `RecvBuffer` を `BytesMut` ベースに変更し、`Buf::advance` でゼロコピー消費を行う (issue 0059)
  - @voluntas
- [UPDATE] `qpack::EncoderStream` / `EncoderStreamReceiver` の send/recv バッファを `BytesMut` に変更し、内部の `encode_integer` / `encode_string` / `encode_string_with_prefix` を `&mut BytesMut` に揃える (issue 0059)
  - @voluntas
- [UPDATE] `qpack::DecoderStream` / `DecoderStreamReceiver` の send/recv バッファを `BytesMut` に変更し、内部の `encode_integer` を `&mut BytesMut` に揃える (issue 0059)
  - @voluntas
- [UPDATE] `stream::SendBuffer.data` を `BytesMut` に変更し、`consumed` オフセットを廃止して `Buf::advance` でゼロコピー消費する (`RecvBuffer` と対称化) (issue 0059)
  - @voluntas
- [ADD] `bytes` クレートを workspace dependencies に追加する (`default-features = false`、no_std 利用に備える) (issue 0059)
  - @voluntas
- [ADD] `Connection::feed_stream_bytes(stream_id, Bytes, fin)` を追加する (zero-copy 用、既存の `feed_stream(&[u8])` も互換維持) (issue 0059)
  - @voluntas
- [ADD] `qpack::Header::from_bytes(Bytes, Bytes)` と tokio-s2n-quic の `H3Request` / `H3Response` / `H3ClientRequest` / `H3ClientResponse` に `header_bytes` / `body_bytes` を追加する (issue 0059)
  - @voluntas
- [CHANGE] `Event::Data.data` / `Event::Header.{name,value}` / `Event::WebTransportBidiStreamData.data` / `Event::WebTransportUniStreamData.data` / `Event::WebTransportDatagram.payload` を `Vec<u8>` から `Bytes` に変更する (issue 0059)
  - @voluntas
- [CHANGE] `Frame::Data.data` / `Frame::Headers.encoded_field_section` / `Frame::Unknown.payload` を `Vec<u8>` から `Bytes` に変更する (issue 0059)
  - @voluntas
- [CHANGE] `qpack::Header` / `qpack::DecodedHeader` / `qpack::DynamicEntry` / `qpack::EncoderInstruction` の name/value および `decode_string` の戻り値を `Vec<u8>` から `Bytes` に変更する (issue 0059)
  - @voluntas
- [CHANGE] `H3InitData.{control_data,encoder_data,decoder_data}` / `Connection::take_stream_data` の戻り値 / `Connection::send_datagram` の戻り値を `Vec<u8>` から `Bytes` に変更する (issue 0059)
  - @voluntas
- [CHANGE] `webtransport::Datagram.payload` / `webtransport::Session::buffer_datagram` の引数型 / `webtransport::Session::take_buffered_datagrams` の戻り値を `Vec<u8>` から `Bytes` に変更する (issue 0059)
  - @voluntas
- [CHANGE] `stream::request::RawReceivedData::Data` / `Headers` / `Trailers` および `RequestStream::set_qpack_blocked` / `take_qpack_blocked_header` を `Vec<u8>` から `Bytes` に変更する (issue 0059)
  - @voluntas
- [CHANGE] tokio-s2n-quic の `WtRecvStream::recv()` / `WtBiStream::recv()` / `WtSession::recv()` の戻り値および `H3Request` / `H3Response` / `H3ClientRequest` / `H3ClientResponse` の headers/body を `Vec<u8>` から `Bytes` に変更する (issue 0059)
  - @voluntas
- [CHANGE] 未使用の `stream::request::ReceivedData` enum を削除する (`RawReceivedData` のみ使用されていたため dead code) (issue 0059)
  - @voluntas

### misc

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
