# WT 未ネゴシエーション時の 0x54 uni stream 受信が接続エラーになる

- Created: 2026-08-08
- Completed: 2026-08-14
- Branch: feature/fix-wt-uni-stream-negotiation-error
- Polished: 2026-08-08

## 目的

RFC 9114 Section 6.2 の MUST 違反 (未知ストリームタイプを接続エラーにしている) を修正する。

## 現状

- `src/connection/mod.rs` の `Connection::handle_new_unidirectional_stream` はストリームタイプ 0x54 (WT uni stream) を受信したとき、`Connection::is_wt_fully_negotiated()` が false なら `Error::ConnectionError(ErrorCode::StreamCreationError)` を返し**接続全体**をエラーにする
- RFC 9114 Section 6.2「The recipient MUST NOT consider unknown stream types to be a connection error of any kind」に違反。0x54 は既知のストリームタイプだが (draft-16 Section 4.2)、ネゴシエーション未完了時は受信側が意味論を消費できないため「stream type that is not supported by the recipient」(RFC 9114 Section 6.2) に該当し、unknown stream type として扱うのが正しい。ストリーム単位の拒否 (abort) か破棄 (discard) が正しい

> Recipients of unknown stream types MUST either abort reading of the stream or discard incoming data without further processing. If reading is aborted, the recipient SHOULD use the H3_STREAM_CREATION_ERROR error code or a reserved error code (Section 8.1).

- サーバーがクライアントの SETTINGS より先に同一フライトで受信しうる 0x54 で接続が死ぬ経路でもある (draft-16 Section 4.6 は同一フライト送信と順序入れ替えを想定)
- `src/connection/mod.rs` の inline テスト `test_wt_uni_stream_disabled_returns_error` がこの違反を期待値として固定化している
- closed 0044 (WT ネゴシエーションチェックの不整合) の解決方法は「uni stream は `H3_STREAM_CREATION_ERROR` 相当でストリームエラーにする」と決定済みだったが、現在の実装は接続エラーのままであり、本 issue はその決定の**実装漏れ修正**の位置づけ

## 設計方針

- 0x54 をネゴシエーション未完了時に受信したら、**ストリームエラー `Error::StreamError(ErrorCode::StreamCreationError)` を返す方式に確定する** (破棄は採らない)
  - 理由 1: closed 0044 の過去判断「H3_STREAM_CREATION_ERROR 相当でストリームエラーにする」と整合する
  - 理由 2: RFC 9114 Section 6.2 は abort を MUST の選択肢として挙げ、abort 時のエラーコードに H3_STREAM_CREATION_ERROR を SHOULD で推奨する
  - 理由 3: 破棄方式は `ignored_uni_streams` にストリーム ID を登録するだけの実装になり、エントリが除去されず無制限に肥大化する (既存の `_ =>` 分岐 (未知タイプ) と同じ性質の無制限成長を 0x54 にも持ち込む)
  - ストリームエラーは `feed_stream` が `Err` を返すことで通知され、RESET_STREAM / STOP_SENDING の送信は QUIC 統合層の責務とする (error.rs の「ストリームエラー (ストリームをリセットすべき)」)。ストリームエラー後も stream_id はどのマップにも登録されないため、統合層が RESET を送るまでの間に後続データが届くと再び 0x54 として再解釈され、同じストリームエラーが返る (破棄にはならない)。RESET 後の後続データは QUIC 層が破棄する。現状の統合層 (tokio-s2n-quic 等) に StreamError → RESET_STREAM / STOP_SENDING の変換実装は存在しないため、統合層側の対応は本 issue のスコープ外とする (必要なら別 issue)
- **バッファリングは採らない**。draft-16 Section 4.6 の「SHOULD buffer streams and datagrams」は推奨であり MUST ではない。正当な同一フライトで送られた 0x54 は失われるが、RFC 9114 Section 6.2 の abort は MUST で許容される。バッファリング実装 (0147 の CONNECT 保留と同様の方式) は本 issue のスコープ外とし、必要なら別 issue で検討する
- `is_wt_fully_negotiated()` が false になるケース (ピアが WT を広告していない / ピアは WT 対応だが peer SETTINGS 未着 等、実装の false 条件は他にも複数ある) は区別せず、どちらも同じストリームエラーで対応する
- テストを仕様準拠の期待値に修正する
- 変更しない経路: クライアントが未ネゴシエーションで server-initiated bidi stream を受信した場合の接続エラー (`feed_stream` 内の server-initiated bidi 分岐) は、RFC 9114 Section 6.1「Clients MUST treat receipt of a server-initiated bidirectional stream as a connection error of type H3_STREAM_CREATION_ERROR unless such an extension has been negotiated」により接続エラーのままが正しいため対象外

