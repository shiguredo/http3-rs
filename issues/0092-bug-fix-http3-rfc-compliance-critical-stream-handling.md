# 0092: HTTP/3 の critical stream とフレーム検証の RFC 準拠を修正する

- Priority: High
- Created: 2026-05-30
- Polished: 2026-05-30
- Model: Codex 5.3
- Branch: feature/fix-http3-rfc-compliance-critical-stream-handling

## 目的

`src/connection/mod.rs` と `src/frame/decoder.rs`、`src/validation.rs` に RFC 9114 / RFC 9204 の MUST に対する取りこぼしがあり、接続エラー種別の誤判定や malformed 受理につながる。HTTP/3 実装の相互運用性と堅牢性を担保するため、仕様どおりに修正する。

## 優先度根拠

High: いずれも RFC の MUST に直結し、運用時に peer 実装との相互運用問題を引き起こしうる。特に critical stream への `STOP_SENDING` 検出漏れは接続管理の根幹に影響するため、優先して解消する必要がある。

## 現状

- `Connection::stop_sending` が受信側 critical stream (`control_recv` / peer QPACK stream) を判定しており、送信側 critical stream (`control_send` / local QPACK stream) への `STOP_SENDING` を `H3_CLOSED_CRITICAL_STREAM` にできない
- `decode_goaway_frame` が payload 欠落時に `BufferTooShort` を返しうるため、`H3_FRAME_ERROR` へ一貫マップできない経路が残る
- `validate_request_headers` において `Host` を `:authority` 代替で受理する経路で構文検証が不足し、`Host = uri-host [ ":" port ]` (RFC 9110) から外れた値を受理しうる
- 非 `http` / `https` スキームでの `:authority` / `Host` 制約は呼び出し側責務にしており、RFC 9114 Section 4.3.1 の MUST NOT 適用条件に対する検証が弱い

## 設計方針

- 仕様の MUST を最優先し、互換性より堅牢性を優先する
- エラー分類は RFC 9114 の規定 (`H3_CLOSED_CRITICAL_STREAM`, `H3_FRAME_ERROR`, `H3_MESSAGE_ERROR`) に厳密に合わせる
- 既存 API 破壊は避けつつ、必要なら `validation` の入力契約を拡張して request target の authority 有無を判定可能にする
- 変更箇所ごとに単体テストと PBT を追加し、再発を防止する

## 完了条件

- `STOP_SENDING` 受信時に送信側 control stream / local QPACK stream を確実に critical 判定し、`H3_CLOSED_CRITICAL_STREAM` を返す
- GOAWAY の payload 欠落や余剰バイトを必ず `H3_FRAME_ERROR` にマップできる
- `Host` 代替経路で RFC 9110 の構文検証を行い、不正値を `H3_MESSAGE_ERROR` として拒否できる
- 非 `http` / `https` スキームの authority 制約について、RFC 9114 Section 4.3.1 の条件を判定できる設計にするか、制約を満たせる明確な API 契約へ変更する
- 追加したテストが失敗を再現し、修正後に `cargo test --all` で通過する

## 解決方法

1. `Connection::stop_sending` の critical 判定対象を見直し、`control_send.stream_id()` / `encoder_stream_id` / `decoder_stream_id` を含める
2. `decode_goaway_frame` のデコード失敗を `InvalidLength` に統一し、`stream/control.rs` 側の `FrameDecodeError` マッピングを `H3_FRAME_ERROR` へ収束させる
3. `validation` に `Host` 構文検証 (`uri-host[:port]`) を追加し、`:authority` との整合チェックを強化する
4. 非 `http` / `https` スキーム時の authority 取り扱いを RFC 9114 Section 4.3.1 準拠で再設計する
5. 以下のテストを追加する
   - critical stream への `STOP_SENDING` 受信で `H3_CLOSED_CRITICAL_STREAM` になるテスト
   - GOAWAY payload 欠落時に `H3_FRAME_ERROR` になるテスト
   - 不正 `Host` を `H3_MESSAGE_ERROR` で拒否するテスト
