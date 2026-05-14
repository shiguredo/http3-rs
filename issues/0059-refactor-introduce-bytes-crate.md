# bytes クレートを shiguredo_http3 に導入する

Created: 2026-05-07
Model: Opus 4.7

## 背景

現状 `shiguredo_http3` は内部・公開 API ともに `Vec<u8>` および `&[u8]` のみで HTTP/3 ペイロードを扱っている。一方で QUIC エコシステム (s2n-quic) は `bytes::Bytes` / `bytes::BytesMut` / `bytes::Buf` / `bytes::BufMut` を事実上の共通インターフェイスとして採用している。

本 issue は s2n-quic アダプタ層 (`crates/tokio-s2n-quic`) との相性向上と将来の no_std 化を見据えた地ならしとして、`bytes` クレートを `shiguredo_http3` コアクレートに導入することを目的とする。破壊的変更は許容する。

## 対象 / 非対象の明確化

- **対象**: `shiguredo_http3` コアクレート (src/) および s2n-quic アダプタ層 (`crates/tokio-s2n-quic`)
- **非対象**: `crates/tokio-ngtcp2` — このクレートは `shiguredo_http3` ではなく nghttp3 C FFI を直接使用する独立した実装系統であり、本 issue のスコープ外。nghttp3 の FFI バッファと `Bytes::from_owner` の所有権モデル整合性は別 issue で扱う

## 関連 issue

- `0082-refactor-event-enum-nesting`: Event 列挙型の WebTransport バリアントをネスト化する。本 issue の Event 内 `Vec<u8>` → `Bytes` 変更と衝突するため、本 issue を先に実施し、0082 は本 issue 完了後に行う

## 根拠

### s2n-quic アダプタ層でのコピー削減

- `crates/tokio-s2n-quic` は s2n-quic 由来の `Bytes` を受け取っているが、`H3InitData` の各フィールド (`connection/mod.rs:438-446`) が `Vec<u8>` のため `Bytes::from()` で変換している。`take_stream_data` の戻り値 (`connection/mod.rs:3287`) も `Option<(Vec<u8>, bool)>` であり、受信パスでも `Bytes` → `Vec<u8>` → `Bytes` の往復コピーが発生している。
- `shiguredo_http3` の公開型でペイロードを `Bytes` 化すれば、s2n-quic アダプタ層の変換呼び出しが削減できる。

### no_std 親和性

- `bytes` は `default-features = false` で `alloc` のみに依存する形で利用可能。
- パーサ (`varint.rs`, `frame/mod.rs`, `qpack/decoder.rs`) を `Buf` トレイト境界に書き換えると、入力ソースを `&[u8]` / `Bytes` / `BytesMut` などに対して抽象化でき、no_std 化の足場になる。

### 公開 API の自然化

- `Event::Data`、`HeaderField` の name/value、WebTransport の datagram / stream payload は、いずれもユーザが「受け取って `clone` して別ストリームに転送する」「複数キューに分散させる」といった用途を持つ。`Bytes` であれば cheap clone で済む。
- 利用側 (examples/wt_server, interop/h3, interop/wt) はすでに `bytes` を直接取り回しているため、`Vec<u8>` ↔ `Bytes` の変換コードが消える。

## ゴール

- `shiguredo_http3` 本体および s2n-quic アダプタ層 (`tokio-s2n-quic`) で、ペイロードに相当する `Vec<u8>` を `Bytes` に置き換える。
- パーサ/シリアライザは `Buf` / `BufMut` トレイト境界で書き換える。
- ヘッダ等の「短く、所有権が短命」な領域は無理に `Bytes` 化せず、`Vec<u8>` のままで構わない。
- 既存テスト (単体, PBT, fuzz, interop) がすべて通ること。
- ベンチマークでアロケーション/コピーの削減効果を確認する。

## 非ゴール

- `no_std` 対応そのものの実装はしない。今回はあくまで地ならし。
- `crates/tokio-ngtcp2` への `bytes` 導入 — nghttp3 C FFI のバッファ所有権モデルとの整合性検証が必要なため別 issue とする。

## 適用範囲の判断軸

`Bytes` 化する / しないの判断は以下の軸で行う:

