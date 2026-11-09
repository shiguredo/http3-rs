# 受信 StreamReset の doc に Reliable Size 非搭載の理由と final_size の方向依存を記録する

- Created: 2026-11-09
- Completed: {YYYY-MM-DD}
- Branch: feature/update-wt-stream-reset-reliable-size-doc
- Polished: 2026-11-10
- Category: doc

## 目的

`WebTransportEvent::StreamReset` が受信リセットに `Reliable Size` を載せない理由を `src/event.rs` の doc コメントに記録する。
`SessionClosed.reset_streams` の `WtStreamReset.reliable_size` は送信指示であるのに対し、`StreamReset` は受信通知であり、
両者は役割が異なる。受信通知に Reliable Size を載せる仕様上の要求も、値を供給できる下位 API もない。
併せて、同じ variant の `final_size` の doc が方向非依存になっている誤りを修正する。
理由が残っていないと、将来「対称性のため」に値を供給できないフィールドを追加する変更が再提案される。

## 現状

- `WebTransportEvent::StreamReset` は `session_id` / `stream_id` / `error_code` / `final_size` の 4 フィールドで、
  Reliable Size を運ばない。発火点は `Connection::handle_wt_stream_reset`
- `WebTransportEvent::SessionClosed.reset_streams` の `WtStreamReset` は `stream_id` と `reliable_size` を運ぶ。
  `Connection::terminate_wt_session_with` が計算し、`RESET_STREAM_AT` に渡す送信指示である
- draft-17 Section 4.4 は、送信者に `RESET_STREAM_AT` の使用と
  Reliable Size = その送信方向の WT ヘッダー長 + アプリ指定オフセットを MUST とし、
  中継者に同一 frame 型の送出とヘッダー長差の補正を MUST とする。
  受信者へ Reliable Size を引き渡す要求はなく、受信側の MUST は `WT_APPLICATION_ERROR` 範囲のエラーコードを
  変更せずアプリへ配送することである
- Reliable Size は方向依存である。WT ヘッダーは双方向ストリームの開始側だけが送るため、
  ローカル開始双方向ストリームの応答方向をピアがリセットする場合、ピアが送る Reliable Size はアプリ指定オフセットのみで、
  0 も有効値である (draft-ietf-quic-reliable-stream-reset-11 Section 5)
- 同 Section 5.2 は、イニシエータが同じストリームに複数の `RESET_STREAM_AT` を送って Reliable Size を減らせること、
  パケットの並び替えで大きい値が後から届いても受信側は増加を無視して最小値を使うことを定める
- `src/event.rs` の `StreamReset.final_size` の doc は、この値が stream header のバイト数以上であることを
  方向非依存に期待している。しかし `Connection::account_wt_stream_reset` は、ローカル開始双方向ストリームでは
  開始側がヘッダーを送るためピアの `final_size` にヘッダーが含まれないものとして扱っており、doc と矛盾する
- 固定依存の両アダプタは受信 Reliable Size を公開しない
  - `shiguredo_ngtcp2_tokio 2026.1.0-canary.4` の `ConnectionEvent::StreamReset` は
    `stream_id` / `final_size` / `app_error_code` のみ。下位の `ngtcp2_stream_reset` コールバックも
    `stream_id` / `final_size` / `app_error_code` だけを受け取り、Reliable Size を渡さない
  - `s2n-quic 1.91` / `s2n-quic-core 0.91` は `RESET_STREAM_AT` frame を持たず、
    `StreamError::StreamReset` は `error` 以外にプロトコルデータを持たず、Final Size も公開しない
- closed issue 0054 は、受信イベントに `final_size` を追加し、`reliable_size` は `SessionClosed.reset_streams` にだけ
  追加する判断をしている。現在の非対称はこの決定に由来する
- `CODEBASE.md` は、使用箇所とテストを持たない公開 API の追加を禁じている。
  `Some` を供給できる経路がない `reliable_size: Option<u64>` はこの要件を満たせない

## 設計方針

- `src/event.rs` の `WebTransportEvent::StreamReset` の doc に、`reliable_size` を搭載しない理由を記録する
  - draft-17 Section 4.4 は受信者への Reliable Size 引き渡しを要求しない
  - 固定依存の両アダプタが受信 Reliable Size を公開しない
  - 値を供給できない公開フィールドは `CODEBASE.md` の公開 API 要件を満たさない
  - Reliable Size は送信方向に WT ヘッダーがある場合のみヘッダー長を含み、複数 frame では最小値のみ有効であるため、
    単純な対称フィールドとして扱えない
