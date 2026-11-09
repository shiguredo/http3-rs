# datagram の session ID 不正時の接続エラーを H3_ID_ERROR に揃える

- Created: 2026-11-09
- Completed: {YYYY-MM-DD}
- Branch: feature/change-wt-datagram-invalid-session-id-error
- Polished: {YYYY-MM-DD}
- Category: change

## 目的

datagram で受信した session ID が client-initiated bidirectional stream ID に対応しない場合に接続を H3_ID_ERROR で閉じる MUST が規定されている (draft-15 Section 4 (L653-659) から同文で、draft-17 Section 4 (L699-705) にも引き継がれている)。`Connection::feed_datagram` の 4 倍数検査分岐は H3_DATAGRAM_ERROR を返すため、該当規定と矛盾するコードが残っている。該当条件と規定の対応は特定済みのため、この分岐を H3_ID_ERROR に揃える。

## 現状

- `src/connection/wt_stream.rs` の `Connection::feed_datagram` は `session_id & 0x03 != 0x00` の分岐で `Error::ConnectionError(ErrorCode::H3DatagramError)` を返す。直前のコメント引用は `draft-ietf-webtrans-http3-15 Section 4.5` のままである
- `src/webtransport/datagram.rs` の `Datagram::decode` は Quarter Stream ID (`qsi`) の値域を検査した後、`session_id = qsi * 4` で session ID を復元する。decode が返す session ID は必ず 4 の倍数であるため、`feed_datagram` の 4 倍数検査分岐は構造的に到達不能である。`checked_mul` の Err パスも値域検査により到達不能である
- H3_DATAGRAM_ERROR が要求されるのは RFC 9297 Section 2.1 が定める、Quarter Stream ID varint が短すぎる場合と Quarter Stream ID が 2^60-1 を超える場合の 2 条件のみである。どちらも `Datagram::decode` 失敗として既に `H3DatagramError` で満たしており、`src/connection/mod.rs` の `test_feed_datagram_truncated_returns_h3_datagram_error` と `test_feed_datagram_qsi_overflow_returns_h3_datagram_error` が担保している
- 4 倍数違反に H3_DATAGRAM_ERROR を使う由来は closed issue `issues/closed/0047-bug-h3-datagram-error-code.md` だが、その解決方法は RFC 9297 Section 2.1 を根拠としている。RFC 9297 Section 2.1 に 4 倍数条件の規定は無く、当時から draft-15 Section 4 (L653-659) が H3_ID_ERROR を定めていたため、この分岐の根拠引用は誤りだったと特定できる
- 双方向 (`resolve_wt_bidi_stream_header`) と単方向 (`resolve_wt_uni_stream_session_id`) の受信 session ID 検査は既に `ErrorCode::IdError` を使用している
- `Connection::send_datagram` の 4 倍数検査はローカル API 誤用に対する `GeneralProtocolError` であり、本 issue の対象外とする

## 設計方針

- `Connection::feed_datagram` の 4 倍数分岐のエラーコードを `ErrorCode::IdError` に変更し、コメントの引用を draft-17 Section 4 に更新する
- 4 倍数分岐は `Datagram::decode` による `session_id = qsi * 4` 復元の構造上到達不能であるが、`Datagram` の復号構造が将来変わった場合に備えた検証として分岐を削除せずに維持し、到達不能の理由をコメントで記録する
- Quarter Stream ID varint が短すぎる場合と 2^60-1 を超える場合の 2 条件は、RFC 9297 Section 2.1 どおり `H3DatagramError` を維持する
- `Datagram::decode` の `checked_mul` は値域検査後に必ず成功するが、防御として維持し、到達不能の理由をコメントで記録する
- `Datagram::decode` の境界テストを追加する。Quarter Stream ID が `2^60 - 1` (session ID は `2^62 - 4`) のとき成功し、`2^60` のとき失敗する

## 完了条件

- `Connection::feed_datagram` の 4 倍数分岐が `ErrorCode::IdError` を返し、draft-17 Section 4 を引用して到達不能の理由がコメントに記録されている
- Quarter Stream ID varint が短すぎる場合と 2^60-1 を超える場合の 2 条件が引き続き `H3DatagramError` で満たされており、既存テストの退行がない
- `Datagram::decode` の Quarter Stream ID 境界テスト (最大値の成功 / 1 超過の失敗) が追加されている
- `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` と `cargo test --all` が通る

## 関連ファイル

- `src/connection/wt_stream.rs`: `Connection::feed_datagram`
- `src/webtransport/datagram.rs`: `Datagram::decode`
- `src/error.rs`: `ErrorCode::IdError`
- `src/connection/mod.rs`: `test_feed_datagram_truncated_returns_h3_datagram_error`, `test_feed_datagram_qsi_overflow_returns_h3_datagram_error`

## 一次資料

- `refs/webtrans/draft-ietf-webtrans-http3-17.txt` Section 4 (L699-705): unidirectional stream / bidirectional stream / datagram 共通の session ID 規定と H3_ID_ERROR close MUST
- `refs/webtrans/draft-ietf-webtrans-http3-15.txt` Section 4 (L653-659): draft-17 と同文
- `refs/webtrans/rfc9297.txt` Section 2.1: Quarter Stream ID の値域検査と H3_DATAGRAM_ERROR が接続される 2 条件
- `issues/closed/0047-bug-h3-datagram-error-code.md`: 現行の 4 倍数分岐エラーコードの由来
