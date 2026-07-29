# nghttp3 との HTTP/3 e2e 相互運用テストを拡充する

- Priority: Medium
- Created: 2026-06-09
- Completed: 2026-07-30
- Model: Opus 4.7
- Branch: feature/add-nghttp3-interop-tests
- Polished: 2026-07-21

## 目的

shiguredo_http3 と nghttp3 (ngtcp2) との e2e 相互運用テストを RFC 9114 に基づいて拡充する。

AGENTS.md は「ngtcp2 / nghttp3 を正としてテストを行うこと」を方針として掲げているが、現状の interop テストは双方向ペアあたり 4 ケース (GET / POST with body / カスタムレスポンスヘッダー / 404) のみで、HTTP/3 として最低限カバーすべき領域 (メソッド・パス・ステータス・ヘッダー多様性・ボディサイズ・接続あたり複数リクエスト) を網羅していない。本 issue では現状 API で実装可能な範囲に絞り、計 37 件のテストを追加する (`s2n_client_ngtcp2_server.rs` に 15 件、`ngtcp2_client_s2n_server.rs` に 22 件)。

## 優先度根拠

Medium。canary 段階での品質強化目的で、緊急度はないが、IETF リファレンス実装との整合性カバレッジを広げることは canary 卒業前に価値がある。バグ修正ではないため High ではない。

## 現状

### 既存テストファイル

- `interop/h3/tests/s2n_client_ngtcp2_server.rs` (373 行)
  - `test_http3_request_response` (GET /)
  - `test_post_request_with_body` (POST / + body)
  - `test_response_custom_headers` (GET / + カスタムヘッダー検証)
  - `test_status_404` (GET /not-found → 404)
- `interop/h3/tests/ngtcp2_client_s2n_server.rs` (404 行)
  - 同じ 4 ケースを逆方向で実装

### ヘルパーの現状所在

- `interop/h3/src/lib.rs` に `generate_shared_certificate`, `save_certificate_files` (両ファイルから利用)
- `start_ngtcp2_server` は `s2n_client_ngtcp2_server.rs:17-34` にローカル定義 (本 issue 対象ファイルでは 1 か所のみ)
- `send_ngtcp2_request` と `Ngtcp2Response` は `ngtcp2_client_s2n_server.rs:26-101`, `16-23` にローカル定義

### 関連 API の確認結果

実装に先立って関連 API を確認した結果、以下が判明している。テスト設計の前提として記録する。

- `H3Client::send_request` (`crates/tokio-s2n-quic/src/h3/client.rs:160`) は `&mut self` のため逐次のみ。同一インスタンスで複数回呼び出して連続リクエストは可能。
- `H3ClientRequest` には `get(path)` / `post(path)` のみが提供される (`crates/tokio-s2n-quic/src/h3/client.rs:291, 302`)。HEAD / PUT / DELETE 用のコンストラクタは無く、`header(":method", "HEAD")` で擬似ヘッダーを後付けすると `send_request` 内部 (`client.rs:174`) で `:method` が重複してサーバー側 `validate_request_headers` (`src/validation.rs:377`) で拒否される。よって `[s→n]` 方向の HEAD/PUT/DELETE は現状 API では送信不可。
- `H3ClientRequest::header()` (`crates/tokio-s2n-quic/src/h3/client.rs:319`) は送信時に `Header::new()` (`src/qpack/header.rs:420`) を経由するため、大文字フィールド名は `HeaderError::UppercaseFieldName` で即拒否される。
- `H3ClientRequest::body()` 設定時に `content-length` ヘッダーは自動付与されない。明示付与が必要。
- `H3ClientResponse` (`crates/tokio-s2n-quic/src/h3/client.rs:331`) は `status` / `headers` / `body` の 3 フィールドのみ。`stream_id()` ゲッターなし。
- `H3Request::stream_id()` (`crates/tokio-s2n-quic/src/h3/server.rs:359`) は存在。サーバー側からは観測可能。
- `tokio_ngtcp2::Server::run` の handler シグネチャ (`crates/tokio-ngtcp2/src/server.rs:107`) は `FnMut(SocketAddr, Http3Event) -> Option<(Vec<Header>, Vec<u8>)>`。実装上 `Http3Event::HeadersEnd` のときだけ `stream_id` を抽出して `submit_response*` を呼ぶ (`server.rs:199-221`)。HTTP/3 のフレーム到着順 (HEADERS → DATA → end_stream) を踏まえると、`HeadersEnd` 発火時点ではリクエストボディの Data は **まだ届いていない**。したがって `[s→n]` 方向ではリクエストボディの長さ・内容を検証するレスポンスをサーバー側で生成できない (送るタイミングが無い)。リクエストボディ検証は `[n→s]` 方向 (shiguredo_http3 サーバーの `H3Request::body()` 経由) のみで成立する。
- `tokio_ngtcp2::StreamId` は `i64` (`crates/ngtcp2-rs/src/types.rs:153`)。
- shiguredo_http3 のデフォルト `max_field_section_size = 16 KiB` (`src/limits.rs:6`)。nghttp3 側デフォルトは `64 KiB` (`crates/ngtcp2-rs/src/config.rs:49`)。両端で個別に持つため interop 時は小さい方 (16 KiB) が支配する。
- ngtcp2 のデフォルト `initial_max_stream_data_bidi_local/remote/uni = 1024 * 1024` (= 1 MiB ぴったり), `initial_max_data = 10 * 1024 * 1024` (`crates/ngtcp2-rs/src/config.rs:112-118`)。`tokio_s2n_quic::ServerConfig` / `ClientConfig` に QUIC フロー制御値 (`initial_max_data` 等) を上書きする公開 API は存在しない (`crates/tokio-s2n-quic/src/config.rs`, `h3/server.rs:29`, `h3/client.rs:37`)。
- `interop/h3/src/lib.rs:147` のコメント「shiguredo_http3 は静的テーブルのみサポート」のとおり、shiguredo_http3 側は常時 QPACK 静的テーブルのみ。動的テーブルを意図して「踏ませる」テストは書けない。
- RFC 9000 §2.1 によりクライアント発の bidi stream は最低位ビット `00` で stream_id = 0, 4, 8 と +4 ずつ単調増加することが規定されている。s2n-quic / ngtcp2 とも仕様準拠。

