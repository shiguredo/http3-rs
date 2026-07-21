# `declares_flow_control` の draft 引用を是正する

- Priority: Medium
- Created: 2026-06-15
- Model: Opus 4.7
- Branch: feature/fix-declares-flow-control-draft-citation
- Polished: 2026-07-21

## 目的

`src/webtransport/settings.rs:228-236` で `declares_flow_control` のコメントが `draft-15 Section 5.1` を引用しているが、draft-15 ではこの条件 (`wt_max_sessions_draft14 > 1` でフロー制御宣言とみなす) は削除されている。draft-14 のみで成立する条件のため、コメントから draft-15 を外す。

## 優先度根拠

Medium。仕様引用の誤りは将来の改修時に判断ミスを誘発する。AGENTS.md「資料を由来の機能を実装する場合は、根拠資料名、節番号、将来変更される可能性があることをコードコメントで明記すること」に基づき、引用の正確性を維持する責務がある。

## 現状

`src/webtransport/settings.rs:228-236` 前後 (コメント):

```rust
// draft-14 Section 5.1, draft-15 Section 5.1:
// SETTINGS_WT_INITIAL_MAX_STREAMS_BIDI/UNI/SETTINGS_WT_INITIAL_MAX_DATA のいずれかを宣言、または
// SETTINGS_WT_MAX_SESSIONS > 1 (draft-14 のみ)
pub fn declares_flow_control(&self) -> bool {
    self.wt_initial_max_streams_uni.is_some()
        || self.wt_initial_max_streams_bidi.is_some()
        || self.wt_initial_max_data.is_some()
        || self.wt_max_sessions_draft14.is_some_and(|v| v.get() > 1)
}
```

draft-15 Section 5.1 (`refs/webtrans/draft-ietf-webtrans-http3-15.txt` L989-999) は 3 つの `SETTINGS_WT_INITIAL_MAX_*` のみを規定し、`WT_MAX_SESSIONS > 1` 条件は無い。draft-14 Section 5.1 (L923-937) のみが該当する。

## 設計方針

- コメントから `draft-15 Section 5.1` を削除し、`wt_max_sessions_draft14` 条件は「draft-14 互換のみ」であることを明示
- `flow_control_enabled_with_peer` (settings.rs:243) のコメントも同様に再確認
- 仕様引用は draft 番号と節番号を含め、将来変更されうる旨も書く (AGENTS.md 規約)

## 完了条件

- コメントが draft-14 / draft-15 を正しく区別する
- 仕様引用が `refs/webtrans/draft-ietf-webtrans-http3-{14,15}.txt` の節番号と一致する
- `make fmt && make clippy && make check` が通る

## 解決方法

コメントを以下のように書き換える:

```rust
// フロー制御を宣言した状態か。
// - draft-15 Section 5.1: SETTINGS_WT_INITIAL_MAX_STREAMS_BIDI / UNI / SETTINGS_WT_INITIAL_MAX_DATA のいずれかを宣言する
// - draft-14 Section 5.1: 上記に加え、SETTINGS_WT_MAX_SESSIONS > 1 でも宣言扱い
// 将来の draft では変更される可能性がある。
```

`flow_control_enabled_with_peer` のコメントも同様に再確認。

### 関連ファイル

- 修正対象: `src/webtransport/settings.rs:228-236, 243`
- 一次資料: `refs/webtrans/draft-ietf-webtrans-http3-14.txt` / `-15.txt` Section 5.1
