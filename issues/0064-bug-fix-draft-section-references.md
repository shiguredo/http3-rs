# 0064: draft-ietf-webtrans-http3-15 の誤ったセクション参照を修正する

Created: 2026-05-14
Model: deepseek-v4-pro

## 概要

コードコメント内の `draft-ietf-webtrans-http3-15` へのセクション参照が 2 箇所誤っている。

## 対象箇所

### 1. `src/event.rs:7` — WtStreamReset の参照

```rust
// 修正前
/// (draft-ietf-webtrans-http3-15 Section 3.1 / 5.4)

// 修正後
/// (draft-ietf-webtrans-http3-15 Section 6 / Section 4.4 / Section 5.4)
```

Section 3.1 は "Establishing a WebTransport-Capable HTTP/3 Connection" であり、
ストリームリセットとは無関係。正しくは:
- Section 6: セッション終了時の全ストリームリセット義務
- Section 4.4: データストリームのリセット (final size)
- Section 5.4: フロー制御と reliable_size の補足

### 2. `src/webtransport/session.rs:111` — datagrams_received の参照

```rust
// 修正前
/// draft-ietf-webtrans-http3-15 Section 8

// 修正後
/// draft-ietf-webtrans-http3-15 Section 4.5
```

Section 8 は "Security Considerations" であり、データグラムカウンタを定義していない。
データグラム送受信の仕様は Section 4.5 (Datagrams) にある。

## 影響範囲

- `src/event.rs:7`
- `src/webtransport/session.rs:111`
