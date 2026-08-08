# RESET_STREAM 受信時にフロー制御クレジットが計上・回復されない

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-wt-reset-flow-control-credit
- Polished: 2026-08-08

## 目的

ピアがデータストリームを RESET_STREAM で閉じた場合の WT フロー制御クレジット (データ量・ストリーム数) の計上漏れを修正する。

## 現状

- `src/connection/wt_session.rs` の `Connection::handle_wt_stream_reset` は `StreamReset` イベントの発火と `wt_uni_streams` / `wt_bidi_streams` からの除去、`WtSession::disassociate_stream` を行うだけで、以下のフロー制御処理をしていない
  - RESET_STREAM の final_size から WT ストリームヘッダー長を引いた量を受信側データフロー制御に計上しない。受信側は実際に消費されたクレジットを過少計上し、データ量制限の超過検知 (draft-16 Section 5.6.4 の「The sum of the lengths of Stream Body data sent on all streams associated with this session MUST NOT exceed the Maximum Data value advertised by a receiver」) が機能しなくなる
  - `WtSession::on_remote_stream_closed` を呼ばず、`WT_MAX_STREAMS` のクレジットが返却されない。ピアがストリームを使い捨て (開いて即 RESET) すると受信側ウィンドウが枯渇し、以後の正常なストリームが `WT_FLOW_CONTROL_ERROR` でセッション終了する (draft-16 Section 5.6.2)
- FIN 経路 (`BidiStreamEnd` / `UniStreamEnd`) では `on_remote_stream_closed` が呼ばれており、RESET 経路だけが非対称

## 設計方針

- **`handle_wt_stream_reset` で final_size を受信側データ FC に計上する。計上量は「`final_size - wt_stream_header_len` - そのストリームで既に計上済みのボディ量」の残差とする**。受信ペイロードはチャンク受信時に `add_received_data` で逐次計上済みのため (wt_stream.rs、およびセッション確立時の `deliver_buffered_streams` 経路 (wt_session.rs) も含む)、final_size 全体をそのまま計上すると二重計上になり、正規のピアを `check_received_data` が拒否して `WT_FLOW_CONTROL_ERROR` でセッション終了する。per-stream の計上済みボディ量を `WtSession` (または `Connection`) に追跡する必要がある
  - **データ FC 計上は「ピアが送信方向を持つ全ストリーム」を対象とする** (ピア開始 uni / bidi + ローカル開始 bidi のピア送信方向。WT_MAX_DATA の制限はストリーム開始者を問わず「all streams associated with this session」を数える (draft-16 Section 5.6.4) ため、0144 で登録されるローカル開始 bidi でもピアが自分の送信方向を RESET した場合は計上が必要)
  - **ヘッダー減算はピアがヘッダーを送った場合のみ適用する** (ピア開始ストリーム)。ローカル開始 bidi ではヘッダー (0x41 + session ID) は開始側 (ローカル) が送るためピアの final_size に含まれず、`wt_stream_header_len` を引くと過少計上になる (ローカル開始 bidi は減算 0)。ピア開始 / ローカル開始の判定はストリーム ID の下位 2 ビット + ロールで行う (0144 と同じ方式)
  - RESET 計上時も通常受信経路と同様にデータ FC の超過検出を行う (超過時は `WT_FLOW_CONTROL_ERROR` でセッション終了。draft-16 Section 5.6.4 の「If an endpoint receives Stream Body data in excess of this limit, it MUST close the WebTransport session with a WT_FLOW_CONTROL_ERROR error code」)
  - **RESET されたストリームのデータはアプリに配送されないため、残差 (計上した量) を `on_data_consumed` 相当の自動消費として扱い、ウィンドウ (WT_MAX_DATA 再広告) を回復する**。自動消費する量は残差のみとし、既計上分 (受信済みで配送済みのボディ量) はアプリの消費通知 (`on_data_consumed`) に委ねる (二重回復を防ぐ)。自動消費の根拠は RFC 9000 Section 19.4 の「A receiver of RESET_STREAM can discard any data that it already received on that stream」
  - `wt_stream_header_len` の計算は `wt_uni_streams` / `wt_bidi_streams` からの除去**前**に行う (除去後はマップに無いため 0 を返す。`terminate_wt_session_with` と同じ「計算 → 除去」の順序)
  - `final_size` がヘッダー長未満の場合は `saturating_sub` 等でアンダーフローを防ぐ。受信済み量が final_size を超えるケースは QUIC 層の不正 (RFC 9000 Section 4.5 の FINAL_SIZE_ERROR) であり、WT 層では残差 0 (二重計上なし) として扱う
  - **per-stream の計上済みボディ量の追跡エントリは、ストリームの FIN / RESET / セッション終了時に掃除する** (掃除しないと長時間 Established のセッションでストリーム数に比例して無制限に成長し、0139 の方針に反する)
