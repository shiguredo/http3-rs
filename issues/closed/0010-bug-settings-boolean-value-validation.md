# SETTINGS_ENABLE_CONNECT_PROTOCOL と SETTINGS_H3_DATAGRAM の値検証が緩い

Created: 2026-04-05
Model: Opus 4.6

## 概要

`Settings::from_payload()` (`src/settings.rs:172-173`) で `SETTINGS_ENABLE_CONNECT_PROTOCOL` (0x08) と `SETTINGS_H3_DATAGRAM` (0x33) を `*value != 0` で `bool` に丸めている。値 `2` などの不正値を受け入れてしまい、`H3_SETTINGS_ERROR` を返さない。

WebTransport 側の `enable_webtransport_draft02` (0x2b603742) も同様に `*value != 0` で受けている (`src/webtransport/settings.rs:256`)。

## 根拠

- RFC 8441 Section 3: 「The value of the parameter MUST be 0 or 1.」
- nghttp3 は `ENABLE_CONNECT_PROTOCOL` (`nghttp3_conn.c:2275-2285`) と `H3_DATAGRAM` (`nghttp3_conn.c:2291-2296`) の両方で `0/1` 以外を `NGHTTP3_ERR_H3_SETTINGS_ERROR` にしている
- HTTP Datagram の仕様でも `SETTINGS_H3_DATAGRAM` は値 `1` を送信すると規定

## 問題

不正な SETTINGS 値を持つ peer 接続を受け入れてしまう。RFC 準拠の peer とのインターオペラビリティには直接影響しないが、不正な peer に対するバリデーションが不足しており、仕様不適合。

## 対応方針

- `Settings::from_payload()` で `0x08` と `0x33` の値が `0` または `1` 以外の場合 `H3_SETTINGS_ERROR` を返す
- `from_payload()` の戻り値を `Result<Self, Error>` に変更する
- WebTransport 側の `enable_webtransport_draft02` も同様に厳格化する
- 対応する PBT を追加する

Completed: 2026-04-05

## 解決方法

- `Settings::from_payload()` の戻り値を `Result<Self, Error>` に変更し、`0x08` / `0x33` の値が `0` / `1` 以外の場合に `Error::ConnectionError(ErrorCode::SettingsError)` を返すようにした
- `webtransport::Settings::from_payload()` の戻り値を `Result<Option<Self>, Error>` に変更し、`0x2b603742` の値が `0` / `1` 以外の場合に同様のエラーを返すようにした
- 呼び出し元 (`control.rs`, `connection/mod.rs`) を `?` でエラー伝搬するように更新した
- 不正値と正常値の両方を検証する PBT を `pbt/tests/prop_settings.rs` に追加した
