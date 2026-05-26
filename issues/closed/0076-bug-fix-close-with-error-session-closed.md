# 0076: close_with_error がクローズ済みセッションでカプセル未送信状態を作る

- Priority: Medium
- Created: 2026-05-14
- Completed: 2026-05-26
- Model: deepseek-v4-pro
- Branch: feature/fix-close-with-error-session-closed

## 目的

`src/webtransport/session.rs` の `close_with_error` (763行) で:

1. `queue_capsule(CloseSession {...})` がクローズ済みセッションでは `is_closed()` チェックにより早期リターン（738-741行）
2. `close_session_sent = true` が無条件に設定される（781行）

結果として「CLOSE_SESSION カプセルが実際にはキューされていないのに、送信済みフラグが立つ」という不整合が発生する。

## 優先度根拠

Medium: セッション状態の不整合を引き起こすが、クローズ済みセッションに対する操作であるため実害は限定的。ただしデバッグ時の混乱や、`is_close_session_sent()` に依存するロジックに将来的に影響する可能性がある。

## 現状

```rust
// session.rs:780-782
self.queue_capsule(capsule);        // クローズ済みなら何もしない
self.close_session_sent = true;      // 無条件に true
self.close(Some(application_error)); // セッションを閉じる
```

`queue_capsule` (738-742行):
```rust
pub fn queue_capsule(&mut self, capsule: Capsule) {
    if self.is_closed() {
        return;
    }
    self.pending_capsules.push(capsule);
}
```

## 再現手順

1. ピアが先に `WT_CLOSE_SESSION` を送信
2. `process_capsule(CloseSession)` がセッションを `Closed` 状態に遷移させる
3. アプリケーションが応答として `close_with_error` を呼び出す
4. `queue_capsule` は `is_closed()` でスキップされるが `close_session_sent = true` になる
5. `is_close_session_sent()` が `true` を返すが、送信キューにカプセルは存在しない

## 設計方針

`close_session_sent` をカプセルが実際にキューされた場合のみ設定する。`queue_capsule` の返り値を利用するか、キュー前に `is_closed()` チェックを行う。

```rust
// 修正後: クローズ済みなら早期リターン
pub fn close_with_error(&mut self, code: u32, message: impl Into<String>) {
    if self.is_closed() {
        return;
    }
    // ... (既存のメッセージ切り詰め処理)
    self.queue_capsule(capsule);
    self.close_session_sent = true;
    self.close(Some(application_error));
}
```

## テスト戦略

単体テスト: クローズ済みセッションに対して `close_with_error` を呼んだ際に `is_close_session_sent()` が `false` のまま（または事前に close した側のフラグ状態が維持される）ことを確認。

## 完了条件

- クローズ済みセッションで `close_with_error` を呼んだ際に不整合が発生しないこと
- 単体テストが pass すること
- 既存テスト (`cargo test`) が全て pass すること

## 影響範囲

- `src/webtransport/session.rs`: `close_with_error` 関数 (763行)

## 解決方法

`close_with_error` の先頭に `is_closed()` チェックを追加し、クローズ済みセッションでは早期リターンするようにした。

### 変更内容

- `src/webtransport/session.rs`: `close_with_error` の先頭に `if self.is_closed() { return; }` を追加
- `src/webtransport/session.rs`: クローズ済みセッションでの `close_with_error` 呼び出し時に `close_session_sent` が `false` のまま、送信キューが空であることを検証する単体テストを追加

## CHANGES.md エントリ案

```
- [FIX] close_with_error がクローズ済みセッションで close_session_sent フラグを誤設定する問題を修正する
  - @voluntas
```
