# StreamHeader::new が公開 API で panic する

Created: 2026-04-07
Completed: 2026-04-07
Model: Opus 4.6

## 概要

`webtransport::stream::StreamHeader::new` は `session_id % 4 != 0` の場合 `assert!` で panic する公開関数。issue 0031 で「公開 API から panic を撤去する」方針を掲げているにも関わらず、境界が閉じ切っていない。

## 該当箇所

- `src/webtransport/stream.rs` `StreamHeader::new` (現在 L46 付近)

## 修正方針

1. `StreamHeader::new` を `Result<Self, StreamHeaderError>` に変更し、`InvalidSessionId` を返すようにする。
2. クレート内の呼び出し箇所を更新する (内部で session_id が常に正しいと保証できる箇所では `expect` ではなく明示的な保証コメント付きで unwrap してもよい)。
3. 単体テストで `Err(InvalidSessionId)` が返ることを確認する。

## 補足

`varint::encoded_len` / `varint::encode_into_vec` の panic についても 0031 のスコープに含まれるが、varint 実装の慣例上 62 bit 超で panic する設計は許容される範囲とし、本 issue では扱わない。必要なら別 issue で内部化を検討する。

## 解決方法

- `src/webtransport/stream.rs` `StreamHeader::new` のシグネチャを `pub fn new(session_id: u64) -> Result<Self, StreamHeaderDecodeError>` に変更し、`session_id % 4 != 0` の場合は `Err(InvalidSessionId)` を返すようにした (panic の `assert!` を削除)。
- 未使用だった `Stream::header()` (内部で `StreamHeader::new` を呼び出していた) を削除した。
- 呼び出し側を更新:
  - `pbt/tests/prop_webtransport.rs`: `valid_session_id()` strategy が常に有効値を返すため `.unwrap()` を追加。
  - `interop_wt/src/lib.rs`, `crates/tokio-s2n-quic/src/webtransport/session.rs`, `crates/tokio-ngtcp2/tests/webtransport_h3_integration_e2e.rs`: いずれも CONNECT ストリーム ID を渡しているため `.expect("...")` で内部不変条件を表明する。
- 単体テスト `test_stream_header_new_rejects_invalid_session_id` を追加した。