## 設計方針

### スコープ

shiguredo_http3 ↔ nghttp3 の 2 ファイル (`s2n_client_ngtcp2_server.rs`, `ngtcp2_client_s2n_server.rs`) のみ。他組み合わせは本 issue の対象外。既存 4 ケースは温存。

### テスト方向の表記

- `[s→n]`: shiguredo_http3 クライアント → nghttp3 サーバー (`s2n_client_ngtcp2_server.rs` に追加)
- `[n→s]`: nghttp3 クライアント → shiguredo_http3 サーバー (`ngtcp2_client_s2n_server.rs` に追加)
- `[両]`: 両ファイルに追加

両方向追加の原則: 送信側エンコーダと受信側デコーダのどちらに検証主眼を置くかで判断する。エンコーダ/デコーダの双方が shiguredo_http3 側で異なる場合 (例: メソッド・パス・ステータス・ヘッダー・ボディ等) は両方向。検証主眼が片方に偏る場合 (E5 UTF-8 バイト透過は受信側デコーダの検証として `[n→s]` 側に意味があるなど) は片方向。

## 追加するテストケース

各ケースに対応する RFC 節と、サーバー応答 / クライアント assert を明示する。

### A. メソッド (RFC 9114 §4.3.1 `:method`, RFC 9110 §9)

`H3ClientRequest` には `get()` / `post()` のみが提供されており、HEAD / PUT / DELETE のコンストラクタは無い。`[s→n]` 方向は API 上送信不可なので `[n→s]` のみとする (詳細はスコープ外節 1)。

- **A1. `test_method_head` `[n→s]`**
  - リクエスト: `:method = HEAD`, `:path = /`
  - サーバー (shiguredo_http3) 応答: `200`, `content-type: text/plain`, `H3Response::new(200).header("content-type", "text/plain").body("")` (HEAD では RFC 9110 §9.3.2 により content を送らない)
  - assert (nghttp3 クライアント側): `response.status == 200` かつ `response.body.is_empty()`
- **A2. `test_method_put_with_body` `[n→s]`**
  - リクエスト: `PUT /resource` + 1 KiB ボディ (`vec![0x41u8; 1024]` 等)
  - サーバー応答: `200`, body = `"OK"`
  - assert: `response.status == 200`, `response.body == b"OK"`, さらに shiguredo_http3 サーバー側 (`H3Request::body()`) でリクエストボディ長 1024 を確認
- **A3. `test_method_delete` `[n→s]`**
  - リクエスト: `DELETE /resource`
  - サーバー応答: `204`, body 空
  - assert: `response.status == 204` かつ `response.body.is_empty()`

合計: 3 ケース

### B. パス (RFC 9114 §4.3.1 `:path`, RFC 9110 §4.2.1)

