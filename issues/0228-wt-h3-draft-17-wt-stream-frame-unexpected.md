# 不正な位置の WT_STREAM を H3_FRAME_UNEXPECTED にする

- Created: 2026-11-03
- Completed: {YYYY-MM-DD}
- Branch: feature/change-wt-draft-17-frame-unexpected
- Polished: 2026-11-03
- Category: change

## 目的

draft-ietf-webtrans-http3-17 Section 4.3 に従い、不正な位置や方向で受信した `WT_STREAM (0x41)` の接続エラーを `H3_FRAME_ERROR (0x106)` から `H3_FRAME_UNEXPECTED (0x105)` に変更する。許可される位置も、双方向ストリームの開始者が送る最初のバイト列であることが明記された。

## 現状

- `src/stream/request.rs` の `RequestStream::process_raw` は、先頭以外、またはクライアントが受信するレスポンス方向の `WT_STREAM` に `ErrorCode::FrameError` を返す
- `src/stream/control.rs` の `ControlStreamRecv::process` も、SETTINGS 受信後の `WT_STREAM` に `ErrorCode::FrameError` を返す
- 両モジュールのテストが、draft-16 に基づく `FrameError` を期待する
- `src/frame/encoder.rs` の `encode_unknown_frame` は、既に通常の HTTP/3 フレームとしての `0x41` 送信を拒否する
- 登録済みローカル開始 bidi の受信方向には WT ヘッダーが付かず、アプリケーションデータを受信する。そこでのペイロード中の `0x41` は WT_STREAM フレームではない

## 設計方針

- 不正位置で WT_STREAM 型を受信した経路を `ConnectionError(FrameUnexpected)` にする
- 正当な開始者の先頭シグナルと、HTTP/3 フレームとして不正位置に現れる `0x41` を区別する
- WT_STREAM は length フィールドを持たないため、不正な型と判定できる段階で HTTP/3 フレームの length / payload 全体を待たない。分割入力でも判定を一貫させる
- 制御ストリームの先頭 SETTINGS 必須規則など、ほかの違反が競合する場合は、RFC 9114 のエラー選択と既存の優先順位を確認する。`H3_MISSING_SETTINGS` の経路を機械的に変更しない
- 一般のフレーム途中切れや `0x41` 以外の不正シグナルのエラーを、一律に置き換えない
- 不正位置の WT_STREAM に対するエラーは、本文版、旧 draft、WT 未ネゴシエーション時で共通して `H3_FRAME_UNEXPECTED` に更新する。旧版の `H3_FRAME_ERROR` というエラーコードの互換性は維持しない。版を保持しない `RequestStream` / `ControlStreamRecv` の共通処理として変更し、版別判定のための API は追加しない。正当な旧版のストリーム処理は維持する

## 完了条件

- 本文版、旧 draft、WT 未ネゴシエーション時のいずれでも、リクエストの途中、レスポンス方向の先頭 / 途中、SETTINGS 受信後の制御ストリームで、不正な WT_STREAM が `H3_FRAME_UNEXPECTED` になる
- WT_STREAM 型の varint が分割された入力と、後続の length が存在しない入力を検証する
- クライアント開始 / サーバー開始の正当な bidi は引き続き使える
- 登録済み bidi の逆方向に送られる通常データが、値 `0x41` を含んでも誤拒否されない
- 一般のフレーム途中切れ、SETTINGS 未受信、通常の未知フレームの既存動作を維持する
- 該当する単体テストと接続 API 経由のテストが通る

## 関連ファイル

- `src/stream/request.rs`: `RequestStream::process_raw`, `test_request_stream_*wt_stream*`
- `src/stream/control.rs`: `ControlStreamRecv::process`, `test_control_stream_recv_wt_stream_*`
- `src/connection/mod.rs`: 双方向ストリームの振り分け
- `src/connection/wt_stream.rs`: `handle_wt_bidi_stream`, `register_local_wt_stream`
- `src/frame/encoder.rs`: `encode_unknown_frame`
- `tests/test_connection.rs`, `tests/test_webtransport_draft_connect.rs`

## 一次資料

- `refs/webtrans/draft-ietf-webtrans-http3-17.txt` Section 4.3
- `refs/webtrans/draft-ietf-webtrans-http3-16.txt` Section 4.3: 旧エラーコード
- `refs/h3/rfc9114.txt` Section 6.2.1 / 7.1 / 8
- <https://www.ietf.org/archive/id/draft-ietf-webtrans-http3-17.txt>