| 対象 | 判断 | 理由 |
| --- | --- | --- |
| Frame ペイロード (DATA, HEADERS, Unknown) | `Bytes` 化する | 容量大、転送・分割・clone の対象 |
| Event 内のデータフィールド (`data`, `payload`) | `Bytes` 化する | 利用側で clone される |
| `HeaderField` の name/value (Event::Header) | `Bytes` 化する | リクエスト寿命中保持される |
| `H3InitData` の control/encoder/decoder data | `Bytes` 化する | s2n-quic アダプタで `Bytes::from()` 変換発生 |
| `Connection::take_stream_data` 戻り値 | `Bytes` 化する | アダプタ層のコピー削減に直結 |
| `Connection::send_datagram` 戻り値 | `Bytes` 化する | エンコード済みデータグラム、公開 API |
| `Connection::get_stream_data` 戻り値 (`&[u8]`) | 変更しない | `BytesMut` からの借用と `advance` の競合を避ける。部分スライスによる元バッファ保持問題も回避 |
| `Datagram::payload` (webtransport/datagram.rs) | `Bytes` 化する | データグラムペイロード、clone の対象 |
| `RawReceivedData` 全バリアント (Headers, Trailers, Data) | `Bytes` 化する | Frame→RawReceivedData のフローで to_vec を避ける |
| `RequestStream` の QPACK ブロックバッファ (`qpack_blocked_header`) | `Bytes` 化する | HeadersPayload が Bytes になるため追随必須 |
| `ReceivedData::Data(Vec<u8>)` | `Bytes` 化する | 上位層へのボディ配送 |
| `EncoderInstruction` の name/value フィールド | `Bytes` 化する | BytesMut からのゼロコピー抽出のため |
| QPACK エンコーダ入力 | `impl AsRef<[u8]>` | 呼出側の自由度を残す |
| QPACK デコーダ出力 (`DecodedHeader`) | `Bytes` 化する | Event::Header の Bytes 化に合わせる |
| 動的テーブル内部表現 (`DynamicEntry`) | `Bytes` 化を試みる | refcount 共有の効果を検証。Phase 3 完了時に計測し効果がなければ `Vec<u8>` に留める。pass/fail 基準: アロケーション回数が 10% 以上削減されること |
| 動的テーブル insert メソッドの引数 | `impl AsRef<[u8]>` | `Bytes` → `Vec<u8>` コピーを回避するため |
| `StreamHeader` の encode/decode バッファ | encode: `&mut Vec<u8>` → `&mut impl BufMut`、decode: `&[u8]` → `&impl Buf` | パーサ内部ワーク領域 |
| Connection 内部バッファ (`capsule_buf`, `pending_*_streams`, `BufferedStreamEntry::data`, `buffered_datagrams`) | `BytesMut` 化する | append/split/advance が cheap |
| `SendBuffer` / `RecvBuffer` (stream/mod.rs) | `BytesMut` 化する | 全ストリームの送受信バッファ基盤 |
| QPACK エンコーダ/デコーダストリーム内部バッファ | `BytesMut` 化する | varint 多用のワーク領域 |
| `Capsule::Unknown { payload: Vec<u8> }` | `Vec<u8>` のまま | Capsule は小さいフィールド中心のため過剰最適化を避ける |
| エラー型に含まれるバイト列 | `Vec<u8>` のまま | サイズ小、所有権単純 |
| `Event::ConnectionError { reason: String }` | 変更しない | String は Vec<u8> と無関係 |

## 進め方

破壊的変更を許容するため、`develop` ブランチで一括移行する。フェーズは PR 単位ではなくコミット単位で区切ることを想定。

### Phase 1: コアクレート依存追加と `varint` / `frame` の `Buf` / `BufMut` 化 (内部 API 変更)

Phase 1 の varint の Buf/BufMut 化は全フェーズの前提となる。Phase 1 の完了なしに Phase 2 以降の個別テストは不可能。