`:path` 値は RFC 9114 §4.3.1 でバイト列として規定。`H3Request::path()` (`crates/tokio-s2n-quic/src/h3/server.rs:345`) は生バイト列を返す (percent-decoding しない)。

- **B1. `test_path_with_query` `[両]`**
  - リクエスト: `GET /search?q=hello&page=2`
  - サーバー応答: `200`, ヘッダー `x-echo-path: <受信した path バイト列>`
  - assert (クライアント側): `x-echo-path` ヘッダー値が `b"/search?q=hello&page=2"` と完全一致 (バイト列等価)
- **B2. `test_path_percent_encoded` `[両]`**
  - リクエスト: `GET /items/%E3%83%86%E3%82%B9%E3%83%88` (「テスト」を percent-encoded した形)
  - サーバー応答: `200` + `x-echo-path: <受信した path バイト列>` (decode しない)
  - assert: `x-echo-path` 値が送信した percent-encoded バイト列と完全一致
- **B3. `test_path_nested` `[両]`**
  - リクエスト: `GET /api/v1/users/123/posts/456`
  - サーバー応答: `200` + `x-echo-path: <受信した path バイト列>`
  - assert: `x-echo-path` 値が送信パスと完全一致

合計: 6 ケース

### C. ステータスコード (RFC 9114 §4.3.2 `:status`, RFC 9110 §15)

- **C1. `test_status_204` `[両]`**
  - サーバー応答: `204`, body は空 (RFC 9110 §15.3.5: 204 MUST NOT have body)
  - assert: `response.status() == 204` かつ `response.body().is_empty()`
- **C2. `test_status_301_with_location` `[両]`**
  - サーバー応答: `301`, `location: /new-path`, body 空
  - assert: status = 301, `location` ヘッダー値が `b"/new-path"` と一致
- **C3. `test_status_500` `[両]`**
  - サーバー応答: `500`
  - assert: `response.status() == 500`
- **C4. `test_status_503_with_retry_after` `[両]`**
  - サーバー応答: `503`, `retry-after: 30`
  - assert: status = 503, `retry-after` ヘッダー値が `b"30"` と一致

合計: 8 ケース

### D. ヘッダー (RFC 9114 §4.2, QPACK 静的テーブル RFC 9204 Appendix A)

shiguredo_http3 の `max_field_section_size = 16 KiB` 制限下で全ケースが収まる。各 `Header` は `name + value + 32 bytes overhead` でサイズが計算される (RFC 9114 §4.2.2)。順序保持の assert は、`headers().iter().filter(|(n,_)| n.starts_with(b"x-test-"))` で得たイテレータを順に `(b"x-test-00", _), (b"x-test-01", _), ..., (b"x-test-15", _)` と等価比較する。

- **D1. `test_request_many_headers` `[両]`**
  - リクエストに `x-test-NN: value-NN` (NN = 00..15、name 9 バイト + value 8 バイト + overhead 32 = 49 バイト/件、計 784 バイト) の 16 個追加ヘッダー
  - サーバー応答: `200`, ヘッダー `x-received-count: <受信した x-test- ヘッダー数>`
  - assert: `x-received-count` 値が `b"16"` と一致。さらに送信順に並んでいることを `assert_eq!(received_xtest_headers, expected_xtest_headers)` で確認
- **D2. `test_response_many_headers` `[両]`**
  - サーバー応答: `200`, `x-response-NN: value-NN` (NN = 00..15) の 16 個追加ヘッダー
  - assert: クライアント側で `assert_eq!(received_xresponse_headers, expected_xresponse_headers)` (送信順含む)
- **D3. `test_large_cookie_header` `[両]`**
  - リクエスト: `cookie` ヘッダーに 2 KiB の値 (`vec![b'a'; 2048]` で文字 'a' のみ) (2048 + 6 + 32 = 2086 バイト、16 KiB 制限内)
  - サーバー応答: `200`, `x-cookie-len: 2048`
  - assert: `x-cookie-len` 値が `b"2048"` と一致
- **D4. `test_response_multiple_set_cookie` `[両]`**
  - サーバー応答: `200`, `set-cookie: a=1`, `set-cookie: b=2`, `set-cookie: c=3` (3 件、QPACK 静的テーブル idx 14 で name-only 参照される可能性あり)
  - assert: `let set_cookies: Vec<&[u8]> = response.headers.iter().filter(|(n,_)| n == b"set-cookie").map(|(_,v)| v.as_slice()).collect();` が `vec![b"a=1", b"b=2", b"c=3"]` と等価
