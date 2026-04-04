# init_h3_streams() のドキュメントにクリティカルストリームの寿命管理を明記する

Created: 2026-03-29
Completed: 2026-03-29
Model: Opus 4.6

## 概要

`Connection::init_h3_streams()` と `H3InitData` のドキュメントに、QUIC ストリームハンドルの寿命管理に関する警告を追記する。

## 根拠

moqt-rust-private の publisher / subscriber / relay の全てで、制御ストリームと QPACK ストリームをドロップさせないために以下のハックを書いている:

```rust
tokio::spawn(async move {
    let _control_send = control_send;
    let _encoder_send = encoder_send;
    let _decoder_send = decoder_send;
    std::future::pending::<()>().await;
});
```

コメントには「クローズすると H3_CLOSED_CRITICAL_STREAM エラーになる」と記載されている。

RFC 9114 Section 6.2.1:
> If either control stream is closed at any point, this MUST be treated as a connection error of type H3_CLOSED_CRITICAL_STREAM.

この要件は Sans I/O ライブラリの責務外だが、全ての利用者が踏むハマりポイント。`init_h3_streams()` のドキュメントで明確に警告すべき。

## 対応方針

- `H3InitData` の doc comment に以下を追記する:
  - 「初期データを送信した後も、対応する QUIC ストリームのハンドルは接続終了まで保持し続けること」
  - 「ストリームをクローズすると相手側で H3_CLOSED_CRITICAL_STREAM エラーが発生する (RFC 9114 Section 6.2.1)」
- `Connection::init_h3_streams()` の doc comment にも同様の警告を追記する

## 解決方法

`H3InitData` と `Connection::init_h3_streams()` の doc comment に以下の警告を追記した:

- 初期データを送信した後も QUIC ストリームのハンドルは接続終了まで保持し続けること
- クリティカルストリームをクローズすると相手側で H3_CLOSED_CRITICAL_STREAM エラーが発生する (RFC 9114 Section 6.2.1)
