# ngtcp2-sys

ngtcp2 の Rust バインディング (FFI) を提供するクレートです。

## 概要

[ngtcp2](https://github.com/ngtcp2/ngtcp2) は C で実装された QUIC ライブラリです。
このクレートは ngtcp2 をビルドし、Rust から利用するための低レベル FFI バインディングを提供します。

## 特徴

- ngtcp2 のソースコードを自動的にクローンしてビルド
- cmake によるクロスプラットフォームビルド
- aws-lc を TLS バックエンドとして使用
- 生成済みバインディングを同梱 (libclang 不要)

## ビルド要件

- CMake
- C コンパイラ (gcc, clang など)
- Go (aws-lc のビルドに必要)

## バインディングの再生成

ngtcp2 のバージョンを更新した際にバインディングを再生成する場合:

```bash
cargo build -p ngtcp2-sys --features overwrite
```

注意: `overwrite` feature を使用する場合は libclang が必要です。

## ngtcp2 ライセンス

<https://github.com/ngtcp2/ngtcp2/blob/main/COPYING>

```text
The MIT License

Copyright (c) 2019 nghttp3 contributors

Permission is hereby granted, free of charge, to any person obtaining
a copy of this software and associated documentation files (the
"Software"), to deal in the Software without restriction, including
without limitation the rights to use, copy, modify, merge, publish,
distribute, sublicense, and/or sell copies of the Software, and to
permit persons to whom the Software is furnished to do so, subject to
the following conditions:

The above copyright notice and this permission notice shall be
included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE
LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION
OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION
WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
```

## ライセンス

Apache License 2.0

```text
Copyright 2026-2026, Shiguredo Inc.

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
```
