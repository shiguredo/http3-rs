# 先行ストリームのバッファ拒否後も WT セッションを継続できることを検証する

- Created: 2026-11-03
- Completed: {YYYY-MM-DD}
- Branch: feature/add-wt-draft-17-buffer-rejection-tests
- Polished: 2026-11-03
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
- SETTINGS 受信前の保留は、サーバーが受信するクライアント開始 bidi を対象にする。クライアントにはサーバー SETTINGS を受信させ、transport parameter 検証も済ませた上で、クライアント SETTINGS のサーバーへの投入だけを遅らせる。上限到達で先行ストリームを拒否した後、遅延させた SETTINGS と CONNECT の処理、2xx 応答を完了し、同じ session ID の新しいストリームでデータを受信する
- uni / bidi / datagram のセッション Pending 中の保留は、必要な SETTINGS 交換と transport parameter 検証を完了してから試験する。クライアントが 2xx を待たずに送るケースではサーバーの受理前に到着させる。サーバーからの先着ケースでは、サーバーの `send_response` でセッションを確立した後、生成した 2xx バイトのクライアントへの投入だけを遅らせ、クライアントの既存 Pending セッションに先行データを到着させる
- 保留上限を解放した後の再送を使い、上限超過中でも無制限に新ストリームを受理することは要求しない
- 拒否時点までに受信した対象ストリームのデータがセッション確立時に配送されず、別ストリームは正常に配送されることを確認する。拒否後の後続チャンクの吸収は、既存の `ignored_pre_negotiation_wt_bidi` に記録される SETTINGS 受信前の拒否経路で検証する
- SETTINGS 交換後の拒否では、`BufferedStreamRejected` が統合層へ通知する `RESET_STREAM` / `STOP_SENDING` による QUIC ストリームの終了を前提にする。この経路で、拒否後も同じ stream ID に任意チャンクを渡し続ける耐性を新たに保証することは、本 issue のテスト対象に含めない。`stream_reset` はピアからの RESET 受信通知であり、ローカルの拒否処理完了通知として呼び出さない
- データグラムの先行送信と上限超過も、破棄がセッション終了へ波及しないことを同じ観点で確認する。自動再送は実装しない
- 入力順序や分割を変える性質は PBT、意図的な上限超過は境界値テストとして分担する

## 完了条件

- 拒否対象に `WT_BUFFERED_STREAM_REJECTED` が通知され、接続エラーや `SessionClosed` は発生しない
- 同じセッションが 2xx 後に確立し、新しい bidi / uni ストリームで再送したデータを受信できる
- 拒否時点までに受信した対象ストリームのデータを誤配送せず、保持済みのほかのストリームも利用できる。SETTINGS 受信前の保留拒否では、同じ stream ID の後続チャンクも配送されない
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