- 同じ doc で、`SessionClosed.reset_streams` の `WtStreamReset.reliable_size` は送信指示であり、
  本イベントの受信通知とは役割が異なることを明記する
- `StreamReset.final_size` の doc を方向依存に修正する。ピアが送信方向に WT ヘッダーを送っている場合は
  ヘッダー長以上になるが、ローカル開始双方向ストリームの応答方向ではヘッダーを含まない
- 再検討条件を `StreamReset` の doc に書く。
  両アダプタが受信 Reliable Size を公開し、かつ受信アプリが宣言値そのものを必要とするユースケースが生じた場合は、
  `StreamReset` へのフィールド追加と `Connection::stream_reset` の引数追加を破壊的変更として許容して再検討する
- doc コメントに issue 番号を書かず、理由そのものを書く
- 本 issue は受信通知である `StreamReset` の doc に限定し、送信指示である `WtStreamReset` の doc は変更しない
- コードと公開 API は変更しない。`docs/WT_HTTP3.md` は送信側の Reliable Size を記載しており、
  受信側の非搭載と矛盾しないため変更しない

## 完了条件

- `WebTransportEvent::StreamReset` の doc に、非搭載理由 4 点と再検討条件が書かれている
- `SessionClosed.reset_streams` の `WtStreamReset.reliable_size` が送信指示であることと、
  受信通知との役割差が `WebTransportEvent::StreamReset` の doc に書かれている
- `StreamReset.final_size` の doc が方向依存を反映し、`Connection::account_wt_stream_reset` の扱いと一致している
- 既存の doc (`SessionClosed` の説明) と矛盾しない
- `cargo fmt --all -- --check` と
  `cargo clippy --workspace --exclude interop_h3 --exclude interop_wt --all-targets -- -D warnings` と
  `cargo test --workspace --exclude interop_h3 --exclude interop_wt` が通る

## 関連ファイル

- `src/event.rs`: `WebTransportEvent::StreamReset`, `WtStreamReset`, `WebTransportEvent::SessionClosed`
- `src/connection/mod.rs`: `Connection::stream_reset` (再検討時に引数の追加対象になる)
- `src/connection/client.rs` / `src/connection/server.rs`: `stream_reset` (再検討時に同じく追加対象になる)
- `src/connection/wt_session.rs`: `Connection::handle_wt_stream_reset`,
  `Connection::terminate_wt_session_with`, `Connection::account_wt_stream_reset`
- `src/connection/wt_stream.rs`: `Connection::wt_stream_header_len`
- `crates/tokio-ngtcp2/src/h3.rs`: `ConnectionEvent::StreamReset` のハンドリングと `stream_reset` 呼び出し
- `crates/tokio-s2n-quic/src/internal/connection_state.rs`: `stream_reset_only`
- `crates/tokio-ngtcp2/Cargo.toml` / `crates/tokio-s2n-quic/Cargo.toml`: 固定依存のバージョン
- `crates/tokio-ngtcp2/README.md` / `crates/tokio-s2n-quic/README.md`: `reset_stream_at` の制約の記載

## 一次資料

- `refs/webtrans/draft-ietf-webtrans-http3-17.txt` Section 4.4 (L809-887): 送信者 MUST、中継者 MUST、
  受信側は `WT_APPLICATION_ERROR` 範囲のエラーコード配送の MUST のみ
- 同 Section 4.3 (L754-807): WT ヘッダーは双方向ストリームの開始側だけが送ること (方向依存の根拠)
- `refs/quic/draft-ietf-quic-reliable-stream-reset-11.txt` Section 4 (L190-248): RESET_STREAM_AT frame と
  Reliable Size フィールド
- 同 Section 5 (L250-275): Reliable Size の 0 が有効値であること
- 同 Section 5.2 (L298-320): 複数 frame、減少のみ、受信側は最小値を使うこと
- `issues/closed/0054-bug-wt-reset-stream-at-final-size.md`: 受信 `final_size` と送信 `reliable_size` の先行決定
- `issues/pending/0229-wt-h3-draft-17-partial-reset-offset.md`: 送信側のアプリケーション指定オフセット
  (本 issue とは目的が異なる)
- 固定依存の配布ソース: `shiguredo_ngtcp2_tokio 2026.1.0-canary.4` の `src/lib.rs` と
  `shiguredo_ngtcp2_sys 2026.1.0-canary.4` の `src/bindings.rs`、`s2n-quic-core 0.91.0` の `src/stream/error.rs`
  - <https://crates.io/api/v1/crates/shiguredo_ngtcp2_tokio/2026.1.0-canary.4/download>
  - <https://crates.io/api/v1/crates/s2n-quic-core/0.91.0/download>
