# draft-ietf-webtrans-http3-16 に追従する

- Created: 2026-07-31
- Completed: 2026-07-31
- Branch: feature/change-webtrans-http3-draft-16
- Polished: 2026-07-31

## 目的

draft-ietf-webtrans-http3-16 (2026-07-06 公開、WG Last Call 中) に追従する。draft-15 から draft-16 でコード変更が必要な仕様変更に対応し、RFC 化に備える。

## 現状

コードベース・README はすべて draft-15 止まり。draft-16 は 2026-07-06 に公開され、WG Last Call に入っている。IANA 登録値 (コードポイント) に変更はないが、エラー処理・検証ロジックに影響する変更が含まれる。

一次資料: `refs/webtrans/draft-ietf-webtrans-http3-16.txt` (追加済み)

## 設計方針

draft-15 → draft-16 のコード変更が必要な 8 件に個別に対応する。コードポイントの変更はないため、`Setting` enum や `CapsuleType` の定義変更は不要。

**DraftVersion の扱い**: draft-15 と draft-16 は SETTINGS コードポイントが同一 (`SETTINGS_WT_ENABLED=1`) で、SETTINGS レベルでは区別不能。`DraftVersion::Draft16` variant は追加せず、draft-16 の挙動変更 (項目 5・6・7) は draft-15 以降に無条件で適用する。README の対応 draft 表には draft-16 を追加するが、`DraftVersion` enum は変更しない。

### 1. SETTINGS_WT_ENABLED > 1 の検証 (Section 3.1)

draft-16 追加要件:

> A value of "1" indicates support for the variant of WebTransport that is described in this document (that is, "webtransport-h3"). Clients MUST treat values greater than "1" as a connection error of type H3_SETTINGS_ERROR.

- `src/webtransport/settings.rs` の `Settings::detect_draft_pattern` / `is_enabled` は `wt_enabled.get() > 0` で判定している
- SETTINGS 受信時に `wt_enabled > 1` を検出して `H3_SETTINGS_ERROR` 接続エラーを返す検証を追加する。SETTINGS フレームの処理は `src/connection/mod.rs` の `process_control_stream` にあるため、検証の呼び出しは同関数に追加する (検証関数自体を `wt_session.rs` に置く設計にしてもよいが、呼び出し側の変更が必須)
- `src/webtransport/connect/mod.rs` の `CapabilityError::MissingWebTransportSetting` の Display メッセージを `"value > 0"` から `"value of 1"` に更新する
- `src/webtransport/connect/connect_error.rs` の `CapabilityError::MissingWebTransportSetting` と `TransportCapabilities.wt_enabled` の doc コメントも `"> 0"` から `"= 1"` に更新する

### 2. 非対応リソースへの応答を 404 → 405 に変更 (Section 3.2)

draft-15:

> If it does not, it SHOULD reply with status code 404

draft-16:

> If the target resource does not support WebTransport, the server SHOULD reply with status code 405

- Sans I/O ライブラリ本体はステータスコードを強制しないため、変更対象は `examples/wt_server/src/main.rs` の `--reject-connect` デモ (現在 404 を使用。doc コメント・ログ・CLI ヘルプの 4 箇所) のみ

### 3. 0-RTT 再開時のクライアント検証 (Section 3.2)

draft-16 追加要件:

> A client MUST close the connection with H3_SETTINGS_ERROR if the SETTINGS frame received in the resumed connection reduces any flow control values from the cached previous values.

- クライアントが 0-RTT 再開時にフロー制御値の減少を検出して `H3_SETTINGS_ERROR` で接続を閉じるロジックを追加する
- 比較対象: `wt_initial_max_streams_uni`、`wt_initial_max_streams_bidi`、`wt_initial_max_data` の 3 フィールド
- Sans I/O ライブラリとしては、呼び出し側が前回の SETTINGS を保持して比較できる API を提供する (API の具体的なシグネチャは実装時に決定する)