- **D5. `test_content_length_header` `[両]`**
  - リクエスト: `POST / + body "Hello, server!"` + 明示的に `header("content-length", "14")` を付与
  - サーバー応答: `200`, `content-length: 2` + body `"OK"`
  - assert: クライアント側で `content-length` ヘッダー値が `b"2"`, body 長が 2

合計: 10 ケース

### E. ボディ (RFC 9114 §4.1, §7.2.1 DATA frame, フロー制御は RFC 9000 §4)

リクエストボディ送信の検証は、`tokio_ngtcp2::Server::run` の handler モデル制約 (`HeadersEnd` 時点で応答送信、Data 蓄積を待てない) により `[s→n]` 方向ではサーバー側でレスポンスに反映できない。よってリクエストボディの長さ・内容を検証するケース (E2/E3/E5/E6) は `[n→s]` 方向のみとする。`[n→s]` 方向では shiguredo_http3 サーバーが `H3Request::body()` でボディを全受信したうえでレスポンスに反映する。

レスポンスボディの検証 (E4) は受信側 (クライアント) で行うため、双方向 (`[両]`) で実装する。

空ボディ (E1) は `HeadersEnd { fin: true }` で完了するため、`[s→n]` 方向でも Data 蓄積を待たずに `HeadersEnd` 時点でレスポンスを返せる。`[両]` で実装する。

- **E1. `test_empty_body_post` `[両]`**
  - リクエスト: `H3ClientRequest::post("/").header("content-length", "0").body(b"")` (`[s→n]`) / nghttp3 クライアント側で `:method POST` + `content-length: 0` + body 空 (`[n→s]`)
  - サーバー応答 (`[s→n]` 側): nghttp3 サーバー handler は `HeadersEnd { fin: true }` を見て `H3Response` を返す。`200` + `x-body-recv: 0`
  - サーバー応答 (`[n→s]` 側): shiguredo_http3 サーバー (`H3Request::body()`) で長さ 0 を確認、`200` + `x-body-recv: 0`
  - assert: `response.status == 200` かつ `x-body-recv` 値が `b"0"`
- **E2. `test_medium_body_64kib` `[n→s]`** (リクエストボディ送信の検証なので `[n→s]` のみ)
  - 64 KiB を選ぶ理由: ngtcp2 のクライアント送信バッファ `send_buf = 1350` バイト (`crates/tokio-ngtcp2/src/client.rs:166`) を大きく超え、約 49 パケットに分割される。複数 DATA frame fragment 再構築の検証として実用的サイズ。
  - リクエスト: `POST / + 64 KiB ボディ` (`(0..64*1024).map(|i| (i % 256) as u8).collect()`)
  - サーバー応答: shiguredo_http3 サーバー側で `H3Request::body().len() == 65536` を確認、`200` + `x-body-len: 65536` + `x-body-prefix: "0001020304050607"` (先頭 8 バイトの hex)
  - assert: 上記 2 ヘッダーが期待値と一致
- **E3. `test_large_body_800kib_request` `[n→s]`** (リクエスト方向のみ)
  - 800 KiB を選ぶ理由: `[n→s]` 方向は ngtcp2 クライアント → s2n-quic サーバー。実効上は s2n-quic サーバーが広告する `initial_max_stream_data_bidi_remote` が支配的だが、双方向化を見越して ngtcp2 側 (将来 `[s→n]` 拡張時に問題) のデフォルト `initial_max_stream_data_bidi_*` = 1 MiB ちょうど (`crates/ngtcp2-rs/src/config.rs:112-114`) に対しても DATA frame ヘッダーと QPACK エンコード済みヘッダーセクションの余裕を残すために 800 KiB を選ぶ。1 MiB 以上は別 issue (スコープ外 13) で `tokio_s2n_quic::ServerConfig` / `ClientConfig` への Limits 公開 API を整備したうえで扱う。
  - リクエスト: `POST / + 800 KiB ボディ` (`(0..800*1024).map(|i| (i % 256) as u8).collect()`)
  - サーバー応答: `200` + `x-body-len: 819200` + `x-body-prefix: "0001020304050607"`
  - assert: 上記 2 ヘッダーが期待値と一致
- **E4. `test_large_body_800kib_response` `[両]`**
  - サーバーは固定の 800 KiB ボディ (`(0..800*1024).map(|i| (i % 256) as u8).collect()`) を返す
  - assert: `response.body.len() == 819200`, 先頭 8 バイトが `[0,1,2,3,4,5,6,7]` と一致
