# `Capsule::decode_payload` の `length as usize` キャスト後の算術オーバーフローを修正する

- Priority: High
- Created: 2026-06-15
- Model: Opus 4.7
- Branch: feature/fix-capsule-decode-payload-usize-overflow
- Polished: 2026-07-21

## 目的

`src/webtransport/capsule.rs:317-320` で VarInt から取得した `length: u64` を `as usize` で素朴に切り詰めてから `offset + length` の境界判定に使っている。32-bit ターゲット (将来の wasm32 等) で `length` が切り詰められて境界判定が緩み、過小なバイト列でも `Some(...)` を返してしまう。さらに 64-bit でも `offset + length` がオーバーフローし得る。`usize::try_from` + `checked_add` で安全化する。

## 優先度根拠

High。Sans I/O のデコーダーは任意のピアからの入力を受ける前提なので、境界判定がプラットフォーム依存に緩むのはセキュリティ・堅牢性の両面で問題。AGENTS.md「性能より堅牢性を優先」原則に直接抵触する。

## 現状

`src/webtransport/capsule.rs:317-320` 前後:

```rust
let length = length as usize;
if offset + length > buf.len() {
    return Ok(None);
}
```

- `length` は `u64` で VarInt 範囲 (`2^62 - 1`) まで取りうる
- 32-bit ターゲットでは `length as usize` で上位ビットが切り捨てられ、巨大値が小さな usize に化ける
- 64-bit でも `offset.checked_add(length)` でないとオーバーフローで判定が誤動作する

加えて `src/webtransport/capsule.rs:212` `encode_as_data_frame` の `capsule_bytes.len() as u64` も VarInt 上限 (`2^62 - 1`) を超えると `encode_varint` で panic する経路があり (関連リスク)、こちらも合わせて見直しが妥当。

## 設計方針

- `usize::try_from(length).ok()?` で u64 → usize の安全変換を行う (失敗時は `Ok(None)` で「不完全」と扱う)
- `offset.checked_add(length).ok_or(...)?` で境界判定を行う
- 同種のキャスト・加算がモジュール内に複数存在するため、ヘルパー (`fn varint_to_usize(length: u64) -> Option<usize>`) を導入してパターンを統一する
- 32-bit 環境で `length > usize::MAX as u64` の場合は「以後デコード不能」として errored Result を返す方が安全か検討する (Sans I/O の API 契約として、不完全 vs エラーをどう扱うかをモジュールの既存方針に合わせる)

## 完了条件

- `length as usize` の素朴キャストが廃止される
- `offset + length` 等の算術が `checked_add` 化される
- fuzz_target で任意入力で panic / wrap しないことを確認 (既存 `fuzz/fuzz_targets/fuzz_capsule.rs` がカバーするはず)
- PBT で「任意の `length` 値で `decode_payload` がパニックせず Some/None/Err のいずれかを返す」プロパティを検証
- `make fmt && make clippy && make check` が全て通る

## 解決方法

```rust
let Some(length) = usize::try_from(length).ok() else {
    return Err(CapsuleDecodeError::Malformed);
};
let Some(end) = offset.checked_add(length) else {
    return Err(CapsuleDecodeError::Malformed);
};
if end > buf.len() {
    return Ok(None);
}
```

`encode_as_data_frame` 側もペイロード長が VarInt 範囲を超える場合に Result で返すよう変更する (関連: `Capsule::Unknown` の長大ペイロード回避)。

### 関連ファイル

- 修正対象: `src/webtransport/capsule.rs:317-320`
- 関連リスク: `src/webtransport/capsule.rs:212` (`encode_as_data_frame`), `Unknown` capsule の再エンコード
- 一次資料: `refs/webtrans/rfc9297.txt` Section 2.1, `refs/webtrans/draft-ietf-webtrans-http3-15.txt` Section 5.6
