# 0085: qpack::Header を構築時検査型に変更する

Created: 2026-05-23
Model: Opus 4.7

## 概要

`qpack::Header` を「不正な値を持てない型」に作り直す。現状の
`Header::new(name, value)` は無検査でフィールドを構築でき、`pub name: Vec<u8>` /
`pub value: Vec<u8>` のため構造体リテラルや直接代入でも不正値を注入できる。

破壊的変更を伴う。`qpack::Header` の公開 API を `Result<Self, HeaderError>` 化し、
フィールドを private 化してアクセサを提供する。加えてリテラル定数向けの
`const fn` 構築 API (`Header::from_static`) を提供して **RFC 違反をコンパイル時に
検出可能** にする。

decoder 側の `qpack::DecodedHeader` は本 issue で **`Header` に統合して削除** する。
`validation::HeaderField` トレイトも統合に伴い削除する。

## 設計方針: 二段の構築 API

利用者の入力源によって 2 つの構築点を提供する。

| API | 対象 | 検査タイミング | 失敗時の挙動 |
|---|---|---|---|
| `Header::new(name, value)` | ランタイム値 (`impl AsRef<[u8]>`) | 実行時 | `Err(HeaderError)` |
| `Header::from_static(name, value)` | `&'static [u8]` リテラル | コンパイル時 (`const fn`) | コンパイルエラー (const eval panic) |

`from_static` を `const fn` で実装することで、以下のような定数定義はリテラルが
RFC 違反なら **CI を回す前にコンパイルエラー** で検出される。

```rust
// OK: コンパイル成功
const METHOD: Header = Header::from_static(b":method", b"GET");

// NG: コンパイル時に "field-name must be lowercase" で fail
const BAD: Header = Header::from_static(b"Host", b"example.com");

// NG: コンパイル時に "field-value contains CR" で fail
const INJECT: Header = Header::from_static(b":path", b"/foo\r\nX-Inject: 1");
```

## 背景

現状のコードでは以下の問題がある:

- `Header::new(b":path", b"/foo\r\nX-Inject: 1")` のような CRLF を含む値を構築可能
- 大文字を含む field-name (`Header::new(b"Host", b"...")`) を構築可能
- `pub` フィールドのため `header.name = vec![b'H', b'o', b's', b't']` のように
  構築後に不正値を代入可能
- `qpack::DecodedHeader` も同様にフィールドが `pub` で、decoder が wire 上の
  バイト列をそのまま渡す経路がある
- field-name / field-value の構文検査は `src/validation.rs` の
  `validate_request_headers` / `validate_response_headers` で事後検査するが、
  構築点で検出できない → HTTP Response Splitting (CWE-113) のリスク
- HTTP/3 は QPACK 圧縮で値の出所が追跡しにくく、構築時検査の価値は HTTP/1.1 以上に大きい

## 根拠

- RFC 9114 §4.2: "HTTP/3 follows the requirements established in Section 5 of
  [HTTP] regarding ... field names" → RFC 9110 §5.1 の field-name 制約を継承
- RFC 9114 §4.2: "characters in field names MUST be converted to lowercase
  prior to their encoding" (MUST)
- RFC 9114 §4.2: "A field value MUST NOT contain the zero value (ASCII NUL,
  0x00), line feed (ASCII LF, 0x0a), or carriage return (ASCII CR, 0x0d) at
  any position"
- RFC 9114 §4.2: "A field value MUST NOT start or end with an ASCII whitespace
  character (ASCII SP or HTAB, 0x20 or 0x09)"
- RFC 9114 §4.3: 疑似ヘッダー名は `:` で始まる定義済みの集合のみ許可
  (`:method`, `:scheme`, `:authority`, `:path`, `:status`)
- RFC 8441 §4: `:protocol` 疑似ヘッダー (RFC 9220 で HTTP/3 への適用が定義)
- RFC 9220: HTTP/3 における Extended CONNECT (`:protocol` の利用)
- RFC 9110 §5.1, §5.6.2: field-name = token = 1*tchar
- RFC 9110 §5.5: field-value の field-content 文法 (field-vchar = 0x21-0x7E | 0x80-0xFF、
  先頭末尾は field-vchar、途中 SP/HTAB のみ許可)

## スコープ

`shiguredo_http3` ルートクレート内の `qpack::Header` 構築 API すべてを対象とする。
`qpack::DecodedHeader` を廃止し `Header` に統合する。

### DecodedHeader と Header の統合

- `qpack::DecodedHeader` を削除する
- `Decoder::decode` / `DynamicDecoder::decode` の戻り値型を
  `Vec<DecodedHeader>` / `DecodeOutput::Decoded(Vec<DecodedHeader>)` から
  `Vec<Header>` / `DecodeOutput::Decoded(Vec<Header>)` に変更する