- **E5. `test_utf8_multibyte_body` `[n→s]`**
  - リクエスト: `POST / + body "あいうえお🎌🇯🇵中文ABC".as_bytes()` (UTF-8 で計 36 バイト: あ/い/う/え/お は 5 文字 × 各 3 バイト = 15, 🎌 = 4, 🇯🇵 = 8 (Regional Indicator 2 つ), 中/文 は 2 文字 × 各 3 バイト = 6, ABC は 3 文字 × 各 1 バイト = 3)
  - サーバー応答: shiguredo_http3 サーバー側で `H3Request::body()` が送信バイト列と完全一致を確認、`200` + `x-body-len: 36` + `x-body-prefix: "e38182e38184e381"` (先頭 8 バイトの hex)
  - assert: 上記 2 ヘッダーが期待値と一致
- **E6. `test_binary_body_all_bytes` `[n→s]`**
  - リクエスト: `POST / + body (0..=255u8).cycle().take(4096).collect()` (4 KiB、全バイト範囲を含む)
  - サーバー応答: shiguredo_http3 サーバー側で長さと内容を確認、`200` + `x-body-len: 4096` + `x-body-prefix: "0001020304050607"`
  - assert: 上記 2 ヘッダーが期待値と一致

合計: `[両]` 2 件 + `[n→s]` 4 件 = 6 件 (s2n_client 側 2 件、ngtcp2_client 側 6 件)

### F. 接続あたり複数リクエスト (RFC 9114 §6.1)

RFC 9000 §2.1 によりクライアント発の bidi stream ID は 0, 4, 8 と +4 ずつ増える。`H3Client::send_request` (`crates/tokio-s2n-quic/src/h3/client.rs:160`) は同一インスタンスで連続呼び出し可能。`[n→s]` 方向は `tokio_ngtcp2::Client::send_request` (`crates/tokio-ngtcp2/src/client.rs:258`) を同一 `Client` で複数回呼ぶ。

クライアント側 `H3ClientResponse` / `Ngtcp2Response` には `stream_id()` ゲッターがないため、stream_id 検証はサーバー側で行う。`tokio_ngtcp2::StreamId` は `i64`。

- **F1. `test_sequential_3_requests` `[両]`**
  - クライアントは 1 接続を確立した後、`/req-0`, `/req-1`, `/req-2` への 3 回 GET リクエストを **逐次 (1 件目の `StreamEnd` を待って return → 次の `send_request`)** で送信。`send_ngtcp2_requests` ヘルパーの戻り値 `Vec<Ngtcp2Response>` は送信順 = 完了順
  - サーバー側 handler は `Vec<i64>` を `mut` ローカル変数として宣言し、`move` キャプチャしたうえで `HeadersEnd` 毎に `stream_id` を `push`、`x-stream-ids: <CSV>` ヘッダーに乗せて返す。スケルトン:

    ```rust
    let mut stream_ids: Vec<i64> = Vec::new();
    server.run(move |_addr, event| {
        if let Http3Event::HeadersEnd { stream_id, .. } = event {
            stream_ids.push(stream_id);
            let csv = stream_ids.iter().map(|id| id.to_string()).collect::<Vec<_>>().join(",");
            Some((
                vec![Header::status(200), Header::new(b"x-stream-ids", csv.as_bytes())],
                vec![],
            ))
        } else {
            None
        }
    });
    ```

  - 「3 回」の根拠: 2 回では「stream ID が +4 で増える」しか確認できない。3 回なら「累積処理 (`Vec<i64>::push`) が 2 回目 → 3 回目で破綻しない」「ストリーム ID 払い出しが連続単調 (`0, 4, 8`) で進む」の両方を確認できる
  - assert (3 件すべて確認): `for i in 0..3 { assert_eq!(responses[i].status, 200); let expected: &[u8] = match i { 0 => b"0", 1 => b"0,4", _ => b"0,4,8" }; let actual = responses[i].headers.iter().find(|(n,_)| n == b"x-stream-ids").map(|(_,v)| v.as_slice()).unwrap(); assert_eq!(actual, expected); }`

合計: 2 ケース

### 総ケース数

カテゴリ別内訳:
- A (メソッド): `[n→s]` のみ 3 件 (A1-A3) → ngtcp2_client 側 3 件
- B (パス): `[両]` 3 件 (B1-B3) → 各ファイル 3 件
- C (ステータス): `[両]` 4 件 (C1-C4) → 各ファイル 4 件
- D (ヘッダー): `[両]` 5 件 (D1-D5) → 各ファイル 5 件
- E (ボディ): `[両]` 2 件 (E1, E4) + `[n→s]` のみ 4 件 (E2, E3, E5, E6) → s2n_client 側 2 件、ngtcp2_client 側 6 件
- F (連続リクエスト): `[両]` 1 件 (F1) → 各ファイル 1 件