- `bytes = { version = "1", default-features = false }` を workspace dependencies に追加。コアクレートの `Cargo.toml` に `bytes.workspace = true` を追加
- `src/varint.rs`:
  - `encode(&mut [u8], value)` → `encode(buf: &mut impl BufMut, value) -> Result<usize, EncodeError>`
    - 関数内部で `buf.remaining_mut()` をチェックし、不足なら `BufferTooShort` エラーを返す（ `put_slice` のパニック回避）
  - `decode(&[u8])` → `decode(buf: &impl Buf) -> Result<(u64, usize), DecodeError>`
    - `buf.chunk()` や `buf.remaining()` はいずれも `&self` で取れるため `&mut` 不要。関数内部では `advance` を呼び出さず、戻り値の `usize` は「消費すべきバイト数」とする。呼び出し側が `advance` の責任を持つ。この設計により、frame decoder 側のオフセット計算と peek パターンを維持したまま `Buf` 化できる
  - `encode_into_vec` / `try_encode_into_vec` → `encode_to_bufmut` / `try_encode_to_bufmut` にリネームし、`BytesMut` を含む任意の `BufMut` を受け取る
- `src/frame/encoder.rs`: `encode_frame` / `encode_frame_header` (`&mut [u8]` 引数) を `BufMut` ベースに変更
- `src/frame/decoder.rs`: `decode_frame` / `decode_frame_header` (`&[u8]` 引数) を `Buf` ベースに変更。`process_raw()` (stream/request.rs) のパターンが `peek()` → decode → `consume()` から、`Buf::chunk()` → decode → 呼出側 `advance()` に変わる
  - エラー時は advance しない。部分 decode の途中でエラーが発生した場合、バッファ位置はデコード開始前に留まる。 DecodeHeader が成功していればヘッダー分だけ advance している
- `src/qpack/encoder_stream.rs`, `src/qpack/decoder_stream.rs`, `src/webtransport/stream.rs`, `src/webtransport/datagram.rs`, `src/webtransport/capsule.rs` など varint の呼び出し元全ファイルを追随させる

### Phase 2: Frame / Event / H3InitData 層 (公開 API 破壊)

- `DataPayload { data: Vec<u8> }` → `DataPayload { data: Bytes }`
- `HeadersPayload { encoded_field_section: Vec<u8> }` → `HeadersPayload { encoded_field_section: Bytes }`
- `Frame::Unknown { payload: Vec<u8> }` → `Frame::Unknown { payload: Bytes }`
- `Event::Data { data: Vec<u8> }` → `Bytes`
- `Event::Header { name: Vec<u8>, value: Vec<u8> }` → `Bytes`
- `Event::WebTransportBidiStreamData { data: Vec<u8> }` → `Bytes`
- `Event::WebTransportUniStreamData { data: Vec<u8> }` → `Bytes`
- `Event::WebTransportDatagram { payload: Vec<u8> }` → `Bytes`
- `H3InitData` の `control_data`, `encoder_data`, `decoder_data` → `Bytes`
- `Connection::take_stream_data` 戻り値 → `Option<(Bytes, bool)>`
- `Connection::send_datagram` 戻り値 → `Result<Bytes, Error>`。内部の `payload.to_vec()` → `Bytes::copy_from_slice(payload)` に変更
- `Connection::send_body` 内部の `DataPayload::new(data.to_vec())` を `Bytes::copy_from_slice(data)` に変更
- `RawReceivedData::Headers(Vec<u8>)`, `RawReceivedData::Trailers(Vec<u8>)`, `RawReceivedData::Data(Vec<u8>)` → すべて `Bytes`
- `RequestStream::qpack_blocked_header: Option<(Vec<u8>, bool)>` → `Option<(Bytes, bool)>`、`take_qpack_blocked_header` 戻り値も追随
- `ReceivedData::Data(Vec<u8>)` → `Bytes`
- `Datagram::payload: Vec<u8>` → `Bytes`。`Datagram::new` の引数型も追随、`Datagram::encode` のバッファ引数も追随（ `&mut Vec<u8>` → `&mut impl BufMut` ）
- `StreamHeader` encode メソッドのバッファ引数を `&mut impl BufMut` に変更（decode 側は Phase 1 の `varint::decode` Buf 化で追随）

### Phase 3: QPACK 層 (公開 API 破壊)

- `Header::new` のシグネチャを `name: impl AsRef<[u8]>, value: impl AsRef<[u8]>` に変更
- `DecodedHeader` の `name`, `value` を `Bytes` 化
- `DynamicEntry` の `name`, `value` を `Bytes` 化
- `DynamicTable::insert` / `insert_with_name_ref` の引数を `impl AsRef<[u8]>` に変更
- `DynamicEncoder::insert` / `insert_with_static_name_ref` / `insert_with_dynamic_name_ref` および `DynamicDecoder::insert` も追随
- `EncoderInstruction::InsertWithNameReference { value: Vec<u8> }` 等を `Bytes` 化し、受信側で `BytesMut` からゼロコピー抽出できるようにする
- QPACK エンコーダ/デコーダストリームの内部バッファを `BytesMut` 化
- 動的テーブル内部表現の `Bytes` 化は、本 Phase 完了時にアロケーション回数を計測して効果を確認する。効果がなければ `Vec<u8>` にロールバックする