- `validation::HeaderField` トレイトを削除し、`validate_*` 系の関数は
  `&[Header]` を直接受け取るシグネチャに変更する

### フィールドの private 化

`Header` の全フィールド (`name`, `value`) を private にし、以下のアクセサを提供する:

```rust
impl Header {
    pub fn name(&self) -> &[u8];
    pub fn value(&self) -> &[u8];
    pub fn size(&self) -> usize;  // RFC 9114 §4.2.2 field section size 用
}
```

これにより構造体リテラルでの直接構築や、構築後のフィールド書き換えを防止する。

### API 変更

```rust
// 変更前
pub struct Header {
    pub name: Vec<u8>,
    pub value: Vec<u8>,
}

impl Header {
    pub fn new(name: impl Into<Vec<u8>>, value: impl Into<Vec<u8>>) -> Self;
}

// 変更後
pub struct Header {
    name: Cow<'static, [u8]>,
    value: Cow<'static, [u8]>,
}

impl Header {
    /// ランタイム値から検査つきで構築する
    pub fn new(name: impl AsRef<[u8]>, value: impl AsRef<[u8]>)
        -> Result<Self, HeaderError>;

    /// 静的バイト列から検査つきで構築する (const fn)
    ///
    /// 不正なリテラルを渡すとコンパイル時に panic (= コンパイルエラー) になる。
    pub const fn from_static(name: &'static [u8], value: &'static [u8]) -> Self;

    /// 検証済みバイト列から検査をスキップして構築する (crate 内部専用)
    pub(crate) fn from_validated_parts(
        name: Cow<'static, [u8]>,
        value: Cow<'static, [u8]>,
    ) -> Self;

    pub fn name(&self) -> &[u8];
    pub fn value(&self) -> &[u8];
    pub fn size(&self) -> usize;
}
```

### `const fn` の実装上の注意

- `from_static` 内部では `Vec<u8>` を作れないため、`Header` の内部表現を
  `Cow<'static, [u8]>` に変更する。issue 0059 (Bytes 化) と統合検討する。
  `from_static` 経路では引数の `&'static [u8]` を `Cow::Borrowed` でラップする
- `const fn` 内で `Err` を返す機構 (`const Try`) は MSRV 1.88 では未安定。
  代わりに `assert!(condition, "message")` で fail させる。const eval が panic を
  含むコードを評価するとコンパイルエラーになるため、利用者から見れば
  「不正リテラル = コンパイル不能」になる
- **`from_static` の const 文脈制約**: `assert!` によるコンパイルエラー検出は
  `const` / `static` 宣言内でのみ機能する。`let h = Header::from_static(...)` の
  ような実行時文脈ではランタイム panic になる。利用者には `from_static` を
  `const` 宣言で使うことをドキュメントで推奨する
- **`from_static` と `new` の戻り値非対称**: `from_static` は const fn 制約により
  `Result` を返せず `-> Self` (失敗時 panic)。`new` は `-> Result<Self, HeaderError>`。
  この差は const Rust の制約に由来するものであり、ランタイム値の検査には
  `new` (構造化エラーあり) を、コンパイル時定数には `from_static` (コンパイルエラー、
  ただしエラーメッセージは定型文字列) を使い分ける
- `const fn` 内の検査ロジックは `while` ループで bytes を走査する (`for` は const 不可)

### QPACK 静的テーブルへの活用

`src/qpack/table.rs` の `STATIC_TABLE` (RFC 9204 Appendix A の 99 エントリ) を
`from_static` で組み立てることで、テーブル定義自体のリテラルが RFC 違反なら
コンパイル時に検出できる:

```rust
// 変更前
pub static STATIC_TABLE: &[StaticEntry] = &[
    StaticEntry { name: b":authority", value: b"" },
    ...
];

// 変更後 (StaticEntry を Header に統合する場合)
pub static STATIC_TABLE: &[Header] = &[
    Header::from_static(b":authority", b""),
    ...
];
```

`StaticEntry` を `Header` に統合するかは設計判断として本 issue で扱う。
統合すれば `find_static_entry` 等の検索 API も `Header` ベースに統一できる。

### 新規エラー型

