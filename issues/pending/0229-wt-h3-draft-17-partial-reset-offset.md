# アプリケーション指定オフセットを RESET_STREAM_AT の Reliable Size に反映する

- Created: 2026-11-03
- Completed: {YYYY-MM-DD}
- Branch: feature/add-wt-draft-17-partial-reset-offset
- Polished: {YYYY-MM-DD}
- Category: add

## 目的

draft-ietf-webtrans-http3-17 Section 4.4 で明確化された、`Reliable Size = 送信方向の WT ヘッダー長 + アプリケーション指定オフセット` を実現する。ヘッダーだけの配送保証に加え、アプリケーションが必要とするペイロード先頭部分も配送できるリセット API を用意する。

## 現状

- `src/connection/wt_stream.rs` の `Connection::wt_stream_header_len` は、登録されたセッション ID からヘッダーのエンコード長を返すだけで、アプリケーションのオフセットを受け取らない
- `src/connection/wt_session.rs` の `terminate_wt_session_with` は、セッション終了用の `WtStreamReset::reliable_size` をヘッダー長で生成する
- `src/event.rs` の `WtStreamReset` は Reliable Size を運べるが、アプリケーションが指定したオフセットから個別ストリームの送信中断を依頼する API はない
- `crates/tokio-s2n-quic/src/webtransport/session.rs` の `WtBiStream` / `WtSendStream` は `send` / `finish` を公開し、オフセット付きリセットを公開しない
- `crates/tokio-ngtcp2/src/h3.rs` の `QuicConnectionRef::reset_stream` は、stream ID と error code だけを下位層へ渡す
- 固定依存 `shiguredo_ngtcp2_tokio = 2026.1.0-canary.4` の配布ソースにある `ClientConnection::reset_stream_reliable` / `AcceptedConnection::reset_stream_reliable` も、stream ID と error code しか受け取らない。送信済みデータの配送保証を依頼できるが、アプリケーションが選んだ部分オフセットは指定できない

## 設計方針

- 送信側中断 API に、WT ペイロード先頭から数える配送保証オフセットを渡せるようにする。省略時は `0` とする
- QUIC の Reliable Size に変換するときだけ、当該送信方向に実際に存在する WT ヘッダー長を加える。ピア開始 bidi の応答方向には WT ヘッダーを加えない
- WT アプリケーションエラーは `ApplicationErrorCode` の既存の変換を使い、セッション終了用のエラーと混同しない
- オフセットが Final Size を超える入力、加算オーバーフロー、QUIC varint の値域超過を拒否する。再リセット時に Reliable Size を増やさない制約とエラーコードの一貫性も下位 API の契約に合わせる
- セッション全体の終了と個別ストリームの中断を区別し、後者でセッションやほかのストリームを終了させない
- 端点実装の範囲に絞る。Section 4.4 の中継時のヘッダー長補正は要件として記録するが、中継機能は追加しない
- 送信側が保証すべき範囲と、受信側がその範囲を超えて配送する可能性を区別する。保証範囲より後ろのデータを、必ず受信できないとは定義しない

## pending の理由

現行の固定依存では、ngtcp2 の Tokio API が任意の Reliable Size / アプリケーション指定オフセットを受け取れない。s2n-quic 1.86 も `RESET_STREAM_AT` の送出 API とピアの transport parameter の取得 API を公開していない。通常の RESET_STREAM へ置き換えたり、サポートフラグを `true` に固定したりしても、本機能の配送保証は満たせないため、必要な下位 API が利用可能になるまで pending とする。

既存の 0224 は `examples/wt_server` が必須 transport parameter とヘッダー配送を満たすための issue、0016 は transport parameter の伝達 API の設計であり、アプリケーション指定オフセットを扱う本 issue とは目的を分ける。

## 完了条件

- 使用する下位 API が、実際のピアの対応状態とアプリケーション指定オフセットを扱える
- オフセット `0` と非ゼロ、複数の session ID の varint 長、ローカル開始 / ピア開始 bidi、送信 uni で Reliable Size が正しい
- 算術変換は PBT、値域外と再リセットの制約は境界値テストで検証する
- 実 QUIC 接続でヘッダーと指定プレフィックスを受信してからリセットを観測し、同じセッションの別ストリームを継続利用できる
- 下位 API が利用できない経路では、未対応として明示し、draft-17 の配送保証を満たしたと報告しない
- 追加する公開 API に実際の使用箇所とテストを用意する

## 関連ファイル

- `src/connection/wt_stream.rs`: `Connection::wt_stream_header_len`
- `src/connection/wt_session.rs`: `Connection::terminate_wt_session_with`
- `src/event.rs`: `WtStreamReset`, `WebTransportEvent::StreamReset`
- `src/webtransport/error.rs`: `ApplicationErrorCode`
- `crates/tokio-ngtcp2/src/h3.rs`: `QuicConnectionRef::reset_stream`
- `crates/tokio-ngtcp2/src/webtransport.rs`: `ClientWebTransportSession`
- `crates/tokio-ngtcp2/Cargo.toml`: 下位 QUIC クレートの固定依存
- `crates/tokio-s2n-quic/src/webtransport/session.rs`: `WtBiStream`, `WtSendStream`

## 一次資料

- `refs/webtrans/draft-ietf-webtrans-http3-17.txt` Section 4.3 / 4.4
- `refs/webtrans/draft-ietf-webtrans-overview-14.txt` Section 4.3
- `refs/quic/draft-ietf-quic-reliable-stream-reset-11.txt` Section 3 / 4 / 5.2
- <https://www.ietf.org/archive/id/draft-ietf-webtrans-http3-17.txt>
- <https://crates.io/api/v1/crates/shiguredo_ngtcp2_tokio/2026.1.0-canary.4/download>: 固定依存の配布物内の `client.rs` / `server.rs` を確認
