# 不正な HTTP Datagram で H3_DATAGRAM_ERROR を返していない

Created: 2026-04-07
Completed: 2026-04-07
Model: Opus 4.6

## 概要

`Connection::feed_datagram` は HTTP Datagram のデコード失敗時に `ErrorCode::GeneralProtocolError` (`H3_GENERAL_PROTOCOL_ERROR`) を返している。RFC 9297 Section 2.1 では Quarter Stream ID が短すぎる、もしくは 2^60-1 を超える場合は `H3_DATAGRAM_ERROR` (0x33) で接続を閉じることが規定されている。現状の実装ではエラーコードが仕様と一致しない。

## 再現手順

1. 不正な HTTP Datagram (Quarter Stream ID varint が途中で切れているもの) をピアから送る。
2. `Connection::feed_datagram` が `Err(ConnectionError(GeneralProtocolError))` を返す。
3. 期待値: `Err(ConnectionError(H3DatagramError))`。

## 該当箇所

- `src/connection/mod.rs` の `feed_datagram` 内、`Datagram::decode` 失敗ハンドリング (現在 L785 付近)

## 根拠

- RFC 9297 Section 2.1: "If an HTTP/3 Datagram is received and its Quarter Stream ID field has a value greater than 2^60-1, the receiver MUST treat this as an HTTP/3 connection error of type H3_DATAGRAM_ERROR (0x33)."
- `refs/webtrans/rfc9297.txt` L187 付近

## 修正方針

1. `ErrorCode` に `H3DatagramError = 0x33` を追加 (未定義の場合)。
2. `feed_datagram` のデコード失敗パスを `H3DatagramError` に差し替える。
3. Quarter Stream ID 範囲外 (2^60-1 超) のケースも RFC 9297 に従い同じエラーコードにする。
4. 単体テストで両ケースを追加する。

## 解決方法

- `src/error.rs` に `ErrorCode::H3DatagramError = 0x33` を追加し、`from_code` / `Display` も対応させた。
- `src/connection/mod.rs` `Connection::feed_datagram` で:
  - `Datagram::decode` 失敗時のエラーコードを `H3DatagramError` に変更した。
  - `session_id & 0x03 != 0` のチェックも `H3DatagramError` に変更した (RFC 9297 Section 2.1 に従い、Quarter Stream ID から復元した stream id が client-initiated bidi でない場合も同じエラーコードで扱う)。
- `src/webtransport/datagram.rs` `Datagram::decode` で Quarter Stream ID が `2^60 - 1` を超える場合に `None` を返すよう修正した (これまで `checked_mul(4)` のみで `2^60 ≤ qsi < 2^62` の範囲が素通りしていた)。
- 単体テストとして `test_feed_datagram_truncated_returns_h3_datagram_error` と `test_feed_datagram_qsi_overflow_returns_h3_datagram_error` を追加した。
