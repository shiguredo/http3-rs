# 0063: send_request が GOAWAY 境界超過 / WT セッション上限超過時に接続全体を閉じる

- Priority: High
- Created: 2026-05-14
- Completed: 2026-05-26
- Model: deepseek-v4-pro
- Branch: feature/fix-send-request-error-level

## 目的

`src/connection/mod.rs` の `send_request` 関数において、2 箇所で `Error::ConnectionError(ErrorCode::RequestRejected)` を返している。RFC 9114 Section 8 では全エラーコードはストリーム・接続の両方で使用可能と規定されているが、Section 4.1.1 では `H3_REQUEST_REJECTED` を「サーバーがレスポンスストリームを abort する際のエラーコード」として記述しており、特定リクエストの拒否に使うストリームレベルの用途を想定している。これらのケースでは接続エラーとして扱うのは RFC の推奨するグレースフルな動作と矛盾する。

現状の実装では、GOAWAY 受信後に新規リクエストを試みた場合や、フロー制御なしの WebTransport セッション上限（同時 1 セッション）を超えた場合に、接続全体が閉じられてしまい、同一接続上の他の進行中リクエスト/レスポンスも失われる。

## 優先度根拠

High: 接続全体を不必要に閉じることで、進行中の他ストリームに影響を与える実害がある。修正自体は 2 箇所の `ConnectionError` → `StreamError` への変更のみで低リスク。

## 現状

`send_request` 関数内の以下 2 箇所が `ConnectionError` を返している:

1. **フロー制御なし WT セッション上限超過** (`connection/mod.rs` の `send_request` 関数内、`!self.is_wt_flow_control_enabled() && self.count_active_wt_sessions() >= 1` の条件分岐)
   - draft-ietf-webtrans-http3-15 Section 5.1: "clients MUST NOT attempt to establish more than one simultaneous WebTransport session" (フロー制御無効時)
   - 同セクション: サーバーは過剰な CONNECT ストリームを `H3_REQUEST_REJECTED` で個別にリセットしなければならない (MUST)

2. **GOAWAY 受信後の新規リクエスト作成** (`connection/mod.rs` の `send_request` 関数内、`self.peer_goaway_request_boundary()` で境界値以上のストリーム ID を検出する条件分岐)
   - RFC 9114 Section 5.2: "Endpoints MUST NOT initiate new requests or promise new pushes on the connection after receiving a GOAWAY frame"
   - 同セクション: 境界超過リクエストは処理されず別接続で再試行可能だが、接続自体は維持される

## 設計方針

両箇所で `ConnectionError` を `StreamError` に変更する。

```rust
// 修正前:
return Err(Error::ConnectionError(ErrorCode::RequestRejected));

// 修正後:
return Err(Error::StreamError(ErrorCode::RequestRejected));
```

同一関数内の他の `ConnectionError` 箇所（`InternalError` を返す設定不足チェック群）は接続レベルの前提条件違反であり、`ConnectionError` のままが正しい。修正対象は `RequestRejected` を返す 2 箇所のみ。

注: `send_request` がエラーを返す時点ではストリームは作成されておらず、wire 上で `H3_REQUEST_REJECTED` が送信されるわけではない。返り値の `StreamError(RequestRejected)` は呼び出し元に「このリクエストは拒否されたが接続は維持されている」というセマンティクスを伝える内部 API のエラーである。RFC 9114 Section 4.1.1 の "Clients MUST NOT use the H3_REQUEST_REJECTED error code" はクライアントがストリームリセットとして wire に送出することを禁止する規定であり、内部 API の戻り値には適用されない。

## テスト戦略

単体テストで対応する（意図的なエラーパスの検証）。

### 既存テストの修正

- `tests/test_webtransport_draft_connect.rs`: WT セッション上限超過テストの期待値を `Error::ConnectionError(ErrorCode::RequestRejected)` から `Error::StreamError(ErrorCode::RequestRejected)` に変更する

### 追加テスト

`tests/test_connection.rs`（既存ファイルまたは新規作成）に以下を追加:

- GOAWAY 受信後の `send_request` が `StreamError(RequestRejected)` を返すこと
- GOAWAY 境界値未満のストリーム ID では正常に送信できること（境界値テスト）
- `StreamError` 返却後も接続が維持され、他ストリームの送受信が可能であること

## 完了条件

- 2 箇所の `ConnectionError(RequestRejected)` が `StreamError(RequestRejected)` に変更されていること
- 既存テストの期待値修正が完了し全テスト pass すること
- GOAWAY 境界超過の単体テストが追加され pass すること
- 既存テスト (`cargo test`) が全て pass すること
- CHANGES.md にエントリが追記されていること

## 後方互換性

`send_request` の戻り値型 `Result<u64, Error>` は変更されない。`Error` enum のバリアント `StreamError` は既に定義されている (`src/error.rs`)。ただし、呼び出し元で `ConnectionError(RequestRejected)` にマッチしてハンドリングしているコードがある場合は修正が必要。`send_request` はユーザーが直接呼ぶ API であるため、`[FIX]` として記録する。

## 影響範囲

- `src/connection/mod.rs`: `send_request` 関数内の 2 箇所（WT セッション上限チェック、GOAWAY 境界チェック）
- `tests/test_webtransport_draft_connect.rs`: テスト期待値の修正

## RFC 根拠

- RFC 9114 Section 4.1.1: "A server MAY abort its response stream with the error code H3_REQUEST_REJECTED" — 特定リクエストの拒否をストリーム単位で行う用途を想定
- RFC 9114 Section 8.1: H3_REQUEST_REJECTED (0x10b) の定義。全エラーコードはストリーム・接続の両方で使用可能だが、このケースでは接続エラーは過剰
- RFC 9114 Section 5.2: GOAWAY 受信後の新規リクエスト禁止規定。境界超過リクエストは処理されないが接続自体は維持される
- draft-ietf-webtrans-http3-15 Section 5.1: フロー制御無効時の同時セッション制限。サーバーは過剰ストリームを `H3_REQUEST_REJECTED` で個別にリセットしなければならない (MUST)

## 解決方法

`src/connection/mod.rs` の `send_request` 関数内の 2 箇所で `Error::ConnectionError(ErrorCode::RequestRejected)` を `Error::StreamError(ErrorCode::RequestRejected)` に変更した:

1. フロー制御なし WT セッション上限超過時 (line 3422)
2. GOAWAY 境界超過時 (line 3434)

テスト変更:
- `tests/test_webtransport_draft_connect.rs`: 既存テストの期待値を `ConnectionError` から `StreamError` に修正
- `tests/test_connection.rs`: 新規作成。GOAWAY 受信後の send_request 拒否、境界値テスト、接続維持確認の 3 テストを追加

## CHANGES.md エントリ案

```
- [FIX] send_request で GOAWAY 境界超過およびフロー制御なし WT セッション上限超過時に ConnectionError ではなく StreamError を返すよう修正する
  - @voluntas
```
