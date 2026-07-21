# 禁止 Capsule 受信時のエラーコードを仕様準拠に修正する

- Priority: Medium
- Created: 2026-06-15
- Model: Opus 4.7
- Branch: feature/fix-prohibited-capsule-error-code
- Polished: 2026-07-21

## 目的

`src/webtransport/session.rs:894-901, 1151-1163` で `WT_MAX_STREAM_DATA` / `WT_STREAM_DATA_BLOCKED` (HTTP/3 上で禁止された capsule) を受信した際、`Error::Protocol(ErrorCode::FlowControlError)` を返している。draft-ietf-webtrans-http3-15 Section 5.4 (L1129-1131) は「session error」とだけ規定しエラーコードを指定していないため、`WT_FLOW_CONTROL_ERROR` を流用する根拠が無い。仕様に整合する形に修正する。

## 優先度根拠

Medium。仕様文面上の明示的 MUST 違反ではないが、エラーコードの意味の取り違えで、相互運用時にピア側のエラー解析を混乱させる可能性がある。

## 現状

`src/webtransport/session.rs:894-901`:

```rust
if capsule.is_prohibited_in_http3() {
    return Err(CapsuleProcessError::Connection(ErrorCode::FlowControlError as u64));
}
```

draft-15 Section 5.4 (`refs/webtrans/draft-ietf-webtrans-http3-15.txt` 1129-1131):

> Endpoints MUST treat receipt of a WT_MAX_STREAM_DATA or a WT_STREAM_DATA_BLOCKED capsule as a session error.

「session error」とのみ規定。具体的なエラーコードは指定されていない。

`WT_FLOW_CONTROL_ERROR` は draft-15 Section 9.5 のフロー制御専用エラーコードで、用途が異なる。

## 設計方針

- 禁止 capsule 受信時のエラーコードを変更:
  - 案 A: アプリケーションエラーコード (`Error::application(0, "...")`) でセッションを閉じる
  - 案 B: 仕様文面通り「session error」を返す新しい variant (`CapsuleProcessError::ProhibitedCapsule`) を作り、上位で WT_PROTOCOL_VIOLATION のような汎用エラーで session を閉じる
- 仕様に明示が無いため、相互運用テスト (nghttp3, quiche, s2n-quic) でどう扱われているかを参考に決定
- コメントで「draft-15 Section 5.4 はエラーコードを規定していない」と引用を入れる

## 完了条件

- `WT_FLOW_CONTROL_ERROR` の使用が外される
- 適切なエラーコードまたは session error 経路が選ばれ、コードコメントで仕様根拠が明示される
- `test_session_process_prohibited_capsule_returns_error` の期待値が更新される
- `make fmt && make clippy && make check` が通る

## 解決方法

`session.rs:894-901` 周辺のエラー生成を、選択した方針に従って書き換える。コメントに以下を追加:

```rust
// draft-ietf-webtrans-http3-15 Section 5.4: 禁止 capsule は session error として扱う MUST。
// 仕様はエラーコードを規定していないため、アプリケーションエラーコード経由でセッションを閉じる。
```

### 関連ファイル

- 修正対象: `src/webtransport/session.rs:894-901, 1151-1163`
- 影響: `tests/test_webtransport_flow_control.rs` 内の関連テスト
- 一次資料: `refs/webtrans/draft-ietf-webtrans-http3-15.txt` Section 5.4
