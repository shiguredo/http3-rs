# fuzz_settings が Settings::from_payload の Result 戻り値に追従していない

Created: 2026-04-17
Model: Opus 4.7

## 背景

`src/settings.rs` の `Settings::from_payload` は `Result<Self, Error>` を返すよう変更されたが、`fuzz/fuzz_targets/fuzz_settings.rs` のラウンドトリップテストが `Result` を unwrap せず直接フィールドアクセスしている。

## 再現手順

```bash
cd fuzz && cargo check
```

以下のコンパイルエラー (E0609) が 5 件発生する。

- `fuzz_targets/fuzz_settings.rs:70` — `decoded.qpack_max_table_capacity`
- `fuzz_targets/fuzz_settings.rs:71` — `decoded.max_field_section_size`
- `fuzz_targets/fuzz_settings.rs:72` — `decoded.qpack_blocked_streams`
- `fuzz_targets/fuzz_settings.rs:73` — `decoded.enable_connect_protocol`
- `fuzz_targets/fuzz_settings.rs:74` — `decoded.h3_datagram`

## 根拠

fuzz crate がコンパイルできず、`fuzz_settings` を含む fuzzing 全体が停止している。PBT では扱わない「任意入力に対するパニック安全性」を検証する fuzzing は、継続的に実行可能でなければ意味をなさない。CI で fuzz のビルドを走らせれば検知できた退行である。

## 修正方針

ラウンドトリップは成功するべきシナリオなので、`Settings::from_payload` の戻り値を `expect("roundtrip must succeed")` で unwrap したうえでフィールドアクセスする。
