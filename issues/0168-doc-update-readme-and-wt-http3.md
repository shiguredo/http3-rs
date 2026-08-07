# README と docs/WT_HTTP3.md を現状の実装に合わせて更新する

- Created: 2026-08-08
- Completed: {YYYY-MM-DD}
- Branch: feature/doc-update-readme-and-wt-http3
- Polished: {YYYY-MM-DD}

## 目的

draft-16 追従後のコードと README / docs の記述の乖離を解消する。

## 現状

README.md に以下の不整合がある:

- draft バージョン表記が draft-15 のまま (draft-16 追従済み)。該当箇所: WebTransport サポート表記、draft 毎の挙動差分テーブル (draft-14 / draft-16 の列がない)、ドラフトバージョンネゴシエーション表記、規格書一覧
- サンプルコード 4 箇所が現在の API でコンパイル不可: `Header::new` の `Result` 化、`Encoder::encode` のシグネチャ (`&mut self` + 第 3 引数)、`StreamHeader::new` の `Result` 化
- エラーコード一覧に `H3_VERSION_FALLBACK (0x110)` が記載されているが、0x110 は RFC 9114 Section 8.1 で reserved であり実装も存在しない
- エラーコード一覧に `H3_DATAGRAM_ERROR (0x33)` が欠落している

docs/WT_HTTP3.md:

- draft 02 / 07 / 15 の差分表のみで、実装済みの draft-14 / draft-16 が未記載

その他:

- `src/settings.rs` のモジュール doc の「draft-ietf-webtrans-http3-02 / -07 / -14 / -15」に -16 がない
- `src/webtransport/` 配下のモジュール doc の draft-15 表記が混在 (draft-15 → draft-16 で節番号は一致するため内容の誤りはないが、表記の統一が必要)
- `src/error.rs` の `ErrorCode::MissingSettings` の doc「リクエスト不完全 (0x10a)」が誤り (0x10a は MissingSettings、リクエスト不完全は 0x10d)

## 設計方針

- README を draft-16 表記に更新し、サンプルコードを現在の API に合わせて修正する
- エラーコード一覧を実装に合わせて修正する
- docs/WT_HTTP3.md に draft-14 / draft-16 を追記する
- ソースコード内のドラフト表記・誤った doc も修正する

## 完了条件

- README のサンプルコードがコンパイルできる
- README / docs/WT_HTTP3.md のドラフト表記とエラーコード一覧が実装と一致する
- `cargo test --doc` が通る (README の doctest を含む場合)

## 解決方法

### 関連ファイル

- `README.md`
- `docs/WT_HTTP3.md`
- `src/settings.rs` / `src/error.rs` / `src/webtransport/` 配下の doc コメント