### 4. 楽観的カプセル送信の処理 (Section 3.2)

draft-16 追加要件:

> To reduce latency at the start of a WebTransport session, a client MAY optimistically send capsules on the CONNECT stream before receiving the server's response. A server MUST NOT process these bytes as capsules until it sends a 2xx response accepting the session. Bytes received before the server sends the response are processed once the session is accepted or discarded if the session is rejected.

- 現状: `src/connection/wt_capsule.rs` の `handle_wt_data_frame` は `WtSessionState::Pending` 時に draft-07/14/15 で `H3_MESSAGE_ERROR` を返している
- 変更: **サーバー側のみ** `handle_wt_data_frame` の Pending 分岐でカプセルデータをバッファリングし、`src/connection/wt_session.rs` の `establish_wt_session_server` (2xx 送信時) でバッファを処理する。セッションが拒否された場合はバッファを破棄する。クライアント側は現行の `H3_MESSAGE_ERROR` を維持する (楽観的送信は client → server 方向のみ)
- バッファは `WtSession.capsule_buf` を再利用する。DoS 対策としてバッファ上限を設け、超過時は `H3_MESSAGE_ERROR` でストリームをリセットする

### 5. フロー制御カプセルの単調性チェック厳密化 (Section 5.6.2 / 5.6.4)

draft-15:

> If an endpoint receives a WT_MAX_STREAMS capsule with a Maximum Streams value less than a previously received value

draft-16:

> If an endpoint receives a WT_MAX_STREAMS capsule that does not increase the Maximum Streams value previously received

変更対象 (すべて `<` を `<=` に変更):

- `src/webtransport/session/mod.rs` の `Session::process_capsule` 内の `MaxData` / `MaxStreams` 分岐
- `src/webtransport/capsule.rs` の `Capsule::validate_max_streams` / `Capsule::validate_max_data` (独立した pub 検証関数。同値を Ok とする既存テスト・PBT も更新する必要あり。`CapsuleValidationError::MaxStreamsDecreased` / `MaxDataDecreased` の variant 名または doc コメントも「減少」→「増加しない」に更新する)

### 6. WT_MAX_STREAMS > 2^60 のエラー種別変更 (Section 5.6.2)

draft-15:

> MUST be treated as an HTTP/3 error of type H3_DATAGRAM_ERROR

draft-16:

> Recipients of a capsule with a Maximum Streams value larger than this limit MUST close the WebTransport session with a WT_FLOW_CONTROL_ERROR error code.

変更対象:

- `src/webtransport/session/mod.rs` の `Session::process_capsule` 内の `MaxStreams` 分岐: `CapsuleProcessError::Connection(H3_DATAGRAM_ERROR)` を `CapsuleProcessError::Session(Error::Protocol(ErrorCode::FlowControlError))` に変更する
- `src/webtransport/capsule.rs` の `CapsuleValidationError::MaxStreamsExceedsLimit` の doc コメント: 「H3_DATAGRAM_ERROR として扱う」を draft-16 のセッションエラーに更新する
- `src/webtransport/session/mod.rs` の `CapsuleProcessError::Connection` の doc コメント: 修正後 `Connection` variant の使用例がなくなる場合は variant 自体の削除を検討する

### 7. WT_STREAMS_BLOCKED > 2^60 の検証追加 (Section 5.6.3)

draft-16 追加要件:

> Recipients of a capsule with a Maximum Streams value larger than this limit MUST close the WebTransport session with a WT_FLOW_CONTROL_ERROR error code.

- `src/webtransport/session/mod.rs` の `Session::process_capsule` 内の `StreamsBlocked` 分岐に `> MAX_STREAMS_LIMIT` の検証を追加し、`CapsuleProcessError::Session(Error::Protocol(ErrorCode::FlowControlError))` を返す

### 8. Application Error Message の受信時検証 (Section 6)

draft-16 追加要件:

