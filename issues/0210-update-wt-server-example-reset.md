# examples/wt_server が send_response のエラー時に CONNECT ストリームを reset しない

- Created: 2026-09-12
- Completed: {YYYY-MM-DD}
- Branch: feature/update-wt-server-example-reset
- Polished: {YYYY-MM-DD}

## 目的

`send_response` の新しいエラー契約 (2xx 応答のセッション確立時にバッファリング済みカプセルの検証に失敗した場合は `Error::StreamError(MessageError)` を返す。呼び出し側は CONNECT ストリームを H3_MESSAGE_ERROR で reset する) に example を追従させる。

## 現状

- `examples/wt_server/src/webtransport.rs` の `accept` は `send_response(...)?` のエラーをそのまま伝播し、CONNECT ストリームの reset を行わない
- 2xx 応答前に WT_CLOSE_SESSION + 追加 DATA を送る raw ピアに対して、RESET_STREAM(H3_MESSAGE_ERROR) ではなく FIN が送出される (draft-ietf-webtrans-http3-16 Section 6 の MUST を満たさない)
- `examples/wt_server` は `shiguredo_http3` の `ServerConnection` を直接利用する独自統合層で、本リポジトリの `WtSessionRequest::accept` の修正 (0206) の対象外
- 0206 のレビューで、公開 API の契約変更に example が未追随であることが検出された

## 設計方針

- `accept` の `send_response` エラー時に CONNECT ストリームへ `H3_MESSAGE_ERROR` の RESET_STREAM を送ってからエラーを返す
- `shiguredo_http3` の公開 API を利用する (内部ヘルパーを利用できない場合は同等の reset 処理を example 内に実装する)
- 統合テスト (raw ピアで 2xx 前に close + 追加 DATA) を追加できるか検討し、難しい場合は example の修正のみとしてその旨を解決方法に記録する

## 完了条件

- 2xx 前に WT_CLOSE_SESSION + 追加 DATA が届いた場合、example がピアへ RESET_STREAM(H3_MESSAGE_ERROR) を送出する (または example の対象外であることが明確化されている)
- `examples/wt_server` がビルドできる
- 可能なら統合テストでピアが RESET_STREAM を観測することを確認する

## 解決方法

### 関連ファイル

- `examples/wt_server/src/webtransport.rs` (`accept`)
- `examples/wt_server/src/main.rs` (accept 呼び出し側)

### 一次資料

- `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 6 (WT_CLOSE_SESSION 後の追加 DATA は H3_MESSAGE_ERROR で reset)
- `refs/h3/rfc9114.txt` Section 8.1 (HTTP/3 error codes)

### 関連 issue

- 0206 (`WtSessionRequest::accept` の reset 対応。本 issue は example 側の追従)