ファイル別合計:
- `s2n_client_ngtcp2_server.rs`: 0 + 3 + 4 + 5 + 2 + 1 = **15 件**
- `ngtcp2_client_s2n_server.rs`: 3 + 3 + 4 + 5 + 6 + 1 = **22 件**
- 新規合計: **37 件**
- 既存 8 件 (両ファイル合算) と合わせて全体 45 件

## 共通実装方針

### ヘルパーの集約

新規ケース追加に先立ち、以下を `interop/h3/src/lib.rs` に集約する (新規ファイルは作らず、既存の `start_quiche_server` などと並べる):

- `interop/h3/src/lib.rs` に `pub async fn start_ngtcp2_server(cert_path: &Path, key_path: &Path) -> Result<(tokio_ngtcp2::Server, SocketAddr), Box<dyn Error + Send + Sync>>` を追加 (`s2n_client_ngtcp2_server.rs:17-34` のロジックをそのまま移動)
- `Ngtcp2Response { pub status: u16, pub headers: Vec<(Vec<u8>, Vec<u8>)>, pub body: Vec<u8> }` を `pub struct` で `interop/h3/src/lib.rs` に追加 (`ngtcp2_client_s2n_server.rs:16-23` から移動)。フィールドはすべて `pub` で公開する (Cargo.toml の `publish = false` のため公開しても外部互換負担はない)
- `pub async fn send_ngtcp2_request(server_addr: SocketAddr, method: &str, path: &str, extra_headers: &[(&str, &str)], body: Vec<u8>) -> Result<Ngtcp2Response, Box<dyn Error + Send + Sync>>` を追加 (`ngtcp2_client_s2n_server.rs:26-101` を一般化し、追加ヘッダーを受け取れるよう拡張)。内部実装: `body.is_empty()` なら `Client::send_request(&headers)`、それ以外なら `Client::send_request_with_body(&headers, body)` で分岐する (現行 `ngtcp2_client_s2n_server.rs:44-48` の分岐を踏襲)
- 1 接続で複数リクエストを送る F1 用に、`pub async fn send_ngtcp2_requests(server_addr: SocketAddr, requests: Vec<(String, String, Vec<u8>)>) -> Result<Vec<Ngtcp2Response>, Box<dyn Error + Send + Sync>>` を追加 (各 body の所有権を内部で `send_request_with_body` に渡す都合上、`requests` も所有権を受け取る形にする)。内部実装: `Client` を 1 度生成し、各リクエストごとに `send_request` → `Http3Event::StreamEnd` を待って `Ngtcp2Response` を完成 → `Vec` に push → 次の `send_request`。「逐次」とは厳密に「1 件目の `StreamEnd` まで poll → 完了確認 → 次の `send_request`」と定義する
- エラー型は既存ヘルパーと同じ `Box<dyn Error + Send + Sync>` で揃える
- 既存 4 ケースの `use` 文をこれらヘルパー参照に書き換える
- 新規追加した nghttp3 関連ヘルパー (`start_ngtcp2_server`, `send_ngtcp2_request`, `send_ngtcp2_requests`) の `eprintln!` メッセージは英語で記述する (AGENTS.md 「ログメッセージは全て英語」)。例: `"[ngtcp2 server] started: {}"`。既存の quiche/quinn ヘルパー (`start_quiche_server`, `run_quinn_client` 等) の日本語 `eprintln!` 修正は本 issue のスコープ外

### ボディ検証の方針

`tokio_ngtcp2::Server::run` の handler モデルは `HeadersEnd` 起点で応答を返す設計 (`crates/tokio-ngtcp2/src/server.rs:199-221`)。HTTP/3 のフレーム到着順は HEADERS → DATA → end_stream なので、`HeadersEnd` 発火時点ではリクエストの DATA frame はまだ届いていない。したがって `[s→n]` 方向ではリクエストボディ長・内容をレスポンスに反映できない。`tokio_ngtcp2::Server` API 拡張 (例: `StreamEnd` 起点で応答できるハンドラ追加) はスコープ外節 12 として別 issue 化する。

本 issue では以下の縮退方針を採用する:
- `[s→n]` 方向: 空ボディ E1 (`HeadersEnd { fin: true }` で完結) と、応答ボディ送信のみの E4 を実装。リクエストボディ送信検証 (E2/E3/E5/E6) は `[n→s]` 方向に集約
- `[n→s]` 方向: shiguredo_http3 サーバー (`H3Request::body()`) でリクエストボディを全受信し、長さと先頭 8 バイトを `x-body-len: <len>` と `x-body-prefix: <先頭 8 バイトの 16 進文字列>` の 2 ヘッダーに乗せてレスポンスする

