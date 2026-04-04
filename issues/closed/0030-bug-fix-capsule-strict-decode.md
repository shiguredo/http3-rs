# Capsule パーサの余剰バイト拒否とエラー型分離

Created: 2026-04-06
Completed: 2026-04-06
Model: Opus 4.6

## 優先度

P1

## 概要

`webtransport/capsule.rs` の Capsule パーサが RFC 9297 の「payload は定義されたフィールドだけをちょうど含まなければならない」を守っておらず、`WT_MAX_DATA` / `WT_MAX_STREAMS` (Bidi/Uni) / `WT_DATA_BLOCKED` / `WT_STREAMS_BLOCKED` (Bidi/Uni) の 6 ケースで、varint を 1 個読むだけで余剰バイトを無視し成功を返している。また `decode_payload` が `Option<Self>` を返す設計のため、呼び出し側 (`connection/mod.rs:1476`) が malformed と incomplete を区別できず、完全受信済みの不正 Capsule でも追加 DATA / FIN まで保留してしまう。

## 根拠

- RFC 9297 Section 3.2: Capsule payload は定義されたフィールドだけを exactly 含まなければならない
- RFC 9297 Section 3.2: malformed / incomplete message は HTTP message 側のエラー処理に従う
- HTTP/3: malformed は `H3_MESSAGE_ERROR`（`WT_SESSION_GONE` は session 終了後の関連 stream 中断用であり、パース失敗の一次エラーには使わない）
- `webtransport/capsule.rs:304` 以降: varint 1 個を読むだけで `consumed == payload.len()` を検証していない
- `webtransport/capsule.rs:254` / `connection/mod.rs:1476`: `None` 畳み込みにより malformed と incomplete が区別できない

## 修正方針

1. `Capsule::decode` のシグネチャ変更（破壊的変更）:
   - `fn decode(buf: &[u8]) -> Result<Option<(Self, usize)>, CapsuleError>`
   - `Ok(None)`: バッファ不足 (incomplete)
   - `Ok(Some(_))`: 正常デコード
   - `Err(_)`: malformed

2. `decode_payload` を `Result<Self, CapsuleError>` に変更。length-framed の内側では payload は完全なので、varint 不足も malformed 扱い。

3. 以下の 6 ケースで余剰バイト拒否を追加:
   - `WT_MAX_DATA`
   - `WT_MAX_STREAMS_BIDI`
   - `WT_MAX_STREAMS_UNI`
   - `WT_DATA_BLOCKED`
   - `WT_STREAMS_BLOCKED_BIDI`
   - `WT_STREAMS_BLOCKED_UNI`
   ```rust
   let (value, consumed) = decode_varint(payload).ok_or(CapsuleError::Malformed)?;
   if consumed != payload.len() {
       return Err(CapsuleError::Malformed);
   }
   ```

4. 呼び出し側 `connection/mod.rs:1476` では `Err(_)` を `Error::StreamError(ErrorCode::MessageError)` (H3_MESSAGE_ERROR) にマッピング。`WEBTRANSPORT_SESSION_GONE` ではない。

## 影響

- 破壊的変更: `Capsule::decode` 戻り型、`CHANGES.md` に `[CHANGE]`
- テスト: 6 ケース + `CLOSE_SESSION` / `DRAIN_SESSION` に対する余剰バイト拒否の単体テスト、PBT のラウンドトリップを維持

## 解決方法

`src/webtransport/capsule.rs`:
- `CapsuleDecodeError::Malformed` エラー型を新設
- `Capsule::decode` の戻り型を `Result<Option<(Self, usize)>, CapsuleDecodeError>` に変更
  - `Ok(Some(_))`: 正常デコード
  - `Ok(None)`: incomplete (バッファ不足)
  - `Err(Malformed)`: 受信済みバイトが malformed
- `decode_payload` を `Result<Self, CapsuleDecodeError>` に変更
- `decode_exact_varint()` ヘルパーを追加し、payload 全体を exactly 消費することを保証
- `WT_MAX_DATA` / `WT_MAX_STREAMS` (Bidi/Uni) / `WT_DATA_BLOCKED` / `WT_STREAMS_BLOCKED`
  (Bidi/Uni) の 6 ケースで `decode_exact_varint` を使用し、余剰バイトを拒否
- `CloseSession` / `DrainSession` も同様に `Err` を返すよう変更

`src/connection/mod.rs` の WT Capsule 処理:
- `Err(_)` を `Error::StreamError(ErrorCode::MessageError)` (H3_MESSAGE_ERROR) に
  マッピング (RFC 9297 Section 3.2 の malformed HTTP message エラー処理)

呼び出し側の追従:
- `pbt/tests/prop_capsule.rs`, `pbt/tests/prop_webtransport.rs`,
  `fuzz/fuzz_targets/fuzz_capsule.rs`, `fuzz/fuzz_targets/fuzz_webtransport_session.rs`,
  `crates/tokio-ngtcp2/tests/webtransport_h3_integration_e2e.rs` の
  `Capsule::decode` 呼び出しを新しいシグネチャに合わせて更新

全テスト (lib 418 + integration 含む) と PBT 79 件が通過。