## 完了条件

- WT 未ネゴシエーション時の 0x54 受信で `feed_stream` が `Err(Error::StreamError(ErrorCode::StreamCreationError))` を返し、接続は閉じない (ConnectionError を返さない)
- テストが修正・追加される: `test_wt_uni_stream_disabled_returns_error` は未ネゴシエーション一般のケースを代表するテストになるため、`test_wt_uni_stream_not_negotiated_returns_stream_error` 等に改名し、期待値を `Error::StreamError(ErrorCode::StreamCreationError)` に修正する。ストリームエラー後の後続データ・FIN の扱いを検証するテストを追加する (後続データが 0x54 の varint エンコーディング (例: `[0x40, 0x54, ...]`) の場合、再び 0x54 として再解釈され同じストリームエラーが返ること。データなしの FIN のみの場合はストリームエラーにならず `Ok(())` を返すこと)。ストリームエラー後も接続は生存し、別ストリーム (制御ストリーム等) の `feed_stream` が成功することを検証する
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/connection/mod.rs` (`Connection::handle_new_unidirectional_stream` の 0x54 分岐 / `Connection::handle_unidirectional_stream` の FIN 処理 / テスト群)。0x54 分岐のコメントを draft-16 に合わせて更新した
- 一次資料: `refs/h3/rfc9114.txt` Section 6.2 (unknown stream type の扱い)。draft-16 Section 4.6 はバッファリング非採用の判断の参考として参照
- 経緯参照: `issues/closed/0044-bug-wt-negotiation-check-inconsistency.md` (ストリームエラーにする決定)、`issues/closed/0008-bug-webtransport-uni-stream-0x54-ignored.md` (0x54 専用分岐導入時に接続エラーを導入。nghttp3 追随の経緯。本 issue は RFC 準拠を優先し nghttp3 と異なる挙動を採る)

### 修正内容

- `Connection::handle_new_unidirectional_stream` の 0x54 分岐で、`is_wt_fully_negotiated()` が false の場合に返すエラーを `Error::ConnectionError(ErrorCode::StreamCreationError)` から `Error::StreamError(ErrorCode::StreamCreationError)` に変更した (RFC 9114 Section 6.2 の「unknown stream type は接続エラーにしてはならない」MUST NOT への適合。abort 方式を採用)
- `Connection::handle_unidirectional_stream` の FIN 処理に `pending_uni_streams.remove(&stream_id)` を追加し、ストリームタイプ varint 未完のまま FIN が来た場合にバッファを破棄するようにした (RFC 9114 Section 6.2 の「ヘッダー受信前に閉じられた単方向ストリームは許容」)
- `handle_new_unidirectional_stream` の BufferTooShort 分岐でも fin 指定時にバッファを破棄するようにした (同一チャンクで varint 未完 + FIN が届いた場合のリーク防止)
- 0x54 分岐のコメントを draft-15 表記から draft-16 に更新し、バッファリング非採用の根拠 (RFC 9114 Section 6.2 の MUST が定める 2 択のうち abort 方式を採用) を明記した

### 追加・修正したテスト

- `test_wt_uni_stream_not_negotiated_returns_stream_error`: `test_wt_uni_stream_disabled_returns_error` を改名し、期待値を `Error::StreamError(ErrorCode::StreamCreationError)` に修正
- `test_wt_uni_stream_not_negotiated_followup_data_returns_stream_error`: ストリームエラー後の後続データが 0x54 の varint エンコーディングで始まる場合、同じストリームエラーが返ることを検証
- `test_wt_uni_stream_not_negotiated_fin_only_is_ok`: データなしの FIN のみは Ok(()) を返すことを検証
- `test_wt_uni_stream_not_negotiated_partial_type_then_fin_is_ok`: varint 未完バッファの後に FIN が来た場合、バッファを破棄して Ok(()) を返すことを検証
- `test_wt_uni_stream_not_negotiated_partial_type_with_fin_is_ok`: 同一チャンクで varint 未完 + FIN が来た場合もバッファを破棄することを検証
- `test_wt_uni_stream_not_negotiated_split_type_returns_stream_error`: varint 分割到着でも未ネゴシエーションの 0x54 はストリームエラーになることを検証
- `test_wt_uni_stream_not_negotiated_error_keeps_connection_alive`: ストリームエラー後も接続は生存し、制御ストリームの `feed_stream` が成功することを検証