サーバー側 closure の構造 (`[n→s]` 方向):

```text
let bodies: HashMap<u64, Vec<u8>> = ... (省略);
loop {
    let request = conn.accept_request().await?;
    let body_len = request.body().len();
    let prefix = hex_encode(&request.body()[..8.min(body_len)]);
    request.send_response(
        H3Response::new(200)
            .header("x-body-len", body_len.to_string())
            .header("x-body-prefix", prefix)
    ).await?;
}
```

`[s→n]` 方向 (E1 / E4) のサーバー側 closure 構造:

```text
// E1: HeadersEnd で fin: true を確認したうえで固定応答 (リクエストボディが空であることをガード)
server.run(move |_addr, event| {
    if let Http3Event::HeadersEnd { stream_id: _, fin: true } = event {
        Some((vec![Header::status(200), Header::new(b"x-body-recv", b"0")], vec![]))
    } else {
        None
    }
});

// E4: HeadersEnd で 800 KiB body 応答 (fin の値は問わない: GET の場合は fin=true、POST 等の場合は false の可能性)
let response_body: Vec<u8> = (0..800*1024).map(|i| (i % 256) as u8).collect();
server.run(move |_addr, event| {
    if let Http3Event::HeadersEnd { stream_id: _, .. } = event {
        Some((vec![Header::status(200), Header::new(b"x-body-len", b"819200")], response_body.clone()))
    } else {
        None
    }
});
```

### 巨大ボディ (E3/E4) の方針

- 800 KiB を上限とする (1 MiB に届かない値にして ngtcp2 デフォルト `initial_max_stream_data_bidi_*` = 1 MiB のフロー制御内に収める)
- `tokio_s2n_quic::ServerConfig` / `ClientConfig` への Limits 公開 API 追加は本 issue のスコープ外 (スコープ外節 13)。1 MiB 以上の interop は別 issue で取り扱う

### テスト時間と timeout

クライアント側およびサーバー側の両方を同じ `Duration` で `tokio::time::timeout` する (片方だけ伸ばすと片方先に終了して接続が切れる):
- E3 / E4 (800 KiB): 15 秒 (loopback でもパケット数とポーリング間隔の積で 10 秒に近づく可能性があるため余裕を持たせる)
- それ以外 (A〜D, E1, E2, E5, E6, F1): 10 秒

サーバー側ヘルパー (`start_ngtcp2_server` / shiguredo_http3 `H3Server::accept_request` を呼ぶ wrapper) は `Duration` を引数で受け取れるよう設計する。

### テスト数とファイル分割

新規追加後のファイル行数は両ファイルとも 800〜1000 行程度になる見込み。AGENTS.md 「テストファイルが長くなった場合はファイル内で `mod` を使って分割すること」に従い、`mod method { ... } mod path { ... } mod status { ... } mod header { ... } mod body { ... } mod connection { ... }` の 6 モジュール構成で分割する。

### CHANGES.md 扱い

AGENTS.md 「機能に直接影響しない変更 (ドキュメント追加、リファクタリング等) は `### misc` サブセクションに記載すること」に従い、`## develop` の `### misc` セクション (なければ新設) に以下のエントリ案を追加する:

```
### misc

- [ADD] nghttp3 (ngtcp2) との HTTP/3 e2e 相互運用テストを `interop/h3/tests/{s2n_client_ngtcp2_server,ngtcp2_client_s2n_server}.rs` に 37 件追加する
  - @voluntas
```

## 完了条件

- 上記 37 件 (s2n_client 15 件、ngtcp2_client 22 件) を実装し、すべて pass する
- 既存の 8 件 (両ファイル合算) も引き続き pass する
- 各テストファイルが `mod` で分割されている。`s2n_client_ngtcp2_server.rs` (15 件) はメソッドケースを含まないので `path` / `status` / `header` / `body` / `connection` の 5 モジュール。`ngtcp2_client_s2n_server.rs` (22 件) は `method` / `path` / `status` / `header` / `body` / `connection` の 6 モジュール。既存 4 ケースの分配は以下:
  - `mod path`: 既存 `test_http3_request_response`
  - `mod body`: 既存 `test_post_request_with_body`
  - `mod header`: 既存 `test_response_custom_headers`
  - `mod status`: 既存 `test_status_404`