- **リセットされたストリームに対しても `on_remote_stream_closed` を呼びストリーム数クレジットを回復する。ただしローカル開始ストリームは対象外** (WT_MAX_STREAMS はピアが開くストリーム数の制限であり、0144 で登録されるローカル開始 bidi ストリームの RESET で呼ぶとクレジットを誤返却する。ピア開始ストリームのみ `on_remote_stream_closed` を呼ぶ)
- **スコープ: Established / Draining セッションのストリームを対象とする** (Draining は既存ストリームの送受信とカプセル受信が継続できるため、FIN 経路と同様に RESET でもクレジットを回復する)。Pending セッション (バッファリング中) のストリームは `wt_uni_streams` / `wt_bidi_streams` に未登録のため RESET 処理の対象外となり、`buffered_stream_entries` の掃除は 0146 (バッファリング中 WT ストリームの stale エントリ) のスコープとする。0146 と同じ関数 (`handle_wt_stream_reset`) を変更するため、実装順序を調整する。0139 (tombstone による `wt_sessions` 除去) 実装後はセッション不在 = スキップになるため、0139 後の実装順序を想定する (0139 前の実装では Closed 状態のセッションの RESET で FC 計上が走らないようガードする)。ローカル開始 bidi の登録は 0144 に依存するため、実装順序は 0144 → 0145 を想定する

## 完了条件

- RESET_STREAM 受信時に、ピアが送信方向を持つストリーム (ピア開始 uni / bidi + ローカル開始 bidi のピア送信方向) のデータ FC (final_size - ヘッダー長 - 既計上分。ローカル開始 bidi はヘッダー減算なし) が計上され、残差が自動消費されてウィンドウが回復される (二重計上・二重回復されない)
- ピア開始ストリームの RESET で `WT_MAX_STREAMS` クレジットが回復される (ローカル開始ストリームは対象外)
- RESET 計上によるデータ FC 超過が `WT_FLOW_CONTROL_ERROR` で検出される
- テストが追加される: フロー制御有効セッションの構築ヘルパーを用意し、「データ受信 → RESET (final_size > 受信済み)」で二重計上が起きないこと、「RESET で WT_MAX_STREAMS が回復されること (ローカル開始 bidi では誤回復されないこと)」、「ピア開始ストリームで final_size - ヘッダー長が計上されること、ローカル開始 bidi で減算 0 であること」、「RESET の自動消費で WT_MAX_DATA カプセル (再広告値の増加) が発行されること」、「データ FC 超過の RESET でセッション終了すること」を検証する
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/connection/wt_session.rs` (`Connection::handle_wt_stream_reset` / `WtSession::on_remote_stream_closed` の呼び出し追加 / `deliver_buffered_streams` の計上経路)
- `src/connection/wt_types.rs` (`WtSession::on_remote_stream_closed` の定義 / per-stream の計上済みボディ量の追跡)
- `src/connection/wt_stream.rs` (`Connection::wt_stream_header_len` / 計上済み量の記録箇所)
- 関連 issue: 0144 (ローカル開始 bidi ストリームの登録。RESET 時の `on_remote_stream_closed` 対象外。実装順序は 0144 → 0145)、0146 (バッファリング中ストリームの stale エントリ。同一関数を変更)、0139 (ストリームのリソースリーク。tombstone 等で RESET 経路に触れる)
- 一次資料: `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 5.3 / 5.4 / 5.6.2 / 5.6.4、`refs/quic/rfc9000.txt` Section 4.5 / 19.4 (final size と RESET_STREAM の定義)
