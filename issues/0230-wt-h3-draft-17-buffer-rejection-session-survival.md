# 先行ストリームのバッファ拒否後も WT セッションを継続できることを検証する

- Created: 2026-11-03
- Completed: {YYYY-MM-DD}
- Branch: feature/add-wt-draft-17-buffer-rejection-tests
- Polished: {YYYY-MM-DD}
- Category: test

## 目的

draft-ietf-webtrans-http3-17 Section 4.6 で明記された、バッファ済みストリームの拒否が対象セッションに影響しない性質を、公開接続 API を通して保証する。先行送信を拒否された後、別ストリームで送信し直せることまで確認する。

## 現状

- `src/connection/wt_session.rs` の `associate_or_buffer_stream` は、バッファ上限超過を `AssocOutcome::BufferOverflow` として返し、セッション終了とは分けている
- `src/connection/wt_stream.rs` と `src/connection/mod.rs` は、拒否対象のストリームについて `WebTransportEvent::BufferedStreamRejected` を生成する
- `src/connection/wt_types.rs` の `WtSession::buffer_stream` / `append_buffered_stream_data` は、ストリーム数とデータ量に上限を設ける
- `src/connection/mod.rs` の `test_pre_negotiation_wt_bidi_stream_exceeds_stream_limit` / `test_pre_negotiation_wt_bidi_stream_exceeds_data_limit` は拒否イベントを確認するが、その後の同一セッションの確立と別ストリームでの再送までは検証しない
- `tests/test_connection.rs` と `pbt/tests/prop_connection.rs` に、この一連の継続性を確認するテストはない

現時点でセッション終了の不具合を確認したわけではない。本 issue は、明確化された契約に対するテストの追加に絞る。

## 設計方針

- 実際の SETTINGS / CONNECT / 先行 WT ストリームのバイト列を `ClientConnection` / `ServerConnection` に渡す。モックやスタブを使わない
- ストリーム数上限と 1 ストリームのデータ量上限を超えるケースを分ける
- 上限到達で先行ストリームを拒否した後、SETTINGS 交換と CONNECT の 2xx 応答を完了し、同じ session ID の新しいストリームでデータを受信する
- クライアントが 2xx を待たずに送る場合と、サーバーのストリームがクライアントに 2xx より先に届く場合の両方を確認する
- 保留上限を解放した後の再送を使い、上限超過中でも無制限に新ストリームを受理することは要求しない
- 拒否したストリームの後続チャンクが配送されず、別ストリームは正常に配送されることを確認する
- データグラムの先行送信と上限超過も、破棄がセッション終了へ波及しないことを同じ観点で確認する。自動再送は実装しない
- 入力順序や分割を変える性質は PBT、意図的な上限超過は境界値テストとして分担する

## 完了条件

- 拒否対象に `WT_BUFFERED_STREAM_REJECTED` が通知され、接続エラーや `SessionClosed` は発生しない
- 同じセッションが 2xx 後に確立し、新しい bidi / uni ストリームで再送したデータを受信できる
- 拒否されたストリームのデータを誤配送せず、保持済みのほかのストリームも利用できる
- データグラムの破棄後も同じセッションを利用できる
- クライアント側とサーバー側の両方で、先行到着と拒否後の継続性を公開 API 経由で検証できる
- 本番コードの意味変更が必要な不具合を発見した場合は、本 issue に混ぜず別の bug issue として扱う

## 関連ファイル

- `src/connection/wt_session.rs`: `associate_or_buffer_stream`, `deliver_buffered_streams`
- `src/connection/wt_stream.rs`: 先行 uni / bidi と `feed_datagram` の処理
- `src/connection/wt_types.rs`: `WtSession::buffer_stream`, `append_buffered_stream_data`, `buffer_datagram`
- `src/connection/mod.rs`: `buffer_pre_negotiation_wt_bidi`, `test_pre_negotiation_wt_bidi_stream_exceeds_*`
- `tests/test_connection.rs`, `pbt/tests/prop_connection.rs`

## 一次資料

- `refs/webtrans/draft-ietf-webtrans-http3-17.txt` Section 4.6
- <https://www.ietf.org/archive/id/draft-ietf-webtrans-http3-17.txt>