- 共通ヘルパー (`start_ngtcp2_server`, `send_ngtcp2_request`, `send_ngtcp2_requests`, `Ngtcp2Response`) が `interop/h3/src/lib.rs` に集約され、既存ヘルパーの `eprintln!` メッセージが英語になっている
- E3 / E4 は 15 秒、それ以外は 10 秒の timeout 内に完結する
- `cargo test -p interop_h3 --test s2n_client_ngtcp2_server` と `cargo test -p interop_h3 --test ngtcp2_client_s2n_server` が両方とも成功する
- CHANGES.md の `### misc` に `[ADD]` エントリが追加されている

## スコープ外 (本 issue では扱わない)

API 制約・実装制約により本 issue では実装できないものを列挙する。検証価値の高い項目は本 issue マージ後に別 issue として起票する。

1. **`H3ClientRequest` の HEAD / PUT / DELETE 対応 (`[s→n]` 方向の A1-A3)**: 「関連 API の確認結果」のとおり、HEAD/PUT/DELETE は `[s→n]` 方向では送信不可。本 issue では `[n→s]` 方向のみ実装する。
2. **大文字を含む不正リクエストヘッダーの拒否確認**: 「関連 API の確認結果」のとおり、クライアント側 `Header::new()` で即 reject されるため意図送信不可。Sans I/O API (`shiguredo_http3::ClientConnection`) を直接叩く別経路が必要。
3. **必須擬似ヘッダー欠落 / 順序違反 / 不正値 / 不正文字 (RFC 9114 §4.1.2 Malformed の残り項目)**: `H3ClientRequest` API は擬似ヘッダーを内部で組み立てるため、外部から強制できない。2 と同じく Sans I/O API 直叩きが必要。
4. **`connection: keep-alive` 等 connection-specific フィールドの拒否確認 (RFC 9114 §4.2)**: `Header::new()` での検査有無を含め、API 拡張または Sans I/O 直叩きが必要。
5. **GOAWAY 明示送信 (RFC 9114 §5.2)**: `H3Server` / `H3Client` API に GOAWAY 送信メソッドが公開されていない。
6. **RST_STREAM / STOP_SENDING 明示送信 (RFC 9114 §4.1.1)**: 同じく API 未公開。
7. **1 接続内の並列リクエスト**: `H3Client::send_request` が `&mut self` のため、同時実行は不可。並列化には API 拡張 (`&self` 化または `open_stream` メソッド分離) が必要。
8. **トレーラー (HEADERS frame 2 回目、RFC 9114 §4.1)**: `H3Response` / `H3Request` API にトレーラー設定メソッドがない。
9. **CONNECT メソッド (RFC 9114 §4.4)**: WebTransport 以外の純 CONNECT は shiguredo_http3 / tokio-s2n-quic のサポート状況を別途検証する必要。
10. **interim response (1xx, RFC 9114 §4.1)**: `H3Server` の API にステータス 1xx を「interim」として送る仕組みがない。
11. **SETTINGS Initialization / GREASE の interop 検証 (RFC 9114 §7.2.4.2, §7.2.8)**: 接続確立時に交換される SETTINGS の中身を観測する手段が `H3Client` / `H3Server` API で公開されていない (0094 で GREASE 送信は実装済みだが、interop 観点で受信側が正しく無視するかの確認は API 未公開)。
12. **`tokio_ngtcp2::Server::run` の `StreamEnd` 起点応答 API**: 現状 handler は `HeadersEnd` 時点で応答を返す設計のため、`[s→n]` 方向のリクエストボディ送信検証 (E2/E3/E5/E6 相当) が成立しない。`StreamEnd` 起点で応答できる handler API を追加すれば `[s→n]` 方向のボディ送信検証が可能になる。
13. **1 MiB 以上のボディ**: `tokio_s2n_quic::ServerConfig` / `ClientConfig` に QUIC フロー制御値 (`initial_max_data` 等) を上書きする公開 API が無い。1 MiB 以上を扱うには (a) `tokio_s2n_quic` の Config API 拡張、または (b) `s2n_quic::Server::builder().with_limits(...)` を組み込む別 issue が前提。

上記 1〜13 のうち、canary 卒業前に最低限カバーすべき要件 (1, 2, 3, 4, 5, 7, 12) は本 issue マージ後に別 issue として起票する。残りは canary 期間中の優先度を別途検討する。

## 解決方法

コミット f5b5260 で実装した。nghttp3 (ngtcp2) との e2e 相互運用テストを拡充し、メソッド・パス・ステータス・ヘッダー多様性・ボディサイズ・複数リクエスト等のケースを追加した。
