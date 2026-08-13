//! エラー分類のテスト
//!
//! `Error::classify_connection_error` は ngtcp2 の API 契約
//! (`ngtcp2_conn_read_pkt` / `ngtcp2_conn_handle_expiry` のドキュメント) に従い、
//! サーバー実装がエラーを接続単位で処理するための分類を提供する。
//! 各エラーコードが期待どおりの種別に分類されることを検証する。

use shiguredo_ngtcp2::{ConnectionErrorKind, Error};

/// NGTCP2_ERR_DROP_CONN は SilentDrop に分類される
#[test]
fn test_classify_drop_conn() {
    let err = Error::from_ngtcp2(ngtcp2_sys::NGTCP2_ERR_DROP_CONN);
    assert_eq!(
        err.classify_connection_error(),
        ConnectionErrorKind::SilentDrop
    );
}

/// NGTCP2_ERR_IDLE_CLOSE は SilentDrop に分類される
#[test]
fn test_classify_idle_close() {
    let err = Error::from_ngtcp2(ngtcp2_sys::NGTCP2_ERR_IDLE_CLOSE);
    assert_eq!(
        err.classify_connection_error(),
        ConnectionErrorKind::SilentDrop
    );
}

/// NGTCP2_ERR_RETRY は SilentDrop に分類される
///
/// Retry パケット送信は未実装のため、接続を黙って破棄する設計。
#[test]
fn test_classify_retry() {
    let err = Error::from_ngtcp2(ngtcp2_sys::NGTCP2_ERR_RETRY);
    assert_eq!(
        err.classify_connection_error(),
        ConnectionErrorKind::SilentDrop
    );
}

/// NGTCP2_ERR_DRAINING は Terminal に分類される
#[test]
fn test_classify_draining() {
    let err = Error::from_ngtcp2(ngtcp2_sys::NGTCP2_ERR_DRAINING);
    assert_eq!(
        err.classify_connection_error(),
        ConnectionErrorKind::Terminal
    );
}

/// NGTCP2_ERR_CLOSING は Terminal に分類される
#[test]
fn test_classify_closing() {
    let err = Error::from_ngtcp2(ngtcp2_sys::NGTCP2_ERR_CLOSING);
    assert_eq!(
        err.classify_connection_error(),
        ConnectionErrorKind::Terminal
    );
}

/// NGTCP2_ERR_DISCARD_PKT は Ignore に分類される
#[test]
fn test_classify_discard_pkt() {
    let err = Error::from_ngtcp2(ngtcp2_sys::NGTCP2_ERR_DISCARD_PKT);
    assert_eq!(err.classify_connection_error(), ConnectionErrorKind::Ignore);
}

/// NGTCP2_ERR_CRYPTO は TransportClose に分類される
#[test]
fn test_classify_crypto() {
    let err = Error::from_ngtcp2(ngtcp2_sys::NGTCP2_ERR_CRYPTO);
    assert_eq!(
        err.classify_connection_error(),
        ConnectionErrorKind::TransportClose
    );
}

/// 分類対象外の ngtcp2 エラーコードは TransportClose に分類される
#[test]
fn test_classify_other_ngtcp2_error() {
    let err = Error::from_ngtcp2(ngtcp2_sys::NGTCP2_ERR_NOBUF);
    assert_eq!(
        err.classify_connection_error(),
        ConnectionErrorKind::TransportClose
    );
}

/// nghttp3 エラーは ApplicationClose に分類される
#[test]
fn test_classify_nghttp3_error() {
    let err = Error::from_nghttp3(nghttp3_sys::NGHTTP3_ERR_INVALID_STATE);
    assert_eq!(
        err.classify_connection_error(),
        ConnectionErrorKind::ApplicationClose
    );
}

/// StreamDataBlocked はストリーム単位のシグナルとして Ignore に分類される
#[test]
fn test_classify_stream_data_blocked() {
    let err = Error::StreamDataBlocked(0);
    assert_eq!(err.classify_connection_error(), ConnectionErrorKind::Ignore);
}

/// StreamShutWr はストリーム単位のシグナルとして Ignore に分類される
#[test]
fn test_classify_stream_shut_wr() {
    let err = Error::StreamShutWr(0);
    assert_eq!(err.classify_connection_error(), ConnectionErrorKind::Ignore);
}

/// Internal は Internal に分類される
#[test]
fn test_classify_internal() {
    let err = Error::Internal("test".to_string());
    assert_eq!(
        err.classify_connection_error(),
        ConnectionErrorKind::Internal
    );
}

/// InvalidArgument は Internal に分類される
#[test]
fn test_classify_invalid_argument() {
    let err = Error::InvalidArgument("test".to_string());
    assert_eq!(
        err.classify_connection_error(),
        ConnectionErrorKind::Internal
    );
}

/// BufferTooSmall は Internal に分類される
#[test]
fn test_classify_buffer_too_small() {
    assert_eq!(
        Error::BufferTooSmall.classify_connection_error(),
        ConnectionErrorKind::Internal
    );
}

/// StreamNotFound は Internal に分類される
#[test]
fn test_classify_stream_not_found() {
    let err = Error::StreamNotFound(0);
    assert_eq!(
        err.classify_connection_error(),
        ConnectionErrorKind::Internal
    );
}

/// ConnectionClosing は Internal に分類される
#[test]
fn test_classify_connection_closing() {
    assert_eq!(
        Error::ConnectionClosing.classify_connection_error(),
        ConnectionErrorKind::Internal
    );
}
