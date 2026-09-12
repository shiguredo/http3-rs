# examples/wt_server が WT_MAX_DATA をクライアントへ通知せず WebKit で双方向ストリームの送信が停止する

- Created: 2026-09-12
- Completed: 2026-09-12
- Branch: feature/fix-wt-server-max-data-capsule
- Polished: {YYYY-MM-DD}

## 目的

`examples/wt_server` に接続した WebKit (Safari 相当) が、双方向ストリームへ書き込むと応答が返らず停止する問題を修正する。Chromium では同じ操作が成功するため、ブラウザ間で挙動が異なる状態を解消する。

## 現状

Chromium と WebKit で双方向ストリームのエコーを試すと、WebKit だけが停止する。

| エンジン | 双方向ストリームのエコー |
|---|---|
| Chromium | 成功する |
| WebKit | 書き込みが完了せず停止する (16 KiB でも停止する) |

原因は WebTransport のデータフロー制御である。

- `examples/wt_server` の `build_server_wt_settings` は draft-07 形状のサーバー設定を返す。Safari 26.4 は応答 SETTINGS に `SETTINGS_WT_INITIAL_MAX_*` を含めると `H3_REQUEST_CANCELLED` で CONNECT をリセットするため (`docs/SAFARI_WT.md`)、この形状では `wt_initial_max_data` を広告できない
- `Connection::initialize_session_flow_control` (`src/connection/wt_session.rs`) は、ローカル設定の `wt_initial_max_data` が 0 の場合に `WT_MAX_DATA` カプセルを生成しない。初期カプセルは設定値と同じ値を送出するため、設定値が 0 だと 1 つも生成されない
- その結果、WebKit のクライアントは送信ウィンドウが 0 のままとなり、双方向ストリームへの書き込みが完了しない
- `examples/wt_server` は `Connection::wt_data_consumed` を呼んでいない。データを受信しても `WT_MAX_DATA` によるウィンドウ更新が発生しないため、仮に初期値を通知してもウィンドウは回復しない
- `crates/tokio-s2n-quic` にも `wt_data_consumed` の呼び出しが無い

Chromium では同じサーバーで停止しないため、ブラウザ実装の差によって症状が隠れていた。

## 設計方針

- `Connection::initialize_session_flow_control` が、ローカル設定の `wt_initial_max_data` に依存せず初期の `WT_MAX_DATA` を送出できるようにする。draft-07 形状のように SETTINGS で広告できない場合でも、セッション確立後のカプセルで通知する必要がある
- `examples/wt_server` がデータを受信した時点で `Connection::wt_data_consumed` を呼び、消費に応じた `WT_MAX_DATA` の更新を送出する
- `crates/tokio-s2n-quic` の `WtSession` も同様に、受信データの消費を `wt_data_consumed` へ伝える
- 窓の回復量と閾値 (どの程度消費したら更新を送るか) は実装時に確定する

## 完了条件

- WebKit から `examples/wt_server` へ接続し、初期フロー制御窓を超える双方向ストリームのエコーが成功する
- Chromium でも従来どおり成功する
- `make interop-test-browser` の検証項目に「大きいデータの双方向転送」を追加できる
- 既存の `cargo test --workspace --tests` / `make interop-test` が通る
- `cargo fmt --all -- --check` と `cargo clippy --workspace --all-targets -- -D warnings` が通る

## 解決方法

0217 (WebKit から双方向ストリームへ 4 KiB 以上を書き込むと完了しない) に統合して closed にする。

### 統合の理由

本 issue は起票時に「WebKit の送信ウィンドウが 0 のままになるため停止する」と診断したが、実測でこの診断が誤りであることが判明した。

計測した結果:

| データサイズ | WebKit |
|---|---|
| 1 KiB | 成功する |
| 4 KiB | 停止する |
| 16 KiB | 停止する |
| 256 KiB | 停止する |
| 512 KiB | 停止する |

- 閾値は 4 KiB であり、WebTransport のデータフロー制御の窓より小さい。窓が 0 であることが原因なら 1 KiB も停止するはずである
- サーバーがセッション確立直後に送る `WT_MAX_STREAMS` / `WT_MAX_DATA` カプセルの値を変えても、閾値は 1 バイトも変化しなかった。カプセルの値は停止の有無に影響していない
- 停止時、サーバーは双方向ストリームのデータを 1 バイトも受信していない。CONNECT ストリームの受信は成功している

このため、原因は WebTransport のデータフロー制御ではなく、原因自体が未確定である。停止がこのリポジトリ側の問題か WebKit 側の問題かも切り分けられていない。

同一の症状を 0217 で扱っているため、2 つの issue に分けておく意味がない。実測の記録は `docs/WEBKIT_WT.md` にあり、0217 から参照している。

### 起票時に想定した修正について

`Connection::initialize_session_flow_control` が SETTINGS とは独立した上限で初期カプセルを生成できるようにする変更を実装して検証したが、WebKit の閾値は変化しなかった。停止の原因ではないため破棄した。

なお「SETTINGS で `SETTINGS_WT_INITIAL_MAX_*` を広告できない draft 形状では初期クレジットのカプセルが生成されない」という記述自体はコード上そのとおりである。ただし本 issue の症状の原因ではない。

### 関連ファイル

- `src/connection/wt_session.rs` (`Connection::initialize_session_flow_control` / `Connection::wt_data_consumed`)
- `src/connection/wt_types.rs` (`WtSession::initialize_flow_control` / `WtSession::on_data_consumed`)
- `examples/wt_server/src/webtransport.rs` (`WtSessionRequest::accept` / `WtBiStream`)
- `examples/wt_server/src/main.rs` (`handle_bidi_echo` / `handle_uni_echo`)
- `crates/tokio-s2n-quic/src/webtransport/session.rs` (`WtSession::consume_data`)
- 一次資料: `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 5.5, 5.6
- 検証手段: `interop/browser` (Chromium / WebKit の Playwright テスト)
- 実測の記録: `docs/WEBKIT_WT.md`

### 関連 issue

- 0217 (統合先。原因の確定と修正を扱う)
