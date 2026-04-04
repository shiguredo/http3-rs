# Sans I/O 公開 API から panic を撤去する

Created: 2026-04-06
Completed: 2026-04-06
Model: Opus 4.6

## 優先度

P2

## 概要

`shiguredo_http3` を Sans I/O クレートとして公開しているにもかかわらず、公開 API に panic 経路が残っている。具体的には `webtransport::Datagram::new()` が不正 `session_id` で panic し、`varint` モジュールの一部公開関数も内部不変条件違反で panic する。Sans I/O 境界では不正入力に対して `Result` を返し、呼び出し側に判断を委ねるべき。

## 根拠

- `src/webtransport/datagram.rs:45` `Datagram::new()`: `assert!(session_id.is_multiple_of(4), ...)` で panic
- `src/varint.rs:30`, `src/varint.rs:49`: `Result` 版 API が別にあるにもかかわらず panic 経路を公開している
- Sans I/O の原則: ライブラリ境界では panic を公開せず、エラーは値で返す

## 修正方針

1. `Datagram::new(session_id: u64, payload: Vec<u8>) -> Result<Self, Error>` に変更（破壊的変更）。`session_id % 4 != 0` は `Error::InvalidSessionId`（新設）で返す。

2. `varint` モジュールは公開 API と内部 helper を分離する:
   - 公開 API: `try_encoded_len` / `try_encode_into_vec` のように `Result` を返す形に統一
   - 内部 private helper: 既知不変条件前提の軽量版を残し、呼び出し箇所にコメントで不変条件を明記
   - 公開面からの panic 経路を削除

3. `Datagram::new` の呼び出し側 (wrapper / tests / examples) をすべて追従させる。

## 影響

- 破壊的変更: `CHANGES.md` に `[CHANGE]`
- テスト: 不正 `session_id` で `Err` が返ることの単体テスト、正常系の PBT を維持

## 解決方法

`src/webtransport/datagram.rs`:
- `DatagramError::InvalidSessionId` 型を新設
- `Datagram::new()` を `Result<Self, DatagramError>` 返却に変更 (panic 撤去)

`src/varint.rs`:
- 公開 API として `try_encoded_len` / `try_encode_into_vec` を追加 (Result 返却)
- 既存の `encoded_len` / `encode_into_vec` は呼び出し側が不変条件 (value <= MAX_VALUE)
  を保証する内部向けヘルパーとしてドキュメント更新 (panic 経路は残す)

呼び出し側の追従:
- `src/connection/mod.rs`: `Datagram::new` の戻り値を `.map_err` で
  `Error::ConnectionError(ErrorCode::InternalError)` に変換
- `pbt/tests/prop_datagram.rs`, `pbt/tests/prop_webtransport.rs`,
  `crates/tokio-ngtcp2/tests/webtransport_h3_integration_e2e.rs`,
  `fuzz/fuzz_targets/fuzz_datagram.rs`: `Datagram::new(...).unwrap()` に更新

全テスト (lib 418 + PBT 79 + integration + その他) が通過。
