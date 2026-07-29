# WebTransport フロー制御カウンタの u64 加算オーバーフローを修正する

- Priority: High
- Created: 2026-06-15
- Completed: 2026-07-30
- Model: Opus 4.7
- Branch: feature/fix-webtransport-flow-control-overflow
- Polished: 2026-07-21

## 目的

`src/webtransport/session.rs` および `src/webtransport/stream.rs` のフロー制御加算が `u64` の素朴な `+=` / `+` で実装されており、オーバーフロー保護が無い。`bytes` と上限値は VarInt 範囲 (`2^62 - 1`) まで取りうる入力なので、攻撃者または不具合により上限超過が wrap で黙認される可能性がある。`checked_add` / `saturating_add` を使って安全化し、超過時には WT_FLOW_CONTROL_ERROR でセッションを閉じる経路を実装する。

## 優先度根拠

High。WebTransport のフロー制御は draft-ietf-webtrans-http3-15 Section 5.6 の `MUST` 要件 (ピアからの advertised 上限を超える送信を許さない) に直接対応している。オーバーフロー時に上限チェックが誤って通過するとフロー制御プロトコルが破綻し、ピア側で `WT_FLOW_CONTROL_ERROR` を引き起こすか、最悪リソース枯渇を招く。

## 現状

以下の箇所で `u64` 加算が無保護:

- `src/webtransport/session.rs:225-227`:
  ```rust
  self.total_received + bytes <= self.advertised_max
  ```
- `src/webtransport/session.rs:226-227` `on_data_received`:
  ```rust
  self.total_received += bytes;
  ```
- `src/webtransport/session.rs:561`:
  ```rust
  self.flow_state.data_sent + bytes <= self.remote_limits.max_data
  ```
- `src/webtransport/session.rs:618`:
  ```rust
  self.flow_state.data_sent += bytes;
  ```
- `src/webtransport/session.rs:651-654`:
  ```rust
  self.flow_state.data_received += bytes;
  ```
- `src/webtransport/stream.rs:270, 274`:
  ```rust
  self.bytes_sent += bytes;
  self.bytes_received += bytes;
  ```

加えて `src/webtransport/session.rs:88, 97, 108, 113` 等のストリーム数カウンタ (`streams_uni_opened` 等) も同様の問題を持つ。

`bytes` は外部 API から受け取り、`advertised_max` / `max_data` も SETTINGS / Capsule 由来。いずれも VarInt 範囲 (`2^62 - 1`) まで取りうる。`(2^62 - 1) + 1` の組み合わせで `<= advertised_max` 判定が wrap し、フロー制御を素通りする経路が存在する。

draft-ietf-webtrans-http3-15 Section 5.6 (1202-1207 行) は「endpoint MUST NOT exceed advertised limits」を要求しており、本実装はこれを保証できない。

## 設計方針

- 加算箇所を `checked_add` で実装し、None の場合は超過扱いで `Err(...)` または `false` を返す
- `check_received`, `can_send_data`, `try_send_data`, `add_received_data`, `on_data_received` 等のフロー制御関連メソッドを `Result` / `bool` 返却の整合した形に統一する
- 受信側の上限超過時には WT_FLOW_CONTROL_ERROR を上位 (`Session::process_capsule` / `connection::feed_stream_with_session`) に伝播する経路を確保する
- ストリーム数カウンタ (`streams_uni_opened` 等) も `saturating_add` 化する。これらは現実的にはオーバーフローしないが防御深さとして
- PBT で「`bytes = u64::MAX` のときに wrap せず False / Err を返す」プロパティを検証する

## 完了条件

- 上記列挙箇所のすべての u64 加算が `checked_add` / `saturating_add` に置き換わる
- 上限超過時にフロー制御チェックが正しく False / Err を返すことを検証する PBT が追加され成功する
- 上限超過時のセッションクローズ経路が `WT_FLOW_CONTROL_ERROR` で発火することを検証する単体テストが追加される
- 既存テスト (`tests/test_webtransport_flow_control.rs` 等) が全てパスする
- `make fmt && make clippy && make check` が全て通る

## 解決方法

例として `check_received`:

```rust
pub fn check_received(&self, bytes: u64) -> bool {
    self.advertised_max
        .checked_sub(self.total_received)
        .is_some_and(|remaining| bytes <= remaining)
}
```

`add_received_data`:

```rust
pub fn add_received_data(&mut self, bytes: u64) -> Result<(), CapsuleProcessError> {
    let new_total = self
        .total_received
        .checked_add(bytes)
        .ok_or(CapsuleProcessError::FlowControlExceeded)?;
    if new_total > self.advertised_max {
        return Err(CapsuleProcessError::FlowControlExceeded);
    }
    self.total_received = new_total;
    Ok(())
}
```

ストリーム数カウンタは `saturating_add` で十分。

PBT 例:

```rust
proptest! {
    #[test]
    fn 受信データ加算は上限超過時にエラーを返す(
        advertised in 0u64..=(1u64 << 62) - 1,
        total in 0u64..=(1u64 << 62) - 1,
        bytes in 0u64..=u64::MAX,
    ) {
        // ...
    }
}
```

### 関連ファイル

- 修正対象:
  - `src/webtransport/session.rs:225-227, 561, 618, 651-654, 88, 97, 108, 113`
  - `src/webtransport/stream.rs:270, 274`
- PBT 追加: `pbt/tests/prop_webtransport/session.rs`, `pbt/tests/prop_webtransport/stream.rs`
- 一次資料: `refs/webtrans/draft-ietf-webtrans-http3-15.txt` Section 5.6

## 解決方法

コミット f5b5260 で実装した。WebTransport のセッション・ストリームフロー制御カウンタの加算を checked_add で安全化し、超過時に WT_FLOW_CONTROL_ERROR でセッションを閉じる経路を実装した。
