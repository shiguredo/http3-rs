# 0093: リクエストの Host ヘッダー検証を :authority と同等に強化する

- Priority: Medium
- Created: 2026-05-30
- Model: Opus 4.8
- Branch: feature/add-host-header-validation
- Polished: 2026-05-30

## 目的

`validate_request_headers` の Host ヘッダー検証が `:authority` より弱く、2 つのギャップがある。

1. 重複 Host を検出しない。`:authority` は重複を `H3_MESSAGE_ERROR` で拒否する (`src/validation.rs:396-398`) が、Host は最後の値で無条件に上書きするだけ (`src/validation.rs:440-441`)。
2. Host 単独経路 (`:authority` 不在で Host のみ) で Host 値の構文を検証しない。`:authority` には `is_valid_authority` が適用される (`src/validation.rs:602-607`) が、Host には適用されない。

Host は `:authority` の代替として authority 情報を運ぶフィールドであり、検証レベルを `:authority` と対等にする。

## 優先度根拠

Medium。RFC 9114 / RFC 9110 が Host 構文不正の拒否を MUST として要求しているわけではない (後述) が、重複 Host の未検出は整合チェックの迂回を許す設計の穴であり、ルーティング誤認につながりうる。

- 重複 Host の危険性: `:authority` と Host の一致チェック (`src/validation.rs:570-573`) は最後の Host としか比較しない。`Host: attacker.example` + `Host: origin.example` のように 2 つ送ると、最後の値だけが `:authority` と照合され、先頭の値は素通りする。RFC 9110 Section 7.2 (`refs/rfc9110.txt`) は Host を単一の authority 情報として定義しており、複数 Host は曖昧でキャッシュポイズニング / リクエストスマグリングの標的になりうる。
- Host 構文不正は malformed の MUST ではない: RFC 9114 Section 4.1.2 / Section 4.3.1 (`refs/h3/rfc9114.txt`) の malformed の定義は pseudo-header に限定される ("contains invalid values for those pseudo-header fields is malformed")。Host は通常フィールドのため含まれない。RFC 9110 Section 7.2 も ABNF `Host = uri-host [ ":" port ]` を与えるのみで受信側の拒否 MUST はない。

よって RFC の MUST ではなく堅牢性・整合性の向上だが、迂回の穴を塞ぐ価値があるため Medium とする。

## 現状

- ヘッダーループで Host は `src/validation.rs:440-441` で無条件に `host = Some(header.value())` され、重複検出をしていない。`:authority` の重複検出 (`src/validation.rs:396-398`) と非対称。
- Host は `is_valid_field_value` (`src/validation.rs:436`)、非空チェック (`src/validation.rs:583-587`)、`:authority` との一致チェック (`src/validation.rs:570-573`) のみを通る。
- `is_valid_authority` (`src/validation.rs:271-311`、`host[:port]` 文法 / IPv6 リテラル対応) は `:authority` にのみ適用される (`src/validation.rs:602-607`)。`:authority` 不在で Host のみのリクエストでは、`Host: example.com:notaport` のような不正値が `is_valid_field_value` を通れば受理されうる。

## 設計方針

- 重複 Host を `:authority` と同じく `H3_MESSAGE_ERROR` で拒否する。これにより一致チェック迂回の穴も同時に塞がる。既存の重複 pseudo-header 拒否と一貫した設計とする。
- Host 単独経路の構文検証に既存の `is_valid_authority` を流用し、新規の検証関数は追加しない。エラー種別は `:authority` と同じ `H3_MESSAGE_ERROR`。
- `is_valid_authority` の doc コメント (`src/validation.rs:265` は「非 CONNECT リクエストの :authority が…」と記載) を Host 流用後の用途へ更新する。
- 構文検証の適用ガードは `authority.is_none() && host.is_some()`。plain CONNECT は `:authority` 必須 (`src/validation.rs:517-518` が不在を拒否) のためこのガードに入らず対象外。非 CONNECT と Extended CONNECT (`:protocol` 有) の Host 単独経路はいずれもガードに入り検証対象となる。`:authority` への既存ガード `(method != b"CONNECT" || protocol.is_some())` (`src/validation.rs:602`) と意味的に整合する。

## 完了条件

- Host ヘッダーが 2 つ以上あるリクエストを `H3_MESSAGE_ERROR` で拒否する。
- `:authority` 不在で Host 単独のリクエスト (非 CONNECT / Extended CONNECT の両経路) で、Host が `uri-host[:port]` から外れる場合に `H3_MESSAGE_ERROR` を返す。
- 既存の正常系 (`:authority` と単一 Host が一致、単一 Host のみ) の挙動は維持する。
- 追加したテストが修正前は失敗し、修正後に `cargo test --workspace --tests` で通過する。

## 解決方法

1. ヘッダーループの Host 取得 (`src/validation.rs:440-441`) を、`:authority` の重複検出 (`src/validation.rs:396-398`) と同じパターンに変更する。`host.is_some()` なら `H3_MESSAGE_ERROR` を返し、そうでなければ `host = Some(header.value())` とする。
2. Host 非空チェック (`src/validation.rs:583-587`) の直後に、`authority.is_none() && host.is_some()` のとき `is_valid_authority(h)` を適用し、不正なら `H3_MESSAGE_ERROR` を返す。`:authority` と Host の両方が存在する場合は一致チェック (`src/validation.rs:570-573`) と `:authority` の検証 (`src/validation.rs:602-607`) で既に担保されるため、Host 単独経路のみ追加すればよい (重複検証を避ける)。
   - `is_valid_authority` は内部の `is_valid_reg_name` (`src/validation.rs:213`) 経由で `@` (userinfo) を常に拒否する。`:authority` の userinfo 拒否は http/https 限定で別途 `src/validation.rs:591-597` にあるが、Host では scheme に依らず常に拒否される。Host に userinfo は不要なため妥当。
3. 追加するコードコメントに、根拠として RFC 9110 Section 7.2 (`Host = uri-host [ ":" port ]`) と、uri-host の構文実体である RFC 3986 Section 3.2.2 (host = IP-literal / IPv4address / reg-name) を記載する。あわせて構文検証の `H3_MESSAGE_ERROR` は RFC の MUST ではなく堅牢性向上である旨を明記する。
4. テストを追加する:
   - `tests/test_validation.rs`: Host を 2 つ持つリクエストを `H3_MESSAGE_ERROR` で拒否すること (`:authority` 有無の両方で確認)。
   - `tests/test_validation.rs`: `:authority` 不在で不正な Host (非数字ポート `example.com:notaport`、末尾コロン `example.com:`、ホスト名に不正文字) を `H3_MESSAGE_ERROR` で拒否すること。非 CONNECT と Extended CONNECT の両経路を含める。
   - `tests/test_validation.rs`: `:authority` 不在で正当な Host (`example.com` / `example.com:8443` / `[::1]:443`) を受理すること。両経路を含める。
   - `pbt/tests/prop_validation.rs`: 現状 `is_valid_authority` を対象とする構文検証 PBT は存在しない (既存 PBT は `:authority` に固定値を使うのみ)。authority 値を変化させる strategy を新規追加し、「`is_valid_authority(v)` が false ⇔ Host 単独リクエストが Err」「同じ `v` で `:authority` 経路も同一判定」の同値性を検証する。

## 関連

- 0092 の項目 3 から分離した。当初は Host 単独経路の構文検証のみを想定していたが、重複 Host 未検出による整合チェック迂回の穴も同根のため本 issue に含める。
