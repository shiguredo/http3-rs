// ngtcp2 コア API (RFC 9000 QUIC 実装)
// 接続管理、ストリーム操作、パケット送受信など QUIC の主要機能を提供する
#include <ngtcp2/ngtcp2.h>

// QUIC-TLS 統合 API (RFC 9001)
// QUIC と TLS 1.3 を統合するための共通インターフェース
// 鍵導出、暗号レベル管理など TLS バックエンド非依存の処理を担う
#include <ngtcp2/ngtcp2_crypto.h>

// BoringSSL (aws-lc) バックエンド固有の QUIC-TLS 実装
// ngtcp2_crypto.h の抽象インターフェースを BoringSSL で実装する
// このプロジェクトでは aws-lc-sys を TLS ライブラリとして使用するため必須
#include <ngtcp2/ngtcp2_crypto_boringssl.h>
