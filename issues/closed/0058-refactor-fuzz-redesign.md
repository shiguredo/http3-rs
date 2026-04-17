# fuzz を再設計・再構築する

Created: 2026-04-17
Completed: 2026-04-17
Model: Opus 4.7

## 背景

現状の fuzz は CLAUDE.md の役割分担ルール (PBT はプロパティ検証、fuzz はパニック安全性) から逸脱しており、PBT と責務が重複している。加えて `fuzz_qpack.rs` が 286 行に肥大化するなど、各ターゲットのスコープが曖昧なまま修正が重ねられている。

当初発生していた「fuzz 実行時にハングする」問題は、`rustup update nightly` でローカル nightly toolchain を 2026-01-27 版 → 2026-04-16 版に更新することで解消した (下記「ハング問題の調査結果」参照)。以後の論点は設計の責務整理に絞られる。

## 現状の問題点

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

- PBT とのスコープ重複により、同じ入力空間を両方で扱う保守コストが発生している
- 既存 11 ターゲットの責務が曖昧なまま修正を重ねると、issue 0057 のような「シグネチャ変更に追従漏れ」系の退行が再発する
- CI に fuzz ビルドが組み込まれていないため、退行検知の役割を果たしていない

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

## ハング問題の調査結果 (2026-04-17)

### 症状

- `cargo fuzz run fuzz_settings -- -max_total_time=30` を実行しても 30 秒で終わらず、CPU を 100% 消費し続ける
- ビルド済みバイナリを `-runs=0` / `-help=1` で起動してもハングする
- `stderr` に libfuzzer の起動ログが一切出力されない

### 切り分け手順

1. `shiguredo_http3` を一切参照しない空の fuzz ターゲット `fuzz_minimal` を追加して試したところ、同様に libfuzzer の起動ログを出さずにハングした
2. バイナリの共有ライブラリ依存を確認すると `@rpath/librustc-nightly_rt.asan.dylib` に依存していた
3. `rustc --version` が `1.95.0-nightly (e96bb7e44 2026-01-27)` と約 3 ヶ月前のもので、古すぎた
4. `rustup update nightly` で `1.97.0-nightly (7af3402cd 2026-04-16)` に更新
5. 更新後に `fuzz_minimal -runs=0` が正常終了し、`fuzz_settings -- -max_total_time=30` も 30 秒で 10,081,077 回実行して正常終了することを確認した

### 結論

古い nightly toolchain (特に ASan ランタイム) が原因。再構築作業では特に対応不要だが、CI 側では nightly を定期的に更新する仕組み (あるいは明示的に `rustup update nightly` を実行するステップ) を入れておくと退行を防げる。

## pending にした理由

以下の設計判断が必要なため、ユーザーの合意を得てから実装に入る。

1. fuzz 対象とするパース関数の粒度・範囲 (上記の 9 個で妥当か、`fuzz_validation` / `fuzz_stream` をどう扱うか)
2. 既存の `Arbitrary` 定義を全て破棄するか、一部を PBT の `Strategy` に転用するか
3. `corpus/` / `artifacts/` ディレクトリを全破棄するか
4. CI に fuzz ビルド (cargo fuzz build) を組み込むか

## 解決方法

2026-04-17 に以下の作業を行い、本 issue で想定していた再設計を完了した。

### 1. ハング問題の解消

ローカル nightly toolchain が 2026-01-27 版と古かったことが原因 (ASan ランタイムの不具合が疑わしい)。`rustup update nightly` で 2026-04-16 版に更新したところ、libfuzzer の起動ログが正常に出力され、全 fuzz ターゲットが動作するようになった。

### 2. fuzz ターゲットの純化

全 11 ターゲットを実行して動作確認した上で、CLAUDE.md の役割分担 (fuzz はパニック安全性のみ) に従いラウンドトリップ等のプロパティ検証を削除した。

- `fuzz_varint` / `fuzz_huffman` / `fuzz_datagram` / `fuzz_frame` / `fuzz_capsule`: `|data: &[u8]|` でパース関数を呼び出すだけの最小形式に統一
- `fuzz_settings`: `Arbitrary` の `Vec<(u64, u64)>` で `SettingsPayload` を構築して `from_payload` に流すだけに変更
- `fuzz_qpack`: 286 行 → 約 65 行。静的 / 動的デコーダーとエンコーダー / デコーダーストリームレシーバーへの任意バイト列注入のみに絞った
- `fuzz_validation` / `fuzz_connection` / `fuzz_stream` / `fuzz_webtransport_session`: もともとパニック検証のみだったのでそのまま残した

### 3. PBT への移管

`fuzz_qpack` から削除した `DynamicRoundtrip` と `BlockedThenUnblocked` に相当するプロパティを `prop_qpack.rs` に追加した。

- `prop_dynamic_encoder_decoder_roundtrip`: 同期された `DynamicEncoder` / `DynamicDecoder` 間でのラウンドトリップ
- `prop_blocked_then_unblocked`: RFC 9204 Section 2.1.2 のブロック → テーブル更新 → アンブロック

### 4. CI への組み込み

ユーザーの判断により、今回は `cargo fuzz build` の CI 組み込みは行わない。

### 5. `corpus/` / `artifacts/` の扱い

`fuzz/.gitignore` で ignore 済みのため、git 管理外。今回の動作確認で生成された corpus/artifacts はローカルのみに残る。

### 動作確認

更新後に全 fuzz ターゲットを 15 秒ずつ実行し、クラッシュなしで終了することを確認した。ラウンドトリップ処理の除去により実行速度が向上している。

| Target | 15 秒実行回数 (refactor 前 → 後) |
| --- | --- |
| fuzz_varint | 10,656,772 → 11,999,783 |
| fuzz_huffman | 117,528 → 786,100 |
| fuzz_datagram | 5,770,294 → 10,584,559 |
| fuzz_frame | 2,837,783 → 6,939,022 |
| fuzz_capsule | 4,640,644 → 10,988,514 |
| fuzz_settings | 4,707,786 → 4,819,526 |
| fuzz_qpack | 508,892 → 1,171,331 |
| fuzz_validation | 2,866,345 → 2,811,328 |
| fuzz_connection | 786,812 → 638,764 |
| fuzz_stream | 1,450,618 → 1,311,722 |
| fuzz_webtransport_session | 1,444,484 → 1,606,799 |

PBT 側で追加した 2 つのプロパティも `cargo test -p pbt --test prop_qpack` で成功することを確認済み。
