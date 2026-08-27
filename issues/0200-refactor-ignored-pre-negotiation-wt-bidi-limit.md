# `ignored_pre_negotiation_wt_bidi` / `ignored_uni_streams` に上限を導入する

- Created: 2026-08-28
- Completed: {YYYY-MM-DD}
- Branch: feature/refactor-ignored-pre-negotiation-wt-bidi-limit

## 目的

`Connection::ignored_pre_negotiation_wt_bidi` および `Connection::ignored_uni_streams` の HashSet に上限 (LRU or サイズ上限) を導入し、悪意あるピアが継続的にストリームを開いて拒否させることでメモリを枯渇させる経路を防ぐ。

## 現状

- `src/connection/mod.rs` の `Connection::ignored_pre_negotiation_wt_bidi: HashSet<u64>` は接続終了まで永続化し、上限なし
  - 0178 で追加された。SETTINGS 未受信時の 0x41 bidi 保留上限超過拒否、WT 非対応 SETTINGS 受信時の保留破棄、RESET_STREAM で保留エントリを破棄した際に stream_id を記録する
  - 後続チャンクの誤解釈防止のため保持は必要だが、post-SETTINGS の WT 非対応が確定した後にピアが 0x41 bidi を送り続けると `ignored` の要素数が線形に増加する経路が新設されている
- `Connection::ignored_uni_streams: HashSet<u64>` も同様に永続化・上限なし。RFC 9114 Section 6.2 の未知ストリームタイプ受信時に stream_id を記録する
- draft-16 Section 4.6 の「MUST limit the number of buffered streams and datagrams」は保留と拒否記録の両方に適用されると読める

## 設計方針

- `ignored_pre_negotiation_wt_bidi` と `ignored_uni_streams` の両者に「最大保持数」を導入する
- 実装方針の選択肢:
  - a) LRU (`hashlink::LinkedHashSet` 等の依存追加)
  - b) 単純な最大数 + 到達時に古いエントリを破棄する `VecDeque<u64>` + `HashSet<u64>` の 2 段構造
  - c) 保持しない (別方式でエラー処理する)
- 依存追加を避けるなら b が有力。上限値は既存の `MAX_BUFFERED_STREAMS` (100) と同程度が妥当
- 上限に達したときの挙動: 最も古い stream_id を削除して新規を挿入する。削除された stream_id へ後続チャンクが到着した場合は誤解釈されうるが、SETTINGS 受信済みの状態では varint 判定が働くため、影響は限定的

## 完了条件

- `ignored_pre_negotiation_wt_bidi` と `ignored_uni_streams` の両者に上限が導入され、無制限成長しない
- 上限到達時の挙動をテストで固定する
- `cargo test --all` と `cargo fmt --all -- --check` と `cargo clippy --all-targets --all-features -- -D warnings` が通る

## 解決方法

### 関連ファイル

- `src/connection/mod.rs` (フィールド定義・`buffer_pre_negotiation_wt_bidi` / `dispatch_client_bidi_stream` / `process_pending_wt_bidi_pre_negotiation` の insert 箇所)
- `src/connection/wt_session.rs` (`handle_wt_stream_reset` / `handle_wt_stop_sending` の insert 箇所)

### 一次資料

- `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 4.6 (MUST limit)
- `refs/h3/rfc9114.txt` Section 6.2

### 関連 issue

- 0178 (本 issue の起源。post-SETTINGS で継続的に成長する新規パスの発見)