```rust
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeaderError {
    /// field-name が空
    /// (RFC 9110 §5.1, §5.6.2: token = 1*tchar)
    EmptyFieldName,

    /// field-name に lowercase 以外の ASCII 英字が含まれる
    /// (RFC 9114 §4.2: MUST be lowercase)
    UppercaseFieldName { name: Vec<u8> },

    /// field-name に token 文字以外が含まれる
    /// (RFC 9110 §5.1, §5.6.2 / RFC 9114 §4.2)
    InvalidFieldNameByte { name: Vec<u8>, byte: u8 },

    /// field-value に NUL/CR/LF が含まれる
    /// (RFC 9114 §4.2 MUST NOT)
    InvalidFieldValueByte { name: Vec<u8>, byte: u8 },

    /// field-value が先頭または末尾に SP/HTAB を含む
    /// (RFC 9114 §4.2 MUST NOT)
    FieldValueLeadingOrTrailingWhitespace { name: Vec<u8> },

    /// 疑似ヘッダー名が未定義 (`:foo` のような不明な疑似ヘッダー)
    /// (RFC 9114 §4.3, RFC 8441 §4 / RFC 9220)
    UnknownPseudoHeader { name: Vec<u8> },

    /// 疑似ヘッダー値が構文違反
    /// 各疑似ヘッダーの構文根拠:
    /// - `:method`: RFC 9110 §9.1 (token)
    /// - `:scheme`: RFC 3986 §3.1 (scheme)
    /// - `:path`: RFC 9114 §4.3.1, RFC 9110 §4.1 (absolute-path)
    /// - `:status`: RFC 9110 §15 (3DIGIT)
    /// - `:protocol`: RFC 8441 §4, RFC 9220 (HTTP Upgrade Token)
    /// - `:authority`: RFC 3986 §3.2, RFC 9114 §4.3.1 (authority, userinfo 拒否)
    InvalidPseudoHeaderValue { name: Vec<u8>, value: Vec<u8> },
}

impl core::fmt::Display for HeaderError { ... }
impl std::error::Error for HeaderError {}
```

### 検査内容

`Header::new` で実施する検査:

1. field-name が空でないこと (RFC 9110 §5.1, §5.6.2: `token = 1*tchar`)
2. field-name が lowercase ASCII + token 文字のみであること (RFC 9114 §4.2)
3. field-value の全バイトが field-vchar (0x21-0x7E | 0x80-0xFF) または SP (0x20) /
   HTAB (0x09) であり、先頭と末尾が field-vchar であること (RFC 9110 §5.5 field-content)
   — これにより NUL/CR/LF 混入 (RFC 9114 §4.2) と前後空白 (RFC 9114 §4.2) を
   包含して検出する
4. 疑似ヘッダー (`:` で始まる) の場合は、名前が `:method` / `:scheme` /
   `:authority` / `:path` / `:status` / `:protocol` のいずれかであることを確認
   (RFC 9114 §4.3, RFC 8441 §4, RFC 9220)
5. 疑似ヘッダーの値構文 — **単一フィールドで完結する検査のみ**:
   - `:method`: token (RFC 9110 §9.1, §5.6.2)
   - `:scheme`: scheme 構文 (RFC 3986 §3.1)
   - `:status`: 3DIGIT (RFC 9110 §15)

以下の検査は「ヘッダーリスト全体」または「他のフィールドとの組み合わせ」に依存するため、
`src/validation.rs` 側に残す:

- リクエスト/レスポンスの整合性 (例: CONNECT に `:path` が無い等)
- `:path` の値検査: absolute-path (`/` で始まる) または `*` (asterisk-form) の判定は
  `:scheme` の値 (http/https か否か) と `:method` の値 (OPTIONS か否か) に依存するため
- `:authority` の値検査: authority 構文 (RFC 3986 §3.2) / CONNECT authority-form /
  userinfo 拒否の判定は `:method` と `:scheme` の値に依存するため
- `:protocol` の値検査: HTTP Upgrade Token (RFC 8441 §4) の構文検証は
  新規に `validation.rs` に実装する (現在未実装のため)
- 疑似ヘッダーの順序・重複・存在チェック
- 接続固有フィールド (`connection`, `keep-alive`, `te` 等) の拒否 (RFC 9114 §4.2)

## 影響範囲

- `src/qpack/encoder.rs`: `Header` 構造体の private 化、`from_static` 追加、
  `new` の `Result` 化、フィールドアクセスをアクセサ経由に変更。
  `Header::new` のシグネチャを `impl AsRef<[u8]>` に変更し、内部で検査を実施する
- `src/qpack/decoder.rs`: `DecodedHeader` を削除し、`Decoder::decode` /
  `DynamicDecoder::decode` の戻り値を `Vec<Header>` / `DecodeOutput::Decoded(Vec<Header>)`
  に変更。decoder 内部の `from_validated_parts` 経由で `Header` を構築 (QPACK デコード後の
  値は RFC 9204 §4.1.1 のプレフィックス整数上限制約により field-content に適合している。
  具体的な検証は decoder の自己責任とし、`from_validated_parts` で検査をスキップする)
- `src/qpack/table.rs`: `StaticEntry` を `Header` に統合検討。
  `STATIC_TABLE` を `Header::from_static` で組み立てる。
  `get_static_entry` の戻り値型は `Option<&'static Header>` に変更 (破壊的変更)。
  `find_static_entry` の引数は `&[u8]` のまま維持
