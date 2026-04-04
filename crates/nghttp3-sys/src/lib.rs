//! nghttp3 FFI バインディング
//!
//! このクレートは nghttp3 C ライブラリへの低レベル FFI バインディングを提供する。

#![allow(
    non_snake_case,
    non_upper_case_globals,
    non_camel_case_types,
    dead_code,
    clippy::all
)]

include!("bindings.rs");

// nghttp3_vec に Send/Sync を実装
// SAFETY: nghttp3_vec は writev_stream 呼び出し中にのみ使用され、
// ポインタの有効期間はその呼び出し内で完結する。
// async 関数内で使用する場合も、データをコピーしてからポインタを破棄するため、
// スレッド間で安全に転送可能。
unsafe impl Send for nghttp3_vec {}
unsafe impl Sync for nghttp3_vec {}