### Phase 4: Connection 内部バッファの `BytesMut` 化 (内部のみ)

- `Connection` 内部の `capsule_buf: Vec<u8>` → `BytesMut`
- `pending_uni_streams`, `pending_wt_uni_streams`, `pending_wt_bidi_streams`, `pending_bidi_dispatch` → すべて `BytesMut`
- `BufferedStreamEntry::data: Vec<u8>` → `BytesMut`。セッション確立時に `.freeze()` で `Bytes` に変換してイベント発火する
- `WtSession::buffered_datagrams: Vec<Vec<u8>>` → `Vec<Bytes>`。`buffer_datagram` の引数型も追随
- `SendBuffer` / `RecvBuffer` (stream/mod.rs) を `BytesMut` ベースに変更
  - `Vec::drain` → `BytesMut::advance` に置き換える。全データが消費されたタイミング（`consumed == data.len()`）で `truncate(0)` を呼びメモリを解放する
  - `get_stream_data` (`Option<(&[u8], bool)>`) は変更しない。戻り値の `&[u8]` 参照が生きている間は mutable 操作を避ける設計を維持する

### Phase 5: アダプタ層とテスト・examples・interop・fuzz 追従

- `crates/tokio-s2n-quic` の追従:
  - `connection_state.rs`: `drain_qpack_data`、`get_stream_data`、`init_h3_streams` 等を新 API に追従
  - `h3/server.rs`: `H3Request.headers`, `H3Response.headers` の型を `Vec<(Bytes, Bytes)>` に変更。ボディも `Bytes` 化。`send_response` の `Bytes::from(data)` 等を削減
  - `h3/client.rs`: `H3ClientRequest.body`, `H3ClientResponse.body` を `Bytes` に変更。リクエスト/レスポンス構築時の変換箇所を修正
  - webtransport session ラッパーファイルも含む。`grep Vec<u8>` で検出して漏れなく追従する
- `pbt/`、`tests/`、`examples/wt_server`、`interop/h3`、`interop/wt`、`fuzz/` を新 API に追従
- PBT 対応:
  - `Bytes` は `proptest::arbitrary::Arbitrary` を実装していないため、`#[derive(Arbitrary)]` を使っている構造体（`Frame`, `Event` 等）は手動 `Strategy` 実装に切り替える
  - Strategy では `prop::collection::vec(..)` の出力を `Bytes::from` で包む

## ベンチマークによる効果検証

Phase 5 完了後、s2n-quic アダプタ層で `Bytes` → `Vec<u8>` → `Bytes` の往復コピーが削減されたことを定量化する。
効果が確認できた場合のみ issue を完了とする。

## 依存追加

- `bytes = { version = "1", default-features = false }` を workspace dependencies に追加
- `shiguredo_http3` コアクレート (`Cargo.toml`) に `bytes.workspace = true` を追加

## 完了条件

- 上記 Phase 1〜5 が完了し、`cargo test --workspace`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo fmt --check`、interop (`interop/h3`, `interop/wt`) が成功する
- `CHANGES.md` の `## develop` セクションに `[CHANGE]` で `shiguredo_http3` の公開 API 変更を、`[UPDATE]` で `tokio-s2n-quic` の内部最適化を記載する

## 試験で打ち切る判断基準

以下のいずれかに該当した場合、Phase の途中であっても打ち切り、`issues/pending/` に移して理由を明記する。打ち切ったコードは対象ブランチごと破棄する:

- s2n-quic 側のバッファ所有権モデルと `Bytes` の整合が取れず、unsafe を要求される
- パーサの `Buf` 化により、ゼロコピーで済んでいた箇所が逆にコピーを誘発する
- 公開 API 破壊が利用側で許容できない範囲に及ぶ
- ベンチマークで効果が確認できなかった

いずれの場合も、何が問題で打ち切ったかを issue に追記してから pending に移す。
