# `huffman::decode` の `Vec::with_capacity` 使用を撤廃する

- Priority: High
- Created: 2026-06-15
- Completed: 2026-07-30
- Model: Opus 4.7
- Branch: feature/fix-huffman-decode-with-capacity
- Polished:

## 目的

`src/qpack/huffman.rs:1100` の `Vec::with_capacity(data.len() * 2)` は AGENTS.md「入力バイナリデータをデコードする際には `Vec::with_capacity()` などのメモリを事前に割り当てるメソッドを原則として使用しないこと」に違反している。ピアからの任意バイト列を渡されると `data.len() * 2` の確保を強制でき、さらに `data.len() > usize::MAX / 2` で乗算オーバーフローが発生する (debug で panic、release で wrap)。`Vec::new()` に置換して規約を遵守する。

## 優先度根拠

High。AGENTS.md の明示的な規約違反であり、入力バイナリデコードという最も警戒すべき経路に該当する。性能差は誤差で対処コストも軽微なため、すぐに修正できる。

## 現状

`src/qpack/huffman.rs:1100`:

```rust
pub fn decode(data: &[u8]) -> Result<Vec<u8>, QpackError> {
    let mut result = Vec::with_capacity(data.len() * 2);
    // ...
}
```

AGENTS.md (リポジトリルート):

> 入力バイナリデータをデコードする際には `Vec::with_capacity()` などのメモリを事前に割り当てるメソッドを原則として使用しないこと
> - 入力データが破損している場合などに、サイズやカウントを示す値のデコード結果が極端に大きくなり、メモリを大量に消費してしまうリスクがあるため
> - このケースでも `Vec::new()` を使っておけば、メモリ消費量のオーダーは実際の入力データのサイズから大きく乖離することはない

QPACK の Huffman デコードは入力バイト列を受け取って展開するため、この規約が直接適用される経路。

## 設計方針

`Vec::with_capacity(data.len() * 2)` を `Vec::new()` に置換する。事前確保を廃止しても push が再確保するだけで挙動は等価。性能差は実用上無視できる。

## 完了条件

- `src/qpack/huffman.rs:1100` の `Vec::with_capacity` 使用が解消される
- `cargo test --tests -p shiguredo_http3 -p pbt` が全てパスする
- `cargo test --test fuzz_huffman` 相当の fuzz でクラッシュしない (既存 fuzz_target 経由で確認)
- `make fmt && make clippy && make check` が全て通る

## 解決方法

```rust
pub fn decode(data: &[u8]) -> Result<Vec<u8>, QpackError> {
    let mut result = Vec::new();
    // ...
}
```

### 関連ファイル

- 修正対象: `src/qpack/huffman.rs:1100`
- 規約: `AGENTS.md` ルート (`AGENTS.md` シンボリックリンク)

## 解決方法

コミット f5b5260 で実装した。huffman::decode 内の Vec::with_capacity(data.len() * 2) を Vec::new() に置換し、AGENTS.md 規約への違反と乗算オーバーフローのリスクを解消した。
