# datagram の到達不能な session ID 検査を削除し、規則の適用根拠を明記する

- Created: 2026-11-09
- Completed: {YYYY-MM-DD}
- Branch: feature/refactor-wt-datagram-session-id-check
- Polished: 2026-11-10
- Category: refactor

## 目的

`Connection::feed_datagram` の 4 倍数検査は、`Datagram::decode` が `session_id = qsi * 4` を返すため決して成立しない。
到達不能な検査を残すと、draft-17 Section 4 の H3_ID_ERROR 要件が実行時に守られているように見える一方で実際には発火せず、
存在しない違反経路を読み手に示唆する。
検査と、`Datagram::decode` の到達不能な `checked_mul` の Err パスを削除し、
4 の倍数が復号形式から構造的に保証されることを記録する。
併せて、同じ session ID 規則を適用している双方向 / 単方向ストリームのコメント引用を、
規則が実在する Section 4 に揃える。

## 現状

- `src/connection/wt_stream.rs` の `Connection::feed_datagram` は `Datagram::decode` の結果だけを検査する。
  `session_id & 0x03 != 0x00` の分岐は `ErrorCode::H3DatagramError` を返し、直前のコメントは draft-15 Section 4.5 を引用している
- `src/webtransport/datagram.rs` の `Datagram::decode` は `qsi > 2^60 - 1` を `InvalidQuarterStreamId` で拒否した後、
  `qsi.checked_mul(4)` で session ID を復元する。
  `checked_mul` の Err パスは、値域検査により `qsi * 4` が最大 `2^62 - 4` に収まるため到達不能である
- decode が返す session ID は常に 4 の倍数であり、`pbt/tests/prop_datagram/main.rs` の
  `prop_arbitrary_qsi_decodes_to_multiple_of_4` がこの不変条件を検証している。`Datagram::new` も 4 の倍数を強制する
- したがって 4 倍数検査は公開 API から到達できず、テストで検証できない
- closed issue 0025 は同じ検査のエラーコードを `ErrorCode::IdError` に変更したが、
  closed issue 0047 が RFC 9297 Section 2.1 を根拠に `H3DatagramError` に戻した。
  RFC 9297 Section 2.1 が H3_DATAGRAM_ERROR を要求するのは Quarter Stream ID が短すぎる場合と 2^60-1 を超える場合であり、
  4 倍数違反は規定されていない
- Quarter Stream ID が 2^60-1 を超える datagram は session ID を復号できない不正な HTTP Datagram であり、
  RFC 9297 Section 2.1 の H3_DATAGRAM_ERROR が適用される。
  draft-17 Section 4 の H3_ID_ERROR は復号できた session ID が client-initiated bidirectional stream ID でない場合の規則であり、
  この 2 条件とは対象が異なる
- `resolve_wt_uni_stream_session_id` と `resolve_wt_bidi_stream_header` は同じ規則を `ErrorCode::IdError` で適用しているが、
  コメントの引用は Section 4.2 になっており、規則の実在する Section 4 と一致しない
- `src/connection/mod.rs` の `test_feed_datagram_qsi_overflow_returns_h3_datagram_error` は
  `[0xff; 8]` が `qsi = 2^62 - 1` に復号され値域検査で拒否されるケースを検証している。
  コメントの「0xc0 で 8 バイト長プレフィックス」と「qsi * 4 が u64 をオーバーフローする」は事実と異なり、
  テスト名の overflow も挙動と一致しない (`qsi * 4 = 2^64 - 4` は u64 に収まる)

## 設計方針

- `Connection::feed_datagram` の 4 倍数検査を削除する。
  削除箇所には、`Datagram::decode` が `session_id = qsi * 4` を返すため session ID が常に
  client-initiated bidirectional stream ID になり、draft-17 Section 4 の H3_ID_ERROR 要件が構造的に満たされることを
  コメントで記録する
- `Datagram::decode` の `checked_mul` は `expect` に置き換え、値域検査による不変条件をメッセージで示す。
  到達不能な Err パスを戻り値として残さない
- `resolve_wt_uni_stream_session_id` と `resolve_wt_bidi_stream_header` のインラインコメントの引用を draft-17 Section 4 に更新する。
  エラーコードは既に `ErrorCode::IdError` のため変更しない。
  関数の doc コメントが引用するストリーム形式の Section 4.2 / 4.3 は正しいため変更しない
- `test_feed_datagram_qsi_overflow_returns_h3_datagram_error` のコメントを実際の挙動に合わせて修正し、
  テスト名を値域外であることを表す名前に変更する
- `Datagram::decode` の Quarter Stream ID 境界テストを追加する。
  `2^60 - 1` で成功して session ID が `2^62 - 4` になり、`2^60` で `InvalidQuarterStreamId` になる
- 変更履歴は `CHANGES.md` に記載しない (CODEBASE.md の指示)

## 完了条件

- `Connection::feed_datagram` の 4 倍数検査が削除され、draft-17 Section 4 の充足根拠がコメントで記録されている
- `Datagram::decode` の `checked_mul` が `expect` に置き換えられ、エラーとして返す Err パスが削除され、
  不変条件がメッセージで示されている
- 双方向 / 単方向ストリームのインラインコメントの引用が draft-17 Section 4 に揃っている
- 既存テストのコメントとテスト名の誤りが修正され、`qsi` が 2^60-1 を超えるケースを引き続き検証している
- Quarter Stream ID の境界テストが追加されている
- PBT `prop_arbitrary_qsi_decodes_to_multiple_of_4` が通る
- `cargo fmt --all -- --check` と
  `cargo clippy --workspace --exclude interop_h3 --exclude interop_wt --all-targets -- -D warnings` と
  `cargo test --workspace --exclude interop_h3 --exclude interop_wt` が通る

## 関連ファイル

- `src/connection/wt_stream.rs`: `Connection::feed_datagram`, `Connection::resolve_wt_uni_stream_session_id`,
  `Connection::resolve_wt_bidi_stream_header`
- `src/webtransport/datagram.rs`: `Datagram::decode`
- `src/connection/mod.rs`: `test_feed_datagram_qsi_overflow_returns_h3_datagram_error`
- `pbt/tests/prop_datagram/main.rs`: `prop_arbitrary_qsi_decodes_to_multiple_of_4`
- `docs/WT_HTTP3.md`: エラーコード表 (H3_ID_ERROR を「不正なセッション ID」として全 draft に適用)。本変更後も記載どおりで変更不要

## 一次資料

- `refs/webtrans/draft-ietf-webtrans-http3-17.txt` Section 4 (L699-705): session ID の規則と H3_ID_ERROR close MUST
- `refs/webtrans/draft-ietf-webtrans-http3-15.txt` Section 4 (L653-659): draft-17 と同文
- `refs/webtrans/rfc9297.txt` Section 2.1 (L180-197): Quarter Stream ID の値域と H3_DATAGRAM_ERROR の 2 条件。
  L210-215 の H3_ID_ERROR SHOULD はストリーム数上限のケース
- `issues/closed/0025-bug-datagram-missing-settings-negotiation.md`: 同じ検査を `ErrorCode::IdError` に変更した経緯
- `issues/closed/0047-bug-h3-datagram-error-code.md`: `H3DatagramError` に戻した経緯と根拠引用