> Senders that truncate an application-supplied message MUST do so at a UTF-8 character boundary.
> If the Application Error Message exceeds 1024 bytes or is not valid UTF-8, the receiver MUST reset the stream with code H3_MESSAGE_ERROR.

- 送信側: `Session::close_with_error` と `Error::application` は既に UTF-8 文字境界での切り詰めを実装済み。変更不要
- 受信側: ワイヤー上の挙動は既に draft-16 準拠。`Capsule::decode_payload` が返す `CapsuleDecodeError::Malformed` は `src/connection/wt_capsule.rs` の `process_wt_capsule_data` で `Error::StreamError(ErrorCode::MessageError)` (H3_MESSAGE_ERROR) に変換されている。挙動変更は不要。エラー型の意味論的な区別 (構造的不正 vs アプリケーションエラーメッセージ検証失敗) を追加するかは実装時に判断する

## 完了条件

- 上記 8 件の変更がすべて実装される
- ソースコードのコメントの draft-15 参照が draft-16 に置換される (draft-02/07/14 固有コードのコメントはそのまま)
- README.md の対応 draft バージョン表に draft-16 が追加される
- CHANGES.md の `## develop` セクションに変更履歴が記載される
- 各変更にテストが追加される
- `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` と `cargo test --all` が通る

## 解決方法

項目 1, 2, 5, 6, 7, 8 を実装した。項目 3 (0-RTT 検証) と項目 4 (楽観的カプセル) は設計判断を伴うため別 issue に分割した。

1. `src/connection/mod.rs` の `process_control_stream` に `SETTINGS_WT_ENABLED > 1` の `H3_SETTINGS_ERROR` 検証を追加した
2. `src/webtransport/connect/mod.rs` の Display メッセージと `connect_error.rs` の doc コメントを `"= 1"` に更新した
3. `examples/wt_server/src/main.rs` の拒否ステータスを 404 から 405 に変更した (4 箇所)
4. `src/webtransport/session/mod.rs` の `Session::process_capsule` と `src/webtransport/capsule.rs` の `validate_max_streams` / `validate_max_data` で `<` を `<=` に変更した
5. `CapsuleProcessError::Connection` variant を削除し、WT_MAX_STREAMS > 2^60 を `Session(Error::Protocol(ErrorCode::FlowControlError))` に変更した
6. `StreamsBlocked` 分岐に `> MAX_STREAMS_LIMIT` の検証を追加した
7. 項目 8 はワイヤー挙動が既に draft-16 準拠であることを確認し、変更不要と判断した
8. README.md の対応 draft 表に draft-16 を追加した
9. PBT テストの戦略を単調増加制約に合わせて更新した

### 関連ファイル

- `src/webtransport/session/mod.rs` (`Session::process_capsule`, `Session::close_with_error`, `CapsuleProcessError`)
- `src/webtransport/settings.rs` (`Settings::detect_draft_pattern`, `Settings::is_enabled`)
- `src/webtransport/connect/mod.rs` (`CapabilityError` の Display 実装)
- `src/webtransport/connect/connect_error.rs` (`CapabilityError`, `TransportCapabilities`)
- `src/webtransport/capsule.rs` (`Capsule::decode_payload`, `Capsule::validate_max_streams`, `Capsule::validate_max_data`, `CapsuleValidationError`)
- `src/connection/mod.rs` (`Connection::process_control_stream` - SETTINGS 検証の呼び出し)
- `src/connection/wt_capsule.rs` (`Connection::process_wt_capsule_data`, `Connection::handle_wt_capsule`, `Connection::handle_wt_data_frame`)
- `src/connection/wt_session.rs` (SETTINGS 検証、`establish_wt_session_server`)
- `examples/wt_server/src/main.rs` (405 応答)
- `pbt/tests/prop_capsule/main.rs` (validate_max_streams / validate_max_data の PBT)
- 一次資料: `refs/webtrans/draft-ietf-webtrans-http3-16.txt`