- `src/qpack/dynamic_table.rs`: `insert(name, value)` 内部で
  `Header::from_validated_parts` 経由に変更。
  `DynamicEntry` のフィールドも `Header` に合わせて private 化を検討
  (公開 API のため、本 issue で `DynamicEntry` も private 化するかは別途判断)
- `src/qpack/mod.rs`: `HeaderError` の re-export 追加、`DecodedHeader` の
  re-export 削除、`StaticEntry` 削除 (Header に統合した場合)
- `src/validation.rs`: `HeaderField` トレイト削除。各 `validate_*` 関数を
  `&[Header]` 直受けに変更。field-name / field-value の構築時検査は
  `Header::new` に移し、リスト整合性と疑似ヘッダー組み合わせ検査のみを残す。
  **テストで不正ヘッダーを構築する必要があるため、テストケースでは
  `pub(crate) from_validated_parts` を使用して RFC 違反データを迂回構築する**
- `src/connection/mod.rs`: `DecodedHeader` を使用している
  `is_webtransport_connect_decoded` / `is_plain_connect` / `is_informational_status` /
  `is_success_status` / `is_no_body_status` / `emit_header_events` を
  `&[Header]` 受けに変更。テストのヘルパーも `Header::new(...).unwrap()` に移行
- `src/stream/request.rs`: `recv_headers: Vec<DecodedHeader>` を
  `Vec<Header>` に変更。`ReceivedData::Headers(Vec<DecodedHeader>)` を
  `ReceivedData::Headers(Vec<Header>)` に変更
- `src/event.rs`: `Event::Header { name: Vec<u8>, value: Vec<u8> }` は
  `DecodedHeader` を直接使っていないが、`DecodedHeader` から変換している箇所
  (connection 側) でアクセサ経由に変更する必要がある
- `src/lib.rs`: `HeaderError` の `pub use` 追加。`DecodedHeader` / `HeaderField` /
  `StaticEntry` の `pub use` 削除 (統合した場合)
- `connection/client.rs`: `Header::new` → `Header::new(...)?` に変更
- `examples/`: `Header::new` の `?` 化、フィールドアクセスをアクセサ経由に変更
- `tests/`: 全 `Header::new` 呼び出しの `Result` 化 (`?` または `.unwrap()` 追加)
- `pbt/tests/`: `Header` strategy を新 API に追従、不正値生成から正規値生成に変更
- `fuzz/fuzz_targets/`: 新 API に追従
- `interop/`: 必要に応じて新 API に追従

## CHANGES.md エントリ

```
- [ADD] `qpack::Header::from_static` を追加し、リテラル定数の RFC 違反を
  コンパイル時に検出可能にする
  - @担当者
- [CHANGE] `qpack::Header::new` を構築時検査つきの `Result<Self, HeaderError>` 化する
  - @担当者
- [CHANGE] `qpack::Header` のフィールドを private 化し、アクセサメソッドを提供する
  - @担当者
- [CHANGE] `qpack::DecodedHeader` を削除し、decoder の戻り値型を `Header` に統一する
  - @担当者
- [CHANGE] `validation::HeaderField` トレイトを削除し、`validate_*` 関数を
  `&[Header]` 直受けに変更する
  - @担当者
```

## 受け入れ条件

- `Header::new` が `Result<Self, HeaderError>` を返す
- `Header::from_static` が `const fn` で実装され、`const` / `static` 宣言内で
  不正リテラルを渡すとコンパイルエラーになる (実行時文脈では panic。
  compile_fail テストは issue 0088 で実施)
- `Header` の全フィールドが private で、アクセサ経由でのみ読み取れる
- `Header` の内部表現が `Cow<'static, [u8]>` (または同等の static 対応表現) になっている
- decoder 経路は `from_validated_parts` 経由で構築している
- `DecodedHeader` が削除されている
- `Decoder::decode` / `DynamicDecoder::decode` の戻り値が `Header` ベースになっている
- `validation::HeaderField` トレイトが削除されている
- `validate_*` 関数が `&[Header]` を直接受け取る
- `HeaderError` が `Display` + `std::error::Error` を実装している
- `src/validation.rs` の個別フィールド値検査が `Header::new` に統合され、
  リスト整合性検査のみが残っている
- 既存の全テスト・PBT・fuzz が通る

## 依存

- なし (本 issue は独立した構築時検査型導入。ただし 0059 の Bytes 化により
  `Cow<'static, [u8]>` → `Bytes` への再変更が発生する可能性あり)

## 関連

- [[0059-refactor-introduce-bytes-crate]] (`from_static` の内部表現変更と統合検討)
- [[0086-change-settings-construct-time-validation]] (Settings の構築時検査)
- [[0087-change-frame-construct-time-validation]] (Frame ペイロードの構築時検査)
- [[0088-add-trybuild-and-pbt-construct-time-validation]] (`from_static` の compile_fail テスト)
