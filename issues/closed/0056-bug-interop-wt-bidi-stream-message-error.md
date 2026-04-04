# interop-test-wt の双方向ストリーム関連テストが MessageError で軒並み失敗する

Created: 2026-04-07
Completed: 2026-04-07
Model: Opus 4.6

## 概要

`make interop-test` (= `cd interop_wt && cargo test`) のうち、`tests/ngtcp2_client_s2n_server.rs` で双方向ストリームを扱うテストが 9 件中 6 件失敗する。
原因はサーバ側 (tokio-s2n-quic) の WebTransport 受け入れ処理にあると見られる。

## 失敗テスト

- `test_client_opens_bidi_stream`
- `test_bidi_stream_echo`
- `test_path_and_authority`
- `test_server_opens_bidi_stream`
- `test_multiple_bidi_streams`
- `test_large_data` (`Nghttp3("ERR_H3_FRAME_UNEXPECTED", -601)`)

通っているのは `test_webtransport_session` / `test_unidirectional_stream` / Datagram 系 (bidi を開かないもの)。

## 再現手順

```
make interop-test
# または個別に
cd interop_wt && cargo test --test ngtcp2_client_s2n_server test_client_opens_bidi_stream -- --nocapture
```

## 観測されたエラー

```
thread 'tokio-rt-worker' panicked at tests/ngtcp2_client_s2n_server.rs:314:45:
called `Result::unwrap()` on an `Err` value: Http3(StreamError(MessageError))
```

ライン 314 は `let request = server.accept().await.unwrap();`。
`Http3(StreamError(MessageError))` は `src/validation.rs` 内の CONNECT ヘッダー検証 (`Error::StreamError(ErrorCode::MessageError)`) から発生。

`test_large_data` のみ別エラーで、nghttp3 クライアントが `ERR_H3_FRAME_UNEXPECTED` (-601) を返す。

## 解析メモ

### MessageError 系 (5 件)

- bidi を **開かない** テスト (`test_webtransport_session`) は同じ CONNECT パスを通って成功する
- bidi を開く側のテストは、クライアント側の `[ngtcp2 client] WebTransport session started` ログが表示されるが、これは nghttp3 が CONNECT 送出直後に出す楽観的ログで、実際には 200 応答を受け取っていない (サーバ側 `[s2n server] session request received` が出ない)
- したがって失敗箇所はサーバ側 `WtSessionRequest::from_connection` (crates/tokio-s2n-quic/src/webtransport/server.rs:218 以降) のヘッダー検証
- 仮説: nghttp3 はクライアントが bidi WT ストリームを開くワークロードで、CONNECT リクエストの構造あるいは後続フレームを通常パターンと変えており、`shiguredo_http3` の validation が `MessageError` を返している可能性が高い
  - 例: CONNECT の :authority / :path / :protocol の組み合わせ、または extended CONNECT の必須 pseudo-header の取り扱い
  - 例: CONNECT の HEADERS 後に DATA フレームを付加 (WebTransport では通常カプセルが続く)
  - もしくは、CONNECT 受信完了前に他ストリームのバイトが先に届き、ルーティングが誤って h3_conn に流れている

### ERR_H3_FRAME_UNEXPECTED (1 件)

- 大きいデータを送るテストでのみ発生
- nghttp3 (クライアント側) が予期しない HTTP/3 フレームを観測している
- WT_STREAM (0x41) で送ったストリームを nghttp3 が WebTransport ストリームとして認識していないと、type=0x41 を未知 H3 フレームとして読み飛ばし、後続バイトをフレーム長として誤解釈してこのエラーになる
- ただし SETTINGS_ENABLE_WEBTRANSPORT / H3_DATAGRAM / ENABLE_CONNECT_PROTOCOL は `Settings::enable_webtransport_server` 経由で送出されているはずなので別要因の可能性もある

## ソース上の懸念点 (副次的)

- `crates/tokio-s2n-quic/src/webtransport/session.rs:51-82` `accept_bi_stream`
  - `StreamHeader::decode_bidirectional` は Option しか返さないため、Signal Value が 0x41 でない場合の確定エラーをバッファ不足と区別できず、誤データで無限ループする可能性
  - 受信した WT_STREAM の session_id が当該セッションの ID と一致するか検証していない (draft-15 §4 では H3_ID_ERROR で接続クローズすべき)
- `session.rs:88-113` `open_bi_stream`
  - サーバから `open_bidirectional_stream` で server-initiated bidi を開いている。WebTransport では許容されるが、ピアが WT_STREAM として正しく分類できないと拒否される

## 対応方針 (案)

1. サーバ側で何が `MessageError` を返しているかを特定 (validation.rs に到達した時点のヘッダー値をログ出力するなどで切り分け)
2. ピアが送る CONNECT ヘッダーをデバッグ出力で確認
3. validation 側の不具合かピアの広告内容を吸収できていないかを判定

## 解決方法

`src/connection/mod.rs` の WebTransport CONNECT 受信側検証で、`:protocol` を「ピアの最高ドラフト」一つだけで判定していた箇所を「ローカルとピアが共に広告したドラフトの集合のいずれか」と一致するかで判定するように変更した。
具体的には以下を追加・修正:

- `Connection::mutually_advertised_wt_drafts()` を新設し、ローカルとピアの `wt_settings` の交差集合を新しい順に返すようにした
- `Connection::negotiated_wt_draft_version()` を新設し、上記交差集合の先頭 (最も新しいドラフト) を返すようにした
- WT CONNECT 受信時の `:protocol` 検証を `mutually_advertised_wt_drafts()` の各ドラフトの `protocol_value()` と照合するように変更
- `reset_stream_at` 必須判定も `peer_wt_draft_version()` から `negotiated_wt_draft_version()` に切り替え

これにより、ピアが draft-15 / draft-07 / draft-02 の SETTINGS を併送しつつ古い `:protocol` 値 (`webtransport`) を送ってくるケースでもサーバが CONNECT を受理するようになり、`make interop-test` の全 28 件 (h3 + wt) が通るようになった。

参照: draft-ietf-webtrans-http3-15 Section 3.2 / 7.1
