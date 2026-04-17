# fuzz を再設計・再構築する

Created: 2026-04-17
Model: Opus 4.7

## 背景

`cargo fuzz run` で fuzz ターゲットを実行するとハングし、libfuzzer の起動ログすら `stderr` に出ない状態になっている。ビルド済みバイナリを `-runs=0` で直接起動してもハングするため、fuzzing が実行できない状態が続いている (issue 0057 のコンパイル修正後も残存)。これに加えて、現状の fuzz は CLAUDE.md の役割分担ルール (PBT はプロパティ検証、fuzz はパニック安全性) から逸脱しており、PBT と責務が重複している。

## 現状の問題点

### ハング問題

- `cargo fuzz run fuzz_settings -- -max_total_time=30` を実行しても 30 秒で終わらず、CPU を 100% 消費し続ける
- ビルド済みバイナリを `-runs=0` で直接起動してもハングする
- `stderr` に libfuzzer の起動ログが一切出力されない
- 原因未特定 (aws-lc-sys のグローバル初期化 × sanitizer の相性、あるいは libfuzzer-sys / nightly toolchain / macOS aarch64 の組み合わせが疑わしい)

### 責務重複問題

- `pbt/tests/` の 10 ファイルと `fuzz/fuzz_targets/` の 11 ファイルがほぼ 1:1 対応しており、似た入力空間を両者で扱っている
- `fuzz_settings.rs` は `Roundtrip` variant を持ち、ビルダー構築 → エンコード → デコードのプロパティ検証まで fuzz で行っている (本来 PBT の役割)
- `fuzz_qpack.rs` は 286 行と肥大化しており、`FuzzInput` enum の各 variant で個別のプロパティを検証している
- CLAUDE.md:「PBT に『任意入力でパニックしないことだけを検証するテスト』を書かない (fuzzing の役割)」の裏返しとして、「fuzz にプロパティ検証を書かない」も徹底されるべきだが、現状は逆になっている

### 現状の fuzz ターゲット一覧

| Target | 行数 | PBT 対応 |
| --- | --- | --- |
| fuzz_qpack | 286 | prop_qpack |
| fuzz_stream | 96 | prop_stream |
| fuzz_capsule | 93 | prop_capsule |
| fuzz_frame | 86 | prop_frame |
| fuzz_webtransport_session | 80 | prop_webtransport |
| fuzz_settings | 77 | prop_settings |
| fuzz_validation | 76 | prop_validation |
| fuzz_connection | 72 | prop_connection |
| fuzz_datagram | 64 | prop_datagram |
| fuzz_varint | 30 | prop_varint |
| fuzz_huffman | 26 | (なし) |

## 根拠

- 現状 fuzz は実行できない状態にあり、CI にも組み込まれていないため、退行検知の役割を果たしていない
- PBT とのスコープ重複により、同じ入力空間を両方で扱う保守コストが発生している
- 既存 11 ターゲットの責務が曖昧なまま修正を重ねると、issue 0057 のような「シグネチャ変更に追従漏れ」系の退行が再発する

## 再設計方針 (案)

### 基本方針

1. fuzz ターゲットは「任意バイト列 → パース関数呼び出し → パニックしないこと」だけを検証する形式に統一する

   ```rust
   fuzz_target!(|data: &[u8]| {
       let _ = shiguredo_http3::Varint::decode(data);
   });
   ```

2. `Arbitrary` による構造化入力の生成・ラウンドトリップ検証・プロパティ検証は全て PBT に集約する
3. 各 fuzz ターゲットはパース関数ごとに 1 ファイル・数十行以内に収める
4. 現状の `fuzz_*.rs` にあるラウンドトリップ等のプロパティ検証は、PBT 側に未実装なものだけを `prop_*.rs` に移す

### ターゲット選定 (要合意)

パース関数入口となる以下を fuzz 対象候補とする。

- `Varint::decode`
- `Frame::decode`
- `huffman::decode`
- `qpack::decoder::Decoder::decode`
- `Capsule::decode`
- `Connection` の受信データ入口
- `WebTransportSession` の受信データ入口
- `Datagram::decode`
- `Settings::from_payload`

`fuzz_validation` / `fuzz_stream` は対象関数が不明瞭なため、再検討する。

### ハング問題の調査

再構築作業に入る前に以下を調査する。

1. `shiguredo_http3` に依存しない最小 fuzz ターゲット (例: `fn(data: &[u8]) { let _ = core::str::from_utf8(data); }`) でハングが再現するか確認する
2. `shiguredo_http3` を依存に含めた最小 fuzz ターゲットで再現するか確認する
3. 再現した場合は `aws-lc-sys` のビルドフラグ (sanitizer 経由でのビルド) を疑う
4. 再現しない場合は、既存 fuzz ターゲット内のコード (例えば `Arbitrary` 実装) が起動時に重い処理をしていないか確認する

## pending にした理由

以下の設計判断が必要なため、ユーザーの合意を得てから実装に入る。

1. ハング原因の調査を再構築の前に行うか、並行で行うか
2. fuzz 対象とするパース関数の粒度・範囲
3. 既存の `Arbitrary` 定義を全て破棄するか、一部を PBT の `Strategy` に転用するか
4. `corpus/` / `artifacts/` ディレクトリを全破棄するか
5. CI に fuzz ビルド (cargo fuzz build) を組み込むか
