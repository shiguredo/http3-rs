# bytes クレートの導入を試験する

Created: 2026-05-07
Model: Opus 4.7

## 背景

現状 `shiguredo_http3` は内部・公開 API ともに `Vec<u8>` および `&[u8]` のみで HTTP/3 ペイロードを扱っている。一方で QUIC エコシステム (s2n-quic, quinn, h3, hyper, tonic) は `bytes::Bytes` / `bytes::BytesMut` / `bytes::Buf` / `bytes::BufMut` を事実上の共通インターフェイスとして採用している。

本 issue は QUIC ライブラリとの相性向上と将来の no_std 化を見据えた地ならしとして、`bytes` クレートを段階的に導入できるかを試験することを目的とする。破壊的変更は許容する。

## 根拠

### QUIC アダプタ層でのコピー削減

- `crates/tokio-s2n-quic` は s2n-quic 由来の `Bytes` を受け取って `Vec<u8>` に詰め替えてから `shiguredo_http3` に渡している。`Bytes` のまま流せれば往復のメモリコピーが消える。
- `crates/tokio-ngtcp2` は nghttp3 から FFI で受け取ったバッファを `Vec<u8>` に複製している。`Bytes::from_owner` で所有権ラッパを作れば、再送・分割のたびに発生する `clone()` が O(1) refcount 操作になる。
- いずれも `tokio-ngtcp2` / `tokio-s2n-quic` の I/O ループにおけるアロケーション回数とコピー量を直接削減できる。

### no_std 親和性

- `bytes` は `default-features = false` で `alloc` のみに依存する形で利用可能。
- 現状 `Vec<u8>` を使っている範囲はすべて `alloc` 前提なので、退化はない。
- パーサ (`varint.rs`, `frame/mod.rs`, `qpack/decoder.rs`) を `Buf` トレイト境界に書き換えると、入力ソースを `&[u8]` / `Bytes` / `BytesMut` / chained buffer などに対して抽象化でき、no_std 化の足場になる。

### 公開 API の自然化

- `Frame::Data`、`StreamEvent::Data`、`HeaderField` の name/value、WebTransport の datagram / stream payload は、いずれもユーザが「受け取って `clone` して別ストリームに転送する」「複数キューに分散させる」といった用途を持つ。`Bytes` であれば cheap clone で済む。
- 利用側 (sora-moqt-rs, examples/wt_server, interop/h3, interop/wt) はすでに `bytes` を直接または間接的に取り回しているため、`Vec<u8>` ↔ `Bytes` の変換コードが消える。

## ゴール

- `shiguredo_http3` 本体および QUIC アダプタ層 (`tokio-ngtcp2`, `tokio-s2n-quic`) で、ペイロードに相当する `Vec<u8>` / `&[u8]` を `Bytes` / `BytesMut` / `Buf` / `BufMut` に置き換える。
- パーサ/シリアライザは `Buf` / `BufMut` トレイト境界で書き換える。
- ヘッダ等の「短く、所有権が短命」な領域は無理に `Bytes` 化せず、`Vec<u8>` のままで構わない。判断基準は本 issue の「適用範囲の判断軸」に従う。
- 既存テスト (単体, PBT, fuzz, interop) がすべて通ること。
- `Vec<u8>` → `Bytes` の置換に伴うアロケーション削減を簡易ベンチで確認する (任意、後続 issue 化可)。

## 非ゴール

- `no_std` 対応そのものの実装はしない。今回はあくまで地ならし。
- `tokio` の取り外しはしない。
- `shiguredo_http11` 側の API 変更はしない。

## 適用範囲の判断軸

`Bytes` 化する / しないの判断は以下の軸で行う:

