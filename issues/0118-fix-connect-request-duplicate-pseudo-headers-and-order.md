# `ConnectRequest::from_headers` の重複疑似ヘッダーと順序を検証する

- Priority: Medium
- Created: 2026-06-15
- Completed:
- Model: Opus 4.7
- Branch: feature/fix-connect-request-duplicate-pseudo-headers-and-order
- Polished:

## 目的

`src/webtransport/connect.rs:435-525` の `ConnectRequest::from_headers` は `:method`, `:protocol`, `:scheme`, `:authority`, `:path` の重複を検出せず、後勝ちで上書きする。また通常ヘッダーの後に疑似ヘッダーが現れても許容する。RFC 9114 Section 4.3.1 違反 (疑似ヘッダーは通常ヘッダーより前に並ぶ MUST、重複 MUST NOT)。重複検出と順序検証を追加する。

## 優先度根拠

Medium。実際の HTTP/3 デプロイで悪意あるクライアントが疑似ヘッダー重複を仕込むと、後勝ち解釈で `:method` / `:authority` が攻撃者の意図通り上書きされる可能性がある。

## 現状

`src/webtransport/connect.rs:444-485` 抜粋:

```rust
for header in headers {
    let name = header.name();
    let value = header.value();
    match name {
        b":method" => { method = Some(...); }
        b":protocol" => { protocol = Some(...); }
        // ... 後勝ち上書き
    }
}
```

RFC 9114 Section 4.3.1: 疑似ヘッダーは通常ヘッダーより前 (`MUST`)、重複 (`MUST NOT`)。

## 設計方針

- ループ内で「すでに `Some` の場合はエラー」検査を追加
- 「通常ヘッダーが出現した後の疑似ヘッダー」を検出するフラグ (`seen_regular_header: bool`) を導入
- 検出時は `ConnectError::InvalidHeaders` (or `H3_MESSAGE_ERROR` 経由のエラー) を返す
- PBT で「重複疑似ヘッダー / 順序違反入力で必ずエラーが返る」プロパティを検証

## 完了条件

- 重複 `:method` / `:protocol` / `:scheme` / `:authority` / `:path` でエラーが返る
- 通常ヘッダー後の疑似ヘッダー出現でエラーが返る
- PBT が追加される
- `make fmt && make clippy && make check` が通る

## 解決方法

```rust
let mut seen_regular_header = false;
for header in headers {
    let name = header.name();
    if name.starts_with(b":") {
        if seen_regular_header {
            return Err(ConnectError::PseudoHeaderAfterRegular);
        }
        match name {
            b":method" if method.is_some() => return Err(ConnectError::DuplicatePseudoHeader),
            // ...
        }
    } else {
        seen_regular_header = true;
    }
}
```

### 関連ファイル

- 修正対象: `src/webtransport/connect.rs:435-525`
- PBT 追加: `pbt/tests/prop_webtransport/connect.rs`
- 一次資料: `refs/h3/rfc9114.txt` Section 4.3.1
