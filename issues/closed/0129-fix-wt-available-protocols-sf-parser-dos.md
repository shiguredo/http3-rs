# `WT-Available-Protocols` Structured Fields パーサの DoS リスクに対処する

- Priority: Medium
- Created: 2026-06-15
- Completed: 2026-07-30
- Model: Opus 4.7
- Branch: feature/fix-wt-available-protocols-sf-parser-dos
- Polished: 2026-07-21

## 目的

`src/webtransport/connect.rs:583-588, 690-707` の `parse_sf_list_strings` は `value.split(',')` と `replace` を多用する Structured Fields の最低限実装で、大きな入力 (100MB の `WT-Available-Protocols` 値等) を渡されるとメモリ増幅・CPU 消費の DoS リスクがある。サイズ上限と制御文字検査を追加する。

## 優先度根拠

Medium。Sans I/O ライブラリで悪意ある HTTP/3 ピアから不正な SF を送られたケースに対する防御深さ。実害は上位層の `qpack::Header` のサイズ制限で部分的に緩和されるが、本パーサ自体に上限を持たせるべき。

## 現状

`src/webtransport/connect.rs:690-707`:

```rust
fn parse_sf_list_strings(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|s| s.trim())
        .filter_map(|s| parse_sf_item_string(s))
        .collect()
}
```

`replace` は不変文字列を毎回コピーするため、`\\\\` や `\"` の多用で実行時間 / メモリが線形に膨らむ。

RFC 9651 (`refs/rfc9651.txt`) は Structured Field の上限を明示しないが、実装定義の上限を設けて拒否することが期待される。

## 設計方針

- `WT-Available-Protocols` の生値長 (`value.len()`) に対する上限を導入 (例: 4 KB)
- 上限超えはエラー (`ConnectError::HeaderTooLarge` 等)
- RFC 9651 で string-token に含めてはいけない制御文字 (0x00-0x1F, 0x7F) を検査
- `parse_sf_list_strings` のループ内でも要素数の上限を設ける (例: 100 エントリ)
- PBT で長大入力 / 制御文字混入で必ずエラーが返るプロパティを検証

## 完了条件

- パース前にサイズ上限チェックが入る
- 制御文字検査が入る
- 要素数上限が導入される
- PBT で境界が検証される
- `make fmt && make clippy && make check` が通る

## 解決方法

```rust
const MAX_SF_INPUT_LEN: usize = 4096;
const MAX_SF_LIST_ITEMS: usize = 100;

fn parse_sf_list_strings(value: &str) -> Result<Vec<String>, ConnectError> {
    if value.len() > MAX_SF_INPUT_LEN {
        return Err(ConnectError::HeaderTooLarge);
    }
    if value.bytes().any(|b| b < 0x20 || b == 0x7F) {
        return Err(ConnectError::ControlCharInHeader);
    }
    let items: Vec<_> = value.split(',').take(MAX_SF_LIST_ITEMS + 1).collect();
    if items.len() > MAX_SF_LIST_ITEMS {
        return Err(ConnectError::TooManyItems);
    }
    // ...
}
```

### 関連ファイル

- 修正対象: `src/webtransport/connect.rs:583-588, 690-707`
- PBT 追加: `pbt/tests/prop_webtransport/connect.rs`
- 一次資料: `refs/rfc9651.txt` Section 4

## 解決方法

コミット f5b5260 で実装した。WT-Available-Protocols Structured Fields パーサにサイズ上限と制御文字検査を追加し、DoS リスクに対処した。