| 対象 | 判断 | 理由 |
| --- | --- | --- |
| Frame ペイロード (DATA, WT_STREAM など) | `Bytes` 化する | 容量大、転送・分割・clone の対象 |
| StreamEvent / 公開イベント | `Bytes` 化する | 利用側で clone される |
| QUIC アダプタ層の入出力 | `Bytes` 化する | コピー削減効果が直接出る |
| QPACK エンコーダ入力 | `impl AsRef<[u8]>` | 呼出側の自由度を残す |
| QPACK デコーダ出力 (HeaderField) | 検討 | 短いがリクエスト寿命中保持される。要計測 |
| パーサ内部ワーク領域 | `BytesMut` 化する | append/split が cheap になる |
| エラー型に含まれるバイト列 | `Vec<u8>` のまま | サイズ小、所有権単純 |
| WebTransport Capsule の小さなフィールド | `Vec<u8>` のまま | 過剰最適化を避ける |

## 進め方

破壊的変更を許容するため、`develop` ブランチで一括移行する。フェーズは PR 単位ではなくコミット単位で区切ることを想定。

### Phase 1: アダプタ層 (内部のみ、無破壊)

- `crates/tokio-ngtcp2` の I/O ループで nghttp3 からの受信バッファを `Bytes` で扱う。
- `crates/tokio-s2n-quic` で s2n-quic 由来の `Bytes` を `Vec<u8>` に詰め替えている箇所を排除する。
- ここまでは `shiguredo_http3` 本体の API は触らない。

### Phase 2: Frame / Event 層 (公開 API 破壊)

- `Frame::Data { payload: Vec<u8> }` → `Frame::Data { payload: Bytes }`
- `StreamEvent::Data` / `WebTransportEvent` のペイロードを `Bytes` 化
- `Frame::encode` / `Frame::decode` のシグネチャを `Buf` / `BufMut` ベースに変更

### Phase 3: QPACK 層 (公開 API 破壊)

- `qpack::Decoder` の出力 `HeaderField` を `Bytes` 化するか検討 (計測込み)
- `qpack::Encoder` の入力は `impl AsRef<[u8]>` で受ける
- 動的テーブルの内部表現は `Bytes` 化を試す (refcount で共有可能になる)

### Phase 4: 内部パーサの `Buf` / `BufMut` 化 (内部のみ、無破壊)

- `src/varint.rs`、`src/frame/mod.rs`、WebTransport capsule パーサを `Buf` / `BufMut` トレイト境界で書き換える
- これにより no_std 化と入力ソースの多様化に道筋がつく

### Phase 5: テスト・examples・interop 追従

- `pbt/`、`tests/`、`examples/wt_server`、`interop/h3`、`interop/wt` を新 API に追従させる
- PBT の `Strategy` は `Vec<u8>` → `Bytes` への変換を入れる (proptest 側に `Bytes` Strategy はないので `prop::collection::vec` の出力を `Bytes::from` で包む)

## 依存追加

- `bytes = { version = "1", default-features = false }` を workspace dependencies に追加
- `aws-lc-rs`, `tokio`, `s2n-quic` 経由ですでに `bytes` は依存ツリーに入っているため、新規ビルド負荷の増分はほぼゼロ

## 完了条件

- 上記 Phase 1〜5 が完了し、`cargo test --workspace`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo fmt --check`、interop (`interop/h3`, `interop/wt`) が成功する
- `CHANGES.md` の `## develop` セクションに `[CHANGE]` で公開 API 変更を、`[UPDATE]` で内部最適化を記載する
- 簡易ベンチ (任意) で `Vec<u8>` 比のアロケーション/コピーの削減を確認する。確認できなかった場合は後続 issue として残す

## 試験で打ち切る判断基準

以下のいずれかに該当した場合、Phase の途中であっても本試験を打ち切り、`issues/pending/` に移して理由を明記する:

- nghttp3 / s2n-quic 側のバッファ所有権モデルと `Bytes::from_owner` の整合が取れず、unsafe を要求される
- パーサの `Buf` 化により、ゼロコピーで済んでいた箇所が逆にコピーを誘発する
- 公開 API 破壊が利用側 (sora-moqt-rs) で許容できない範囲に及ぶ

いずれの場合も、何が問題で打ち切ったかを issue に追記してから pending に移す。
