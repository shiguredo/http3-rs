# 0068: send_request/send_response の track_section 順序と WT CONNECT FIN 拒否

- Priority: High
- Created: 2026-05-14
- Completed: 2026-05-26
- Model: deepseek-v4-pro
- Branch: feature/fix-track-section-and-wt-connect-fin

## 目的

`src/connection/mod.rs` の `send_request` / `send_response` において 2 つの問題を修正する。

## 優先度根拠

High: 問題 1 は QPACK エンコーダーの状態不整合を引き起こしリソースリークにつながる。問題 2 は WebTransport CONNECT ストリームで FIN が送信可能な状態を許してしまい、Capsule プロトコル通信が不可能になる。

## 現状

### 問題 1: track_section が send_encoded_headers より先に呼ばれる

`send_request` (3453行) と `send_response` (3577行) で `qpack_encoder.track_section(stream_id, ric)` が `stream.send_encoded_headers()` よりも先に実行されている。`send_encoded_headers` がストリーム状態不正等でエラーを返した場合、セクションがエンコーダーに登録されたままになり、QPACK エンコーダーの状態不整合とリソースリークが発生する。

RFC 9204 Section 2.1.1 / Section 4.4.1 に基づき、track_section は実際にフィールドセクションがストリームに書き込まれた後にのみ記録すべき。

### 問題 2: WebTransport CONNECT で fin=true が拒否されない

`send_request` の 3469行で `if is_connect && !has_protocol {` の条件分岐内でのみ FIN チェックが行われている。WebTransport CONNECT (`is_connect && has_protocol`) の場合は FIN チェックがスルーされる。

CONNECT ストリーム（plain / WebTransport 共に）は長期生存双方向ストリームであり、リクエスト送信時に FIN を送信すると後続の Capsule プロトコル通信が不可能になる（draft-ietf-webtrans-http3-15 Section 3, RFC 9114 Section 4.4）。

## 設計方針

### 修正 1: track_section を send_encoded_headers の成功後に移動

```rust
// 修正前 (send_request, 3453行付近):
let ric = self.qpack_encoder.last_required_insert_count();
self.qpack_encoder.track_section(stream_id, ric);
// ... (多数の処理) ...
stream.send_encoded_headers(&qpack_buf, fin, false)?;

// 修正後:
let ric = self.qpack_encoder.last_required_insert_count();
// ... (多数の処理) ...
stream.send_encoded_headers(&qpack_buf, fin, false)?;
self.qpack_encoder.track_section(stream_id, ric);
```

`send_response` (3577行付近) も同様に修正する。

### 修正 2: WebTransport CONNECT でも FIN を拒否

```rust
// 修正前 (3469行):
if is_connect && !has_protocol {
    if fin {
        return Err(Error::StreamError(ErrorCode::MessageError));
    }
    stream.set_connect_request();
}

// 修正後:
if is_connect {
    if fin {
        return Err(Error::StreamError(ErrorCode::MessageError));
    }
    if !has_protocol {
        stream.set_connect_request();
    }
}
```

## テスト戦略

単体テストで対応する。

### 問題 1 のテスト

`send_encoded_headers` が失敗するケース（ストリーム状態不正）を構築し、`track_section` が呼ばれていないことを検証する。具体的には:
- ストリームを closed 状態にした上で `send_response` を呼び、エラー返却後に QPACK エンコーダーの tracked sections 数が変化していないことを確認する

### 問題 2 のテスト

- WebTransport CONNECT ヘッダー（`:method: CONNECT`, `:protocol: webtransport`）で `fin=true` を指定して `send_request` を呼び、`StreamError(MessageError)` が返ることを確認する
- 同じヘッダーで `fin=false` なら成功することを確認する

## 完了条件

- `send_request` / `send_response` の `track_section` が `send_encoded_headers` の後に移動していること
- WebTransport CONNECT で `fin=true` が拒否されること
- 上記テストが全て pass すること
- 既存テスト (`cargo test`) が全て pass すること
- CHANGES.md にエントリが追記されていること

## 後方互換性

- 問題 1: 外部 API 変更なし。内部状態管理の修正のみ
- 問題 2: WebTransport CONNECT で `fin=true` を指定していたコードは `StreamError(MessageError)` を受け取るようになる。ただし FIN 付き CONNECT は仕様上無意味な操作であるため、既存の正常なコードには影響しない

## 影響範囲

- `src/connection/mod.rs`: `send_request` 関数 (3453行, 3469-3475行)、`send_response` 関数 (3577行)

## RFC 根拠

- RFC 9204 Section 2.1.1: Required Insert Count と section tracking の規定
- RFC 9204 Section 4.4.1: Section Acknowledgment — 送信が完了したセクションのみを追跡すべき
- RFC 9114 Section 4.4: CONNECT メソッド — ストリームは open のまま維持する必要がある
- draft-ietf-webtrans-http3-15 Section 3: WebTransport セッションは CONNECT ストリーム上で確立され、双方向の Capsule 通信に使用される

## 解決方法

### 問題 1: track_section の順序修正

`src/connection/mod.rs` の `send_request` と `send_response` で `qpack_encoder.track_section()` を `stream.send_encoded_headers()` の後に移動した。`send_encoded_headers` が失敗した場合に `track_section` へ到達しないため、未送出セクションがエンコーダーに登録されたままになる問題が解消される。

### 問題 2: WebTransport CONNECT の FIN 拒否

`send_request` の FIN チェックの条件を `is_connect && !has_protocol` から `is_connect` に拡張し、`stream.set_connect_request()` は `!has_protocol` の場合のみ実行するようにした。

### テスト

インラインテスト 2 件を追加:
- `test_wt_connect_rejects_fin`: WebTransport CONNECT で fin=true → `StreamError(MessageError)`
- `test_wt_connect_without_fin_succeeds`: WebTransport CONNECT で fin=false → 成功

## CHANGES.md エントリ案

```
- [FIX] send_request / send_response で track_section が send_encoded_headers の前に呼ばれていた問題を修正する
  - @voluntas
- [FIX] WebTransport CONNECT リクエストで fin=true が拒否されない問題を修正する
  - @voluntas
```
